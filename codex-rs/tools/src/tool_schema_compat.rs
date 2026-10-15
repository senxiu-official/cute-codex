//! Optional compatibility lowering for tool parameter schemas (config
//! `tool_schema_compat`), applied to a copy of the model-visible tool specs
//! right before they are serialized into a request.
//!
//! Some providers reached through OpenAI-compatible gateways convert function
//! declarations into a stricter schema dialect that refuses an `enum` unless
//! all of its values share one primitive type that matches the schema `type`
//! (for example Google function declarations: "Enum values must share one
//! supported primitive type and match the schema type"). MCP servers commonly
//! emit enums that break that rule (`z.enum([...]).nullable()` becomes
//! `{"type": ["string", "null"], "enum": ["a", "b", null]}`, `{"enum": [1, 2]}`
//! without a type is lowered to `type: "string"` by the MCP schema sanitizer,
//! `const: true` becomes a typeless single-value enum, ...).
//!
//! [`ToolSchemaCompat::Strict`] rewrites every schema that carries an `enum`:
//! - `null` is removed from the enum (and from a `type` union);
//! - values that all share one primitive type set `type` to that type
//!   (integers under a declared `number` stay `number`);
//! - mixed, object, array or otherwise mismatched values become their string
//!   form (strings stay as they are, others become their JSON text, duplicates
//!   dropped) with `type: "string"`;
//! - an enum that only held `null` is removed.
//!
//! The walk recurses through `properties`, `items`, `additionalProperties`,
//! `anyOf`, `oneOf`, `allOf`, `$defs` and `definitions`. `const` never reaches
//! this point: MCP schemas lower it to a single-value enum on parse and the
//! typed [`JsonSchema`] has no `const`.
//!
//! [`ToolSchemaCompat::Off`] borrows the specs unchanged, so requests stay
//! byte-identical.

use crate::AdditionalProperties;
use crate::JsonSchema;
use crate::JsonSchemaPrimitiveType;
use crate::JsonSchemaType;
use crate::ResponsesApiNamespaceTool;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use codex_protocol::config_types::ToolSchemaCompat;
use serde_json::Value as JsonValue;
use std::borrow::Cow;

/// Returns the tool specs to serialize under `compat`: borrowed unchanged for
/// [`ToolSchemaCompat::Off`], a normalized copy for
/// [`ToolSchemaCompat::Strict`].
pub fn apply_tool_schema_compat(
    tools: &[ToolSpec],
    compat: ToolSchemaCompat,
) -> Cow<'_, [ToolSpec]> {
    match compat {
        ToolSchemaCompat::Off => Cow::Borrowed(tools),
        ToolSchemaCompat::Strict => {
            let mut tools = tools.to_vec();
            for tool in &mut tools {
                normalize_tool_spec_strict(tool);
            }
            Cow::Owned(tools)
        }
    }
}

/// Applies the strict lowering to every parameter schema of one tool spec.
pub fn normalize_tool_spec_strict(spec: &mut ToolSpec) {
    match spec {
        ToolSpec::Function(tool) => normalize_function_tool(tool),
        ToolSpec::Namespace(namespace) => {
            for tool in &mut namespace.tools {
                match tool {
                    ResponsesApiNamespaceTool::Function(tool) => normalize_function_tool(tool),
                    ResponsesApiNamespaceTool::Custom(_) => {}
                }
            }
        }
        ToolSpec::ToolSearch { parameters, .. } => normalize_json_schema_strict(parameters),
        ToolSpec::WebSearch { .. } | ToolSpec::Freeform(_) => {}
    }
}

fn normalize_function_tool(tool: &mut ResponsesApiTool) {
    normalize_json_schema_strict(&mut tool.parameters);
}

