use super::*;
use crate::FreeformTool;
use crate::FreeformToolFormat;
use crate::ResponsesApiNamespace;
use crate::parse_tool_input_schema;
use pretty_assertions::assert_eq;
use serde_json::json;

fn function(name: &str, input_schema: JsonValue) -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: name.to_string(),
        description: String::new(),
        strict: false,
        defer_loading: None,
        parameters: parse_tool_input_schema(&input_schema).expect("schema parses"),
        output_schema: None,
    })
}

fn parameters(spec: &ToolSpec) -> JsonValue {
    match spec {
        ToolSpec::Function(tool) => serde_json::to_value(&tool.parameters).unwrap(),
        other => panic!("not a function: {other:?}"),
    }
}

fn strict(spec: ToolSpec) -> ToolSpec {
    let tools = [spec];
    apply_tool_schema_compat(&tools, ToolSchemaCompat::Strict)
        .into_owned()
        .remove(0)
}

/// MCP-style input schemas that Google function declarations refuse, each with
/// the strict lowering of its one property.
fn violating_fixtures() -> Vec<(&'static str, JsonValue, JsonValue)> {
    vec![
        (
            "nullable enum (zod .nullable())",
            json!({"type": ["string", "null"], "enum": ["low", "high", null]}),
            json!({"type": "string", "enum": ["low", "high"]}),
        ),
        (
            "typeless integer enum (sanitizer infers string)",
            json!({"enum": [1, 2, 3]}),
            json!({"type": "integer", "enum": [1, 2, 3]}),
        ),
        (
            "const boolean (sanitizer: typeless enum -> string)",
            json!({"const": true}),
            json!({"type": "boolean", "enum": [true]}),
        ),
        (
            "mixed value types",
            json!({"enum": ["auto", 1, true, "1"]}),
            json!({"type": "string", "enum": ["auto", "1", "true"]}),
        ),
        (
            "declared type mismatch",
            json!({"type": "string", "enum": [0.5, 1]}),
            json!({"type": "number", "enum": [0.5, 1]}),
        ),
        (
            "type union with a homogeneous enum",
            json!({"type": ["integer", "string"], "enum": [1, 2]}),
            json!({"type": "integer", "enum": [1, 2]}),
        ),
        (
            "object values",
            json!({"type": "object", "enum": [{"a": 1}], "properties": {}}),
            json!({"type": "string", "enum": ["{\"a\":1}"], "properties": {}}),
        ),
        (
            "only null",
            json!({"type": ["string", "null"], "enum": [null]}),
            json!({"type": "string"}),
        ),
    ]
}

#[test]
fn off_borrows_the_specs_unchanged() {
    let tools = vec![function(
        "t",
        json!({"type": "object", "properties": {"m": {"type": ["string", "null"], "enum": ["a", null]}}}),
    )];
    let applied = apply_tool_schema_compat(&tools, ToolSchemaCompat::Off);
    assert!(matches!(applied, Cow::Borrowed(_)));
    assert_eq!(
        serde_json::to_string(applied.as_ref()).unwrap(),
        serde_json::to_string(&tools).unwrap()
    );
}

#[test]
fn strict_lowers_each_violating_enum_and_the_validator_agrees() {
    for (label, property, expected) in violating_fixtures() {
        let spec = function(
            "t",
            json!({"type": "object", "properties": {"p": property}}),
        );
        assert!(
            !tool_spec_strict_violations(&spec).is_empty(),
            "{label}: the validator should flag {}",
            parameters(&spec)
        );
        let lowered = strict(spec);
        assert_eq!(parameters(&lowered)["properties"]["p"], expected, "{label}");
        assert_eq!(
            tool_spec_strict_violations(&lowered),
            Vec::<String>::new(),
            "{label}"
        );
    }
}

#[test]
fn strict_keeps_valid_enums_byte_identical() {
    let input = json!({
        "type": "object",
        "properties": {
            "s": {"type": "string", "enum": ["a", "b"], "description": "kept"},
            "n": {"type": "number", "enum": [1, 2.5]},
            "i": {"type": "number", "enum": [1, 2]},
            "b": {"type": "boolean", "enum": [false]},
            "free": {"type": ["string", "null"]},
            "role": {"anyOf": [{"type": "string", "enum": ["user", "tool"]}, {"type": "null"}]}
        },
        "required": ["s"],
        "additionalProperties": false
    });
    let spec = function("t", input);
    assert_eq!(tool_spec_strict_violations(&spec), Vec::<String>::new());
    let before = serde_json::to_string(&spec).unwrap();
    assert_eq!(serde_json::to_string(&strict(spec)).unwrap(), before);
}

