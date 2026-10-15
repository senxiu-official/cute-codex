//! Durable display data. These values are never model messages or tool results.
use crate::external_input::Source;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum PresentationFormat {
    PlainText,
    Markdown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub enum PresentationReferenceKind {
    ExternalInput,
    McpInvocation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct PresentationReference {
    pub kind: PresentationReferenceKind,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct Presentation {
    pub id: String,
    pub source: Source,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub format: PresentationFormat,
    pub references: Vec<PresentationReference>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct PresentationAppended {
    pub version: u32,
    pub owner_id: String,
    pub origin_thread_id: String,
    pub presentation: Presentation,
    pub semantic_sha256: String,
    pub receipt_id: String,
}

fn hash(domain: &[u8], fields: &[&str]) -> String {
    let mut value = Sha256::new();
    value.update(domain);
    for field in fields {
        value.update((field.len() as u64).to_be_bytes());
        value.update(field.as_bytes());
    }
    format!("{:x}", value.finalize())
}

impl PresentationAppended {
    /// Generation is deliberately absent: it authorizes a request, not a fact.
    pub fn semantic_digest(&self) -> String {
        let mut fields = vec![
            self.owner_id.as_str(),
            self.origin_thread_id.as_str(),
            self.presentation.id.as_str(),
            match self.presentation.source.kind {
                crate::external_input::SourceKind::Agent => "agent",
                crate::external_input::SourceKind::Service => "service",
            },
            self.presentation.source.id.as_str(),
            self.presentation.title.as_str(),
            self.presentation.body.as_str(),
            match self.presentation.format {
                PresentationFormat::PlainText => "plainText",
                PresentationFormat::Markdown => "markdown",
            },
        ];
        for reference in &self.presentation.references {
            fields.push(match reference.kind {
                PresentationReferenceKind::ExternalInput => "externalInput",
                PresentationReferenceKind::McpInvocation => "mcpInvocation",
            });
            fields.push(&reference.id);
        }
        hash(b"codex:presentation:semantic:v1\0", &fields)
    }

    pub fn receipt_digest(&self) -> String {
        hash(
            b"codex:presentation:receipt:v1\0",
            &[
                &self.owner_id,
                &self.origin_thread_id,
                &self.presentation.id,
                &self.semantic_sha256,
            ],
        )
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 {
            return Err("unsupported presentation version");
        }
        for id in [
            &self.owner_id,
            &self.origin_thread_id,
            &self.presentation.id,
            &self.presentation.source.id,
        ] {
            if id.is_empty() || id.len() > 256 {
                return Err("presentation identity byte limit");
            }
        }
        if self.presentation.title.len() > 256 || self.presentation.body.len() > 65536 {
            return Err("presentation text byte limit");
        }
        if self.presentation.title.is_empty() && self.presentation.body.is_empty() {
            return Err("presentation requires title or body");
        }
        if self.presentation.references.len() > 2 {
            return Err("presentation reference limit");
        }
        for reference in &self.presentation.references {
            if reference.id.is_empty() || reference.id.len() > 256 {
                return Err("presentation reference identity byte limit");
            }
        }
        if self.semantic_sha256 != self.semantic_digest()
            || self.receipt_id != self.receipt_digest()
        {
            return Err("presentation digest or receipt mismatch");
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