/// Strict lowering of one schema and all of its sub-schemas (see the module
/// docs for the rules).
pub fn normalize_json_schema_strict(schema: &mut JsonSchema) {
    normalize_enum(schema);
    if let Some(properties) = schema.properties.as_mut() {
        properties
            .values_mut()
            .for_each(normalize_json_schema_strict);
    }
    if let Some(items) = schema.items.as_mut() {
        normalize_json_schema_strict(items);
    }
    if let Some(AdditionalProperties::Schema(additional)) = schema.additional_properties.as_mut() {
        normalize_json_schema_strict(additional);
    }
    for variants in [
        schema.any_of.as_mut(),
        schema.one_of.as_mut(),
        schema.all_of.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        variants.iter_mut().for_each(normalize_json_schema_strict);
    }
    for table in [schema.defs.as_mut(), schema.definitions.as_mut()]
        .into_iter()
        .flatten()
    {
        table.values_mut().for_each(normalize_json_schema_strict);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    String,
    Integer,
    Number,
    Boolean,
    Null,
    Other,
}

fn value_kind(value: &JsonValue) -> ValueKind {
    match value {
        JsonValue::String(_) => ValueKind::String,
        JsonValue::Bool(_) => ValueKind::Boolean,
        JsonValue::Number(number) if number.is_i64() || number.is_u64() => ValueKind::Integer,
        JsonValue::Number(_) => ValueKind::Number,
        JsonValue::Null => ValueKind::Null,
        JsonValue::Array(_) | JsonValue::Object(_) => ValueKind::Other,
    }
}

/// The one primitive type every value shares (integers widen to `number` when
/// mixed with non-integers), or `None` for mixed / non-primitive values.
fn shared_primitive_type(values: &[JsonValue]) -> Option<JsonSchemaPrimitiveType> {
    let mut shared: Option<ValueKind> = None;
    for kind in values.iter().map(value_kind) {
        shared = match (shared, kind) {
            (_, ValueKind::Null | ValueKind::Other) => return None,
            (None, kind) => Some(kind),
            (Some(current), kind) if current == kind => Some(current),
            (Some(ValueKind::Integer), ValueKind::Number)
            | (Some(ValueKind::Number), ValueKind::Integer) => Some(ValueKind::Number),
            _ => return None,
        };
    }
    match shared? {
        ValueKind::String => Some(JsonSchemaPrimitiveType::String),
        ValueKind::Integer => Some(JsonSchemaPrimitiveType::Integer),
        ValueKind::Number => Some(JsonSchemaPrimitiveType::Number),
        ValueKind::Boolean => Some(JsonSchemaPrimitiveType::Boolean),
        ValueKind::Null | ValueKind::Other => None,
    }
}

fn normalize_enum(schema: &mut JsonSchema) {
    let Some(values) = schema.enum_values.as_mut() else {
        return;
    };
    values.retain(|value| !value.is_null());
    if values.is_empty() {
        // `enum: [null]` (or `[]`): nothing representable is left.
        schema.enum_values = None;
        drop_null_from_type_union(schema);
        return;
    }

    let declared_number = matches!(
        schema.schema_type,
        Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Number))
    );
    match shared_primitive_type(values) {
        Some(JsonSchemaPrimitiveType::Integer) if declared_number => {}
        Some(shared) => schema.schema_type = Some(JsonSchemaType::Single(shared)),
        None => {
            let mut strings: Vec<JsonValue> = Vec::with_capacity(values.len());
            for value in values.iter() {
                let text = match value {
                    JsonValue::String(text) => text.clone(),
                    other => other.to_string(),
                };
                let text = JsonValue::String(text);
                if !strings.contains(&text) {
                    strings.push(text);
                }
            }
            *values = strings;
            schema.schema_type = Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::String));
        }
    }
}

fn drop_null_from_type_union(schema: &mut JsonSchema) {
    if let Some(JsonSchemaType::Multiple(types)) = schema.schema_type.as_mut() {
        types.retain(|schema_type| *schema_type != JsonSchemaPrimitiveType::Null);
        match types.as_slice() {
            [] => schema.schema_type = None,
            [single] => schema.schema_type = Some(JsonSchemaType::Single(*single)),
            _ => {}
        }
    }
}

/// Strict lowering of an already serialized tool (`function`, `namespace`
/// or `tool_search` JSON, as carried by `tool_search_output` /
/// `additional_tools` history items). A parameter schema is rewritten only
/// when the lowering changes it; schemas the typed [`JsonSchema`] cannot read
/// are left as they are.
pub fn normalize_tool_json_strict(tool: &mut JsonValue) {
    let Some(map) = tool.as_object_mut() else {
        return;
    };
    match map.get("type").and_then(JsonValue::as_str) {
        Some("function" | "tool_search") => {
            if let Some(parameters) = map.get_mut("parameters") {
                normalize_schema_json_strict(parameters);
            }
        }
        Some("namespace") => {
            if let Some(tools) = map.get_mut("tools").and_then(JsonValue::as_array_mut) {
                tools.iter_mut().for_each(normalize_tool_json_strict);
            }
        }
        _ => {}
    }
}

