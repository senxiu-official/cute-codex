use super::*;
use crate::external_input::Source;
use crate::external_input::SourceKind;
use pretty_assertions::assert_eq;

fn record() -> PresentationAppended {
    let mut value = PresentationAppended {
        version: 1,
        owner_id: "owner".into(),
        origin_thread_id: "thread".into(),
        presentation: Presentation {
            id: "p".into(),
            source: Source {
                kind: SourceKind::Service,
                id: "s".into(),
            },
            title: String::new(),
            body: "only display 世界".into(),
            format: PresentationFormat::PlainText,
            references: vec![],
        },
        semantic_sha256: String::new(),
        receipt_id: String::new(),
    };
    sign(&mut value);
    value
}
fn sign(value: &mut PresentationAppended) {
    value.semantic_sha256 = value.semantic_digest();
    value.receipt_id = value.receipt_digest();
}
#[test]
fn presentation_roundtrip_keeps_original_receipt_and_no_model_item() {
    let value = record();
    assert_eq!(value.validate(), Ok(()));
    assert_eq!(
        value.semantic_sha256,
        "4e7717dc740b4db1f097b77d23f199666a0ac0f2e3e0620f01c1322274b85e3d"
    );
    assert_eq!(
        value.receipt_id,
        "29e63d453d9e61e1b9fdb345d8697a86a18797c4c154d9bbbcc79e47bca4a8fb"
    );
    let event = crate::protocol::EventMsg::PresentationAppended(value.clone());
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "presentation_appended");
    assert!(
        serde_json::from_value::<crate::models::ResponseItem>(json.clone()).is_err()
            || matches!(
                serde_json::from_value::<crate::models::ResponseItem>(json.clone()).unwrap(),
                crate::models::ResponseItem::Other
            )
    );
    let decoded: crate::protocol::EventMsg = serde_json::from_value(json).unwrap();
    let crate::protocol::EventMsg::PresentationAppended(found) = decoded else {
        panic!("wrong event");
    };
    assert_eq!(found, value);
}
#[test]
fn presentation_byte_boundaries_and_optional_body() {
    let mut value = record();
    value.presentation.body = "界".repeat(21845) + "a";
    sign(&mut value);
    assert_eq!(value.validate(), Ok(()));
    value.presentation.body.push('b');
    sign(&mut value);
    assert_eq!(value.validate(), Err("presentation text byte limit"));
    value.presentation.body.clear();
    value.presentation.title = "x".repeat(256);
    sign(&mut value);
    assert_eq!(value.validate(), Ok(()));
    value.presentation.title.push('x');
    sign(&mut value);
    assert_eq!(value.validate(), Err("presentation text byte limit"));
    value.presentation.title.clear();
    sign(&mut value);
    assert_eq!(value.validate(), Err("presentation requires title or body"));
}
#[test]
fn presentation_identity_binds_fields_and_reference_order() {
    let original = record();
    let mut changed = original.clone();
    changed.presentation.body.push('!');
    assert_eq!(
        changed.validate(),
        Err("presentation digest or receipt mismatch")
    );
    sign(&mut changed);
    assert_ne!(changed.receipt_id, original.receipt_id);
    let mut changed = original.clone();
    changed.origin_thread_id.push('x');
    sign(&mut changed);
    assert_ne!(changed.receipt_id, original.receipt_id);
    let mut changed = original;
    changed.presentation.references = vec![
        PresentationReference {
            kind: PresentationReferenceKind::ExternalInput,
            id: "a".into(),
        },
        PresentationReference {
            kind: PresentationReferenceKind::McpInvocation,
            id: "b".into(),
        },
    ];
    sign(&mut changed);
    let digest = changed.semantic_sha256.clone();
    changed.presentation.references.reverse();
    sign(&mut changed);
    assert_ne!(digest, changed.semantic_sha256);
    changed
        .presentation
        .references
        .push(changed.presentation.references[0].clone());
    sign(&mut changed);
    assert_eq!(changed.validate(), Err("presentation reference limit"));
}
#[test]
fn presentation_rejects_privileged_source_and_version() {
    let value = record();
    let mut json = serde_json::to_value(&value).unwrap();
    json["presentation"]["source"]["kind"] = "system".into();
    assert!(serde_json::from_value::<PresentationAppended>(json).is_err());
    let mut changed = value;
    changed.version = 2;
    assert_eq!(changed.validate(), Err("unsupported presentation version"));
}
