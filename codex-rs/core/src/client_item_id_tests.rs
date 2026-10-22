use super::*;
use codex_protocol::ResponseItemId;
use codex_protocol::external_input as external;
use pretty_assertions::assert_eq;

const ORIGINAL_ID: &str = "jsc_68d6f7ad5a9339150fb4076d3e698194aad4900e4e822ffdc3745bce03a6443b";

#[test]
fn provider_item_id_projection_preserves_recovered_external_identity() -> anyhow::Result<()> {
    let client = test_model_client_with_provider(
        ModelProviderInfo::create_openai_provider(/*base_url*/ None),
        SessionSource::Cli,
    );
    let ids = [
        ORIGINAL_ID.to_string(),
        format!("other_{}", "b".repeat(64)),
        format!("evt_{}", "c".repeat(60)),
        format!("evt_{}", "d".repeat(61)),
        format!("evt_{}", "é".repeat(31)),
        "legacy-id".to_string(),
    ];
    for (index, id) in ids.iter().enumerate() {
        let mut envelope = external::Envelope {
            view: Some(codex_protocol::external_input_view::View {
                schema: "test.v1".into(),
                data: json!({"sentinel":"NON_MODEL_VIEW_SENTINEL"}),
            }),
            version: 2,
            owner_id: "owner".into(),
            thread_id: "thread".into(),
            runtime_generation: 1,
            message: external::Message {
                id: id.clone(),
                source: external::Source {
                    kind: external::SourceKind::Service,
                    id: "service".into(),
                },
                event_type: "opaque.v1".into(),
                delivery: external::Delivery::AfterTurn,
                text: "unchanged data".into(),
            },
            semantic_sha256: String::new(),
        };
        envelope.semantic_sha256 = envelope.digest();
        let commit = external::Commit {
            receipt: external::Receipt::new(&envelope, "turn".into(), 1)?,
            envelope,
        };
        let item = commit.envelope.response_item()?;
        // Simulate loading the already persisted pair; never normalize its IDs.
        let persisted = serde_json::to_vec(&(commit, item))?;
        let (commit, item): (external::Commit, ResponseItem) = serde_json::from_slice(&persisted)?;
        let recovery = external::recover(
            "owner",
            "thread",
            &[
                external::HistoryEntry::Commit(&commit),
                external::HistoryEntry::Item {
                    item: &item,
                    turn_id: "turn",
                },
            ],
        )?;
        let receipt_before = recovery
            .lookup(&commit.envelope)?
            .unwrap()
            .commit
            .receipt
            .clone();
        // Since 0.162 base instructions are a prefixed developer message; omit them here.
        let prompt = Prompt {
            input: vec![item.clone()],
            base_instructions: codex_protocol::models::BaseInstructions {
                text: String::new(),
                provenance: None,
            },
            ..Default::default()
        };
        let mut request = client.build_responses_request(
            &prompt,
            &test_model_info(),
            /*effort*/ None,
            codex_protocol::config_types::ReasoningSummary::None,
            /*service_tier*/ None,
            &test_responses_metadata_for_client(
                &client,
                /*turn_id*/ None,
                format!("{}:0", client.state.thread_id),
                /*parent_thread_id*/ None,
                TestCodexResponsesRequestKind::Turn,
            ),
            /*include_internal*/ false,
        )?;
        client.prepare_response_items_for_request(&mut request.input);
        let serialized = serde_json::to_value(&request)?;
        assert!(!serialized.to_string().contains("NON_MODEL_VIEW_SENTINEL"));
        let mut expected = serde_json::to_value(&item)?;
        if index != 2 {
            expected.as_object_mut().unwrap().remove("id");
        }
        assert_eq!(serialized["input"], json!([expected]));
        assert_eq!(prompt.input, vec![item.clone()]);
        assert_eq!(serde_json::to_vec(&(commit.clone(), item))?, persisted);
        assert_eq!(
            recovery.lookup(&commit.envelope)?.unwrap().commit.receipt,
            receipt_before
        );
        let mut conflict = commit.envelope.clone();
        conflict.message.text.push('!');
        conflict.semantic_sha256 = conflict.digest();
        assert!(recovery.lookup(&conflict).is_err());
        // Projection is repeatable; no generated provider identity or collisions.
        client.prepare_response_items_for_request(&mut request.input);
        assert_eq!(serde_json::to_value(&request)?, serialized);
    }
    Ok(())
}

