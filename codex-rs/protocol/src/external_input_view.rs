//! Bounded inert display facts. This module never constructs model content.
use crate::external_input::Error;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "ExternalInputView", export_to = "v2/")]
#[schemars(rename = "ExternalInputView")]
pub struct View {
    pub schema: String,
    #[ts(type = "Record<string, unknown>")]
    pub data: Value,
}

impl View {
    /// Stable JSON for digest framing, independent of incoming object key order.
    pub fn canonical_json(&self) -> String {
        fn sorted(value: &Value) -> Value {
            match value {
                Value::Object(map) => {
                    let ordered: std::collections::BTreeMap<_, _> = map
                        .iter()
                        .map(|(key, value)| (key.clone(), sorted(value)))
                        .collect();
                    Value::Object(ordered.into_iter().collect())
                }
                Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
                other => other.clone(),
            }
        }
        // Explicit wrapper order is part of the v2 digest contract.
        format!(
            "{{\"data\":{},\"schema\":{}}}",
            sorted(&self.data),
            Value::String(self.schema.clone())
        )
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.schema.is_empty()
            || self.schema.len() > 128
            || !self
                .schema
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || !self.data.is_object()
        {
            return Err(Error::Invalid("view schema or root"));
        }
        fn check(value: &Value, depth: usize, entries: &mut usize) -> Result<(), Error> {
            let children: Vec<&Value> = match value {
                Value::Object(map) => map.values().collect(),
                Value::Array(items) => items.iter().collect(),
                Value::Number(number) if !number.is_i64() && !number.is_u64() => {
                    return Err(Error::Invalid("view integer"));
                }
                _ => return Ok(()),
            };
            if depth > 8 {
                return Err(Error::Invalid("view depth"));
            }
            *entries += children.len();
            if *entries > 1024 {
                return Err(Error::Invalid("view entries"));
            }
            for child in children {
                check(child, depth + 1, entries)?;
            }
            Ok(())
        }
        check(&self.data, 1, &mut 0)?;
        if self.canonical_json().len() > 16384 {
            return Err(Error::Invalid("view bytes"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "external_input_view_tests.rs"]
mod tests;