#[test]
fn strict_recurses_through_nested_schemas() {
    let bad = json!({"type": ["string", "null"], "enum": ["x", null]});
    let good = json!({"type": "string", "enum": ["x"]});
    let spec = function(
        "nested",
        json!({
            "type": "object",
            "properties": {
                "list": {"type": "array", "items": bad},
                "map": {"type": "object", "properties": {}, "additionalProperties": bad},
                "any": {"anyOf": [bad, {"type": "integer", "enum": [1, "1"]}]},
                "one": {"oneOf": [bad]},
                "all": {"allOf": [bad]},
                "ref": {"$ref": "#/$defs/Mode"},
                "legacy": {"$ref": "#/definitions/Mode"}
            },
            "$defs": {"Mode": bad},
            "definitions": {"Mode": bad}
        }),
    );
    assert_eq!(
        tool_spec_strict_violations(&spec).len(),
        8,
        "{}",
        parameters(&spec)
    );
    let lowered = strict(spec);
    assert_eq!(tool_spec_strict_violations(&lowered), Vec::<String>::new());
    let value = parameters(&lowered);
    assert_eq!(value["properties"]["list"]["items"], good);
    assert_eq!(value["properties"]["map"]["additionalProperties"], good);
    assert_eq!(value["properties"]["any"]["anyOf"][0], good);
    assert_eq!(
        value["properties"]["any"]["anyOf"][1],
        json!({"type": "string", "enum": ["1"]})
    );
    assert_eq!(value["properties"]["one"]["oneOf"][0], good);
    assert_eq!(value["properties"]["all"]["allOf"][0], good);
    assert_eq!(value["$defs"]["Mode"], good);
    assert_eq!(value["definitions"]["Mode"], good);
}

#[test]
fn strict_covers_namespaces_and_tool_search_and_skips_other_specs() {
    let bad_schema = || {
        parse_tool_input_schema(&json!({
            "type": "object",
            "properties": {"m": {"type": ["string", "null"], "enum": ["a", null]}}
        }))
        .unwrap()
    };
    let namespace_tool = ResponsesApiTool {
        name: "inner".to_string(),
        description: String::new(),
        strict: false,
        defer_loading: Some(true),
        parameters: bad_schema(),
        output_schema: None,
    };
    let freeform = FreeformTool {
        name: "apply_patch".to_string(),
        description: String::new(),
        defer_loading: None,
        format: FreeformToolFormat {
            r#type: "grammar".to_string(),
            syntax: "lark".to_string(),
            definition: "start: \"x\"".to_string(),
        },
    };
    let tools = vec![
        ToolSpec::Namespace(ResponsesApiNamespace {
            name: "mcp__srv".to_string(),
            description: String::new(),
            tools: vec![
                ResponsesApiNamespaceTool::Function(namespace_tool),
                ResponsesApiNamespaceTool::Custom(freeform.clone()),
            ],
        }),
        ToolSpec::ToolSearch {
            execution: "client".to_string(),
            description: String::new(),
            parameters: bad_schema(),
        },
        ToolSpec::Freeform(freeform),
        ToolSpec::WebSearch {
            external_web_access: Some(false),
            indexed_web_access: None,
            filters: None,
            user_location: None,
            search_context_size: None,
            search_content_types: None,
        },
    ];
    let before: Vec<String> = tools.iter().flat_map(tool_spec_strict_violations).collect();
    assert_eq!(before.len(), 2, "{before:?}");
    assert!(
        before[0].starts_with("mcp__srv.inner/properties/m: "),
        "{before:?}"
    );
    assert!(
        before[1].starts_with("tool_search/properties/m: "),
        "{before:?}"
    );

    let lowered = apply_tool_schema_compat(&tools, ToolSchemaCompat::Strict);
    let after: Vec<String> = lowered
        .iter()
        .flat_map(tool_spec_strict_violations)
        .collect();
    assert_eq!(after, Vec::<String>::new());
    // Freeform and web search specs are untouched.
    assert_eq!(lowered[2], tools[2]);
    assert_eq!(lowered[3], tools[3]);
}

