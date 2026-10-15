use super::*;
use codex_protocol::external_input::Delivery;
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input::Message;
use codex_protocol::external_input::Source;
use codex_protocol::external_input::SourceKind;
use codex_protocol::external_input_record::Record;
use pretty_assertions::assert_eq;
fn pair() -> [RolloutItem; 2] {
    let mut e = Envelope {
        view: None,
        version: 1,
        owner_id: "owner".into(),
        thread_id: "thread".into(),
        runtime_generation: 1,
        message: Message {
            id: "jsc_".to_owned() + &"a".repeat(64),
            source: Source {
                kind: SourceKind::Service,
                id: "build".into(),
            },
            event_type: "notice".into(),
            delivery: Delivery::Passive,
            text: "Body 世界".into(),
        },
        semantic_sha256: String::new(),
    };
    e.semantic_sha256 = e.digest();
    let item = e.response_item().unwrap();
    let receipt = Receipt::new(&e, "original-turn".into(), 1).unwrap();
    [
        RolloutItem::ExternalInput(Record {
            version: 1,
            owner_id: e.owner_id.clone(),
            thread_id: e.thread_id.clone(),
            fact: Fact::Commit {
                commit: Box::new(Commit {
                    envelope: e,
                    receipt,
                }),
            },
        }),
        RolloutItem::ResponseItem(item.into()),
    ]
}
#[test]
fn complete_pair_preserves_original_identity_and_body() {
    let pair = pair();
    let mut p = ExternalInputProjection::default();
    assert_eq!(p.observe(&pair[0]).unwrap(), None);
    assert!(p.is_pending());
    let item = p.observe(&pair[1]).unwrap().unwrap();
    assert!(!p.is_pending());
    assert_eq!(item.turn_id, "original-turn");
    let ThreadItem::FunctionCallOutput {
        id,
        name,
        namespace,
        output,
        external_input_view,
    } = item.item
    else {
        panic!("wrong item")
    };
    assert_eq!(id, "jsc_".to_owned() + &"a".repeat(64));
    assert_eq!(name, "external_event");
    assert_eq!(external_input_view, None);
    assert_eq!(namespace, Some("external".into()));
    assert!(output.to_text().unwrap().contains("Body 世界"));
}
#[test]
fn no_pair_no_trusted_display_and_mismatch_errors() {
    let pair = pair();
    assert_eq!(
        ExternalInputProjection::default()
            .observe(&pair[1])
            .unwrap(),
        None
    );
    let mut p = ExternalInputProjection::default();
    p.observe(&pair[0]).unwrap();
    assert_eq!(p.observe(&pair[0]), Err(Error::Corrupt));
    let mut wrong = pair[1].clone();
    if let RolloutItem::ResponseItem(item) = &mut wrong
        && let codex_protocol::models::ResponseItem::FunctionCallOutput { call_id, .. } =
            &mut item.item
    {
        *call_id = Some("fake-call".into());
    }
    let mut p = ExternalInputProjection::default();
    p.observe(&pair[0]).unwrap();
    assert_eq!(p.observe(&wrong), Err(Error::Corrupt));
}

#[test]
fn full_rebuild_buffers_original_turn_and_rejects_partial_pair() {
    let pair = pair();
    let mut builder = crate::ThreadHistoryBuilder::new();
    builder.handle_rollout_item(&pair[0]);
    assert!(builder.finish_checked().is_err());
    let mut builder = crate::ThreadHistoryBuilder::new();
    for item in &pair {
        builder.handle_rollout_item(item);
    }
    builder.handle_rollout_item(&RolloutItem::EventMsg(
        codex_protocol::protocol::EventMsg::TurnStarted(
            codex_protocol::protocol::TurnStartedEvent {
                turn_id: "original-turn".into(),
                trace_id: None,
                root_turn_id: None,
                started_at: Some(1),
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            },
        ),
    ));
    let turns = builder.finish_checked().unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].id, "original-turn");
    assert_eq!(turns[0].items.len(), 1);
}

#[test]
fn exact_replay_matches_but_conflicting_identity_is_rejected() {
    let pair = pair();
    let mut p = ExternalInputProjection::default();
    p.observe(&pair[0]).unwrap();
    let first = p.observe(&pair[1]).unwrap();
    p.observe(&pair[0]).unwrap();
    assert_eq!(first, p.observe(&pair[1]).unwrap());
    let mut conflict = pair[0].clone();
    let RolloutItem::ExternalInput(record) = &mut conflict else {
        panic!()
    };
    let Fact::Commit { commit } = &mut record.fact else {
        panic!()
    };
    commit.envelope.message.text = "Different body".into();
    commit.envelope.semantic_sha256 = commit.envelope.digest();
    commit.receipt = Receipt::new(&commit.envelope, "original-turn".into(), 1).unwrap();
    let output = RolloutItem::ResponseItem(commit.envelope.response_item().unwrap().into());
    p.observe(&conflict).unwrap();
    assert_eq!(p.observe(&output), Err(Error::Conflict));
}

#[test]
fn structured_view_complete_pair_projects_separately() {
    let mut pair = pair();
    let RolloutItem::ExternalInput(record) = &mut pair[0] else {
        panic!("commit")
    };
    let codex_protocol::external_input_record::Fact::Commit { commit } = &mut record.fact else {
        panic!("commit")
    };
    commit.envelope.version = 2;
    commit.envelope.view = Some(codex_protocol::external_input_view::View {
        schema: "test.v1".into(),
        data: serde_json::json!({"sentinel":"NON_MODEL_VIEW"}),
    });
    commit.envelope.semantic_sha256 = commit.envelope.digest();
    commit.receipt = codex_protocol::external_input::Receipt::new(
        &commit.envelope,
        commit.receipt.turn_id.clone(),
        commit.receipt.ordinal,
    )
    .unwrap();
    let mut projection = ExternalInputProjection::default();
    assert!(projection.observe(&pair[0]).unwrap().is_none());
    let item = projection.observe(&pair[1]).unwrap().unwrap().item;
    let value = serde_json::to_value(item).unwrap();
    assert_eq!(
        value["externalInputView"]["data"]["sentinel"],
        "NON_MODEL_VIEW"
    );
    assert!(!value["output"].to_string().contains("NON_MODEL_VIEW"));
}