fn normalize_schema_json_strict(parameters: &mut JsonValue) {
    let Ok(original) = serde_json::from_value::<JsonSchema>(parameters.clone()) else {
        return;
    };
    let mut lowered = original.clone();
    normalize_json_schema_strict(&mut lowered);
    if lowered != original
        && let Ok(value) = serde_json::to_value(&lowered)
    {
        *parameters = value;
    }
}

/// Lists strict-enum violations in a serialized JSON Schema: an `enum` /
/// `const` without one single `type`, with `null`, object or array values,
/// with values of more than one primitive type, or with values whose type
/// differs from `type` (integers satisfy `number`). Each entry is
/// `<path>: <problem>: <schema>`. Used to audit tool lists; see
/// [`tool_spec_strict_violations`].
pub fn strict_schema_violations(schema: &JsonValue, path: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_violations(schema, path, &mut out);
    out
}

/// [`strict_schema_violations`] for every parameter schema of a tool spec,
/// prefixed with the tool (and namespace) name.
pub fn tool_spec_strict_violations(spec: &ToolSpec) -> Vec<String> {
    let mut out = Vec::new();
    let mut function = |prefix: &str, tool: &ResponsesApiTool| {
        let parameters = serde_json::to_value(&tool.parameters).unwrap_or(JsonValue::Null);
        out.extend(strict_schema_violations(
            &parameters,
            &format!("{prefix}{}", tool.name),
        ));
    };
    match spec {
        ToolSpec::Function(tool) => function("", tool),
        ToolSpec::Namespace(namespace) => {
            let prefix = format!("{}.", namespace.name);
            for tool in &namespace.tools {
                if let ResponsesApiNamespaceTool::Function(tool) = tool {
                    function(&prefix, tool);
                }
            }
        }
        ToolSpec::ToolSearch { parameters, .. } => {
            let parameters = serde_json::to_value(parameters).unwrap_or(JsonValue::Null);
            out.extend(strict_schema_violations(&parameters, "tool_search"));
        }
        ToolSpec::WebSearch { .. } | ToolSpec::Freeform(_) => {}
    }
    out
}

fn collect_violations(schema: &JsonValue, path: &str, out: &mut Vec<String>) {
    let map = match schema {
        JsonValue::Object(map) => map,
        JsonValue::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect_violations(item, &format!("{path}[{index}]"), out);
            }
            return;
        }
        _ => return,
    };

    let mut values: Vec<JsonValue> = map
        .get("enum")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(value) = map.get("const") {
        values.push(value.clone());
    }
    if map.contains_key("enum") || map.contains_key("const") {
        let declared = map.get("type");
        let problem = if values
            .iter()
            .any(|value| matches!(value_kind(value), ValueKind::Null | ValueKind::Other))
        {
            Some("null or non-primitive enum value")
        } else if values.is_empty() {
            None
        } else if shared_primitive_type(&values).is_none() {
            Some("enum values of mixed types")
        } else if declared.is_none() {
            Some("enum without type")
        } else if !declared.is_some_and(JsonValue::is_string) {
            Some("enum with a type union")
        } else {
            let declared = declared.and_then(JsonValue::as_str).unwrap_or_default();
            let shared = shared_primitive_type(&values);
            let matches = match shared {
                Some(JsonSchemaPrimitiveType::String) => declared == "string",
                Some(JsonSchemaPrimitiveType::Integer) => {
                    declared == "integer" || declared == "number"
                }
                Some(JsonSchemaPrimitiveType::Number) => declared == "number",
                Some(JsonSchemaPrimitiveType::Boolean) => declared == "boolean",
                _ => false,
            };
            (!matches).then_some("enum values do not match the type")
        };
        if let Some(problem) = problem {
            out.push(format!("{path}: {problem}: {schema}"));
        }
    }

    for key in ["properties", "$defs", "definitions", "patternProperties"] {
        if let Some(children) = map.get(key).and_then(JsonValue::as_object) {
            for (name, child) in children {
                collect_violations(child, &format!("{path}/{key}/{name}"), out);
            }
        }
    }
    for key in [
        "items",
        "additionalProperties",
        "anyOf",
        "oneOf",
        "allOf",
        "prefixItems",
        "not",
    ] {
        if let Some(child) = map.get(key) {
            collect_violations(child, &format!("{path}/{key}"), out);
        }
    }
}

#[cfg(test)]
#[path = "tool_schema_compat_tests.rs"]
mod tests;