#[test]
fn validator_reports_const_and_typeless_enums_in_raw_json() {
    let raw = json!({
        "type": "object",
        "properties": {
            "c": {"const": "x"},
            "ok": {"type": "string", "const": "x"},
            "e": {"enum": ["a"]},
            "n": {"type": "integer", "enum": [1, null]}
        }
    });
    let violations = strict_schema_violations(&raw, "t");
    assert_eq!(violations.len(), 3, "{violations:?}");
    assert!(violations[0].starts_with("t/properties/c: enum without type"));
    assert!(violations[1].starts_with("t/properties/e: enum without type"));
    assert!(violations[2].starts_with("t/properties/n: null or non-primitive enum value"));
}

#[test]
fn strict_lowers_serialized_tools_from_history_items() {
    let bad = function(
        "loaded",
        json!({"type": "object", "properties": {"m": {"type": ["string", "null"], "enum": ["a", null]}}}),
    );
    let good = function(
        "fine",
        json!({"type": "object", "properties": {"m": {"type": "string", "enum": ["a"]}, "n": {"type": "integer", "minimum": 1}}}),
    );
    let mut namespace = json!({
        "type": "namespace",
        "name": "mcp__srv",
        "description": "",
        "tools": [serde_json::to_value(&bad).unwrap(), serde_json::to_value(&good).unwrap()]
    });
    // Unknown keywords in a schema that needs no change stay byte-identical.
    namespace["tools"][1]["parameters"]["properties"]["n"]["x-extra"] = json!(true);
    let untouched = namespace["tools"][1].clone();
    normalize_tool_json_strict(&mut namespace);
    assert_eq!(
        namespace["tools"][0]["parameters"]["properties"]["m"],
        json!({"type": "string", "enum": ["a"]})
    );
    assert_eq!(namespace["tools"][1], untouched);

    let mut search = json!({"type": "tool_search", "execution": "client", "description": "",
        "parameters": {"type": "object", "properties": {"k": {"type": "string", "enum": [1, "a"]}}}});
    normalize_tool_json_strict(&mut search);
    assert_eq!(
        search["parameters"]["properties"]["k"],
        json!({"type": "string", "enum": ["1", "a"]})
    );
    // Other shapes are ignored.
    let mut other = json!({"type": "custom", "name": "apply_patch"});
    let before = other.clone();
    normalize_tool_json_strict(&mut other);
    assert_eq!(other, before);
}

#[test]
fn strict_normalizes_boxed_enums_without_changing_the_source_specs() {
    let original_schema = JsonSchema {
        schema_type: Some(JsonSchemaType::Multiple(Box::new([
            JsonSchemaPrimitiveType::String,
            JsonSchemaPrimitiveType::Null,
        ]))),
        enum_values: Some(Box::new([
            json!("auto"),
            json!(1),
            json!(true),
            json!("1"),
            json!(null),
        ])),
        ..JsonSchema::default()
    };
    let tools = vec![ToolSpec::Function(ResponsesApiTool {
        name: "boxed_enum".to_string(),
        description: String::new(),
        strict: false,
        defer_loading: None,
        parameters: original_schema.clone(),
        output_schema: None,
    })];
    let before = serde_json::to_string(&tools).unwrap();
    let lowered = apply_tool_schema_compat(&tools, ToolSchemaCompat::Strict);
    assert!(matches!(&lowered, Cow::Owned(_)));
    assert_eq!(
        parameters(&lowered[0]),
        json!({"type": "string", "enum": ["auto", "1", "true"]})
    );
    assert_eq!(serde_json::to_string(&tools).unwrap(), before);
    let ToolSpec::Function(original) = &tools[0] else {
        panic!("function expected");
    };
    assert_eq!(original.parameters, original_schema);
}

#[test]
fn strict_removes_null_only_boxed_enum_and_preserves_remaining_union_types() {
    use JsonSchemaPrimitiveType::Integer;
    use JsonSchemaPrimitiveType::Null;
    use JsonSchemaPrimitiveType::String;

    for (types, expected_type) in [
        (vec![Null], None),
        (vec![String, Null], Some(JsonSchemaType::Single(String))),
        (
            vec![String, Null, Integer],
            Some(JsonSchemaType::Multiple(Box::new([String, Integer]))),
        ),
    ] {
        let mut schema = JsonSchema {
            schema_type: Some(JsonSchemaType::Multiple(types.into_boxed_slice())),
            enum_values: Some(Box::new([json!(null)])),
            ..JsonSchema::default()
        };
        normalize_json_schema_strict(&mut schema);
        assert_eq!(schema.enum_values, None);
        assert_eq!(schema.schema_type, expected_type);
        let normalized = schema.clone();
        normalize_json_schema_strict(&mut schema);
        assert_eq!(schema, normalized, "normalization must be idempotent");
    }
}