#[test]
fn strict_responses_providers_project_unpaired_external_input_as_user_message() {
    let external = ResponseItem::FunctionCallOutput {
        id: Some(ResponseItemId::from_server("evt_external".to_string())),
        call_id: None,
        name: Some("external_event".to_string()),
        namespace: Some("external".to_string()),
        output: FunctionCallOutputPayload::from_text(
            json!({"source":{"kind":"service","id":"build"},"type":"message","text":"hello"})
                .to_string(),
        ),
        internal_chat_message_metadata_passthrough: None,
    };
    let paired = ResponseItem::FunctionCallOutput {
        id: Some(ResponseItemId::from_server("fco_valid".to_string())),
        call_id: Some("call_preserved".to_string()),
        name: Some("shell".to_string()),
        namespace: None,
        output: FunctionCallOutputPayload::from_text("tool result".to_string()),
        internal_chat_message_metadata_passthrough: None,
    };
    let mut request_items = vec![external.clone(), paired.clone()];

    ModelClient::project_unpaired_external_events_for_strict_responses(&mut request_items);

    assert_eq!(
        request_items[0],
        ResponseItem::Message {
            id: Some(ResponseItemId::from_server("evt_external".to_string())),
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: json!({"source":{"kind":"service","id":"build"},"type":"message","text":"hello"})
                    .to_string(),
            }],
            phase: None,
            encrypted_content: None,
            status: None,
            internal_chat_message_metadata_passthrough: None,
        }
    );
    assert_eq!(request_items[1], paired);
    assert!(matches!(external, ResponseItem::FunctionCallOutput { .. }));
}

#[tokio::test]
async fn provider_item_id_projection_reaches_serialized_http_input() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(/*status*/ 200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\"}}\n\n"),
        )
        .expect(/*requests*/ 1)
        .mount(&server)
        .await;
    let client = ModelClient::new(
        /*auth_manager*/ None,
        AgentIdentityAuthPolicy::JwtOnly,
        ThreadId::new(),
        create_oss_provider_with_base_url(&server.uri(), WireApi::Responses),
        SessionSource::Cli,
        "test_originator".into(),
        /*model_verbosity*/ None,
        /*content_item_kinds_enabled*/ true,
        /*reasoning_effort_override_enabled*/ false,
        /*enable_request_compression*/ false,
        /*include_timing_metrics*/ false,
        /*beta_features_header*/ None,
        /*concurrent_reasoning_summaries_enabled*/ false,
        /*output_token_replay_enabled*/ false,
        /*attestation_provider*/ None,
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
        codex_model_provider::WorkspaceRoutingContext::new(server.uri()),
        Vec::new(),
    );
    let input = json!([
        {"type":"function_call_output","id":ORIGINAL_ID,"name":"external_event","namespace":"external","output":"first"},
        {"type":"function_call_output","id":format!("event_{}", "z".repeat(64)),"name":"external_event","namespace":"external","output":"second"},
        {"type":"function_call_output","id":"fco_valid","call_id":"call_preserved","output":"tool result"},
        {"type":"message","id":"msg_valid","role":"assistant","content":[{"type":"output_text","text":"reply"}]}
    ]);
    let prompt = Prompt {
        input: serde_json::from_value(input.clone())?,
        base_instructions: codex_protocol::models::BaseInstructions {
            text: String::new(),
            provenance: None,
        },
        ..Default::default()
    };
    let metadata = test_responses_metadata_for_client(
        &client,
        /*turn_id*/ None,
        format!("{}:0", client.state.thread_id),
        /*parent_thread_id*/ None,
        TestCodexResponsesRequestKind::Turn,
    );
    let mut session = client.new_session();
    session.set_external_input_attempt(true);
    let mut stream = session
        .stream(
            &prompt,
            &test_model_info(),
            &test_session_telemetry(),
            /*effort*/ None,
            codex_protocol::config_types::ReasoningSummary::None,
            /*service_tier*/ None,
            &metadata,
            &InferenceTraceContext::disabled(),
        )
        .await?;
    while let Some(event) = stream.next().await {
        event?;
    }
    // Remote compaction v2 now uses the same /responses stream and item projection.
    let requests = server.received_requests().await.unwrap();
    let expected = json!([
        {
            "type":"message",
            "role":"user",
            "content":[{"type":"input_text","text":"first"}]
        },
        {
            "type":"message",
            "role":"user",
            "content":[{"type":"input_text","text":"second"}]
        },
        {"type":"function_call_output","id":"fco_valid","call_id":"call_preserved","output":"tool result"},
        {"type":"message","id":"msg_valid","role":"assistant","content":[{"type":"output_text","text":"reply"}]}
    ]);
    assert_eq!(requests.len(), 1);
    for request in requests {
        let sent: serde_json::Value = serde_json::from_slice(&request.body)?;
        assert_eq!(sent["input"], expected);
    }
    assert_eq!(
        serde_json::to_value(&prompt.input)?[0]["id"],
        input[0]["id"]
    );
    Ok(())
}
