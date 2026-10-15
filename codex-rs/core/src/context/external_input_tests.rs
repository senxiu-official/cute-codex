use super::*;
use codex_protocol::external_input::Delivery;
use codex_protocol::external_input::Message;
use codex_protocol::external_input::Source;
use codex_protocol::external_input::SourceKind;

fn envelope(text: String) -> Envelope {
    let mut envelope = Envelope {
        view: None,
        version: 1,
        owner_id: "owner".into(),
        thread_id: "thread".into(),
        runtime_generation: 7,
        message: Message {
            id: "message".into(),
            source: Source {
                kind: SourceKind::Service,
                id: "source".into(),
            },
            event_type: "opaque.v1".into(),
            delivery: Delivery::AfterTurn,
            text,
        },
        semantic_sha256: String::new(),
    };
    envelope.semantic_sha256 = envelope.digest();
    envelope
}

#[test]
fn canonical_item_bound_includes_escaping_and_envelope_overhead() {
    for unit in ["x", "世界", "\n", "\"", "\\", "\u{0000}"] {
        let mut last_good = None;
        for n in 1..=10_000 {
            let envelope = envelope(unit.repeat(n));
            let expected = envelope.response_item().unwrap();
            let bytes = serde_json::to_vec(&expected).unwrap().len();
            let result = ExternalInputContext::new(&envelope, CanonicalBytePolicy::default());
            if bytes > 10_000 {
                assert!(result.is_err());
                assert!(last_good.is_some());
                break;
            }
            let item = result.unwrap().into_response_item();
            assert_eq!(item, expected);
            assert!(matches!(
                item,
                ResponseItem::FunctionCallOutput { call_id: None, .. }
            ));
            last_good = Some(bytes);
        }
    }
}

#[test]
fn exact_receiver_limit_and_off_preserve_transport_and_identity() {
    for text in ["small".into(), "世界\n\"".repeat(2000)] {
        let value = envelope(text);
        let expected = value.response_item().unwrap();
        let bytes = serde_json::to_vec(&expected).unwrap().len();
        let exact = CanonicalBytePolicy::Limit(std::num::NonZeroU32::new(bytes as u32).unwrap());
        let lower =
            CanonicalBytePolicy::Limit(std::num::NonZeroU32::new(bytes as u32 - 1).unwrap());
        assert!(ExternalInputContext::new(&value, lower).is_err());
        for policy in [exact, CanonicalBytePolicy::Off] {
            assert_eq!(
                ExternalInputContext::new(&value, policy)
                    .unwrap()
                    .into_response_item(),
                expected
            );
            assert_eq!(value.digest(), value.semantic_sha256);
        }
    }
    let large_limit = CanonicalBytePolicy::Limit(std::num::NonZeroU32::new(u32::MAX).unwrap());
    for policy in [
        CanonicalBytePolicy::default(),
        large_limit,
        CanonicalBytePolicy::Off,
    ] {
        let oversized = envelope("x".repeat(65_537));
        assert!(ExternalInputContext::new(&oversized, policy).is_err());
    }
    for policy in [large_limit, CanonicalBytePolicy::Off] {
        let at_transport_limit = envelope("x".repeat(65_536));
        assert_eq!(
            ExternalInputContext::new(&at_transport_limit, policy)
                .unwrap()
                .into_response_item(),
            at_transport_limit.response_item().unwrap()
        );
    }
}
