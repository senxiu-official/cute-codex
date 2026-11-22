use super::*;
use serde_json::json;

fn params(prefix: serde_json::Value) -> TurnStartParams {
    serde_json::from_value(json!({
        "threadId": "thread", "clientUserMessageId": "carrier",
        "input": [{"type":"text","text":"/literal/path"}], "ownerInputs": prefix
    }))
    .unwrap()
}

#[test]
fn owner_input_batch_prefix_order_and_empty_opt_in() {
    let request = params(json!([
        {"clientUserMessageId":"a","input":[{"type":"text","text":"a"}]},
        {"clientUserMessageId":"b","input":[{"type":"mention","name":"skill","path":"/skills/x"}]}
    ]));
    request.validate_owner_input_batch().unwrap();
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(wire["ownerInputs"][0]["clientUserMessageId"], "a");
    assert_eq!(wire["ownerInputs"][1]["input"][0]["type"], "mention");
    let mut empty = params(json!([]));
    empty.input =
        serde_json::from_value(json!([{"type":"image","url":"data:image/png;base64,AA=="}]))
            .unwrap();
    empty.validate_owner_input_batch().unwrap();
    assert_eq!(
        serde_json::to_value(empty).unwrap()["ownerInputs"],
        json!([])
    );
}

#[test]
fn owner_input_batch_rejects_identity_aliases_and_invalid_parts() {
    let one = json!({"clientUserMessageId":"a","input":[{"type":"text","text":"a"}]});
    for prefix in [
        json!([one.clone(), one]),
        json!([{"clientUserMessageId":"carrier","input":[{"type":"text","text":"x"}]}]),
        json!([{"clientUserMessageId":"a","input":[]}]),
        json!([{"clientUserMessageId":"a","input":[{"type":"localAudio","path":"x"}]}]),
    ] {
        assert!(params(prefix).validate_owner_input_batch().is_err());
    }
    let mut request = params(json!([]));
    request.client_user_message_id = Some("界".repeat(86));
    assert!(request.validate_owner_input_batch().is_err());
    request.client_user_message_id = Some("a".repeat(256));
    request.validate_owner_input_batch().unwrap();
}

#[test]
fn owner_input_batch_budget_is_whole_request() {
    let prefix: Vec<_> = (0..99).map(|index| json!({"clientUserMessageId":format!("p{index}"), "input":[{"type":"text","text":"a"}]})).collect();
    let mut request = params(json!(prefix));
    request.validate_owner_input_batch().unwrap();
    request.owner_inputs.as_mut().unwrap().push(OwnerInput {
        client_user_message_id: "extra".into(),
        input: request.input.clone(),
    });
    assert!(request.validate_owner_input_batch().is_err());
    request = params(
        json!([{"clientUserMessageId":"a","input":[{"type":"text","text":"a".repeat(1024*1024)}]}]),
    );
    assert!(request.validate_owner_input_batch().is_err());
}

#[test]
fn owner_input_batch_null_and_unknown_member_fields_do_not_downgrade() {
    let schema = serde_json::to_value(schemars::schema_for!(TurnStartParams)).unwrap();
    assert_eq!(schema["properties"]["ownerInputs"]["type"], "array");
    for prefix in [
        serde_json::Value::Null,
        json!([{"clientUserMessageId":"a","input":[],"threadId":"other"}]),
    ] {
        assert!(
            serde_json::from_value::<TurnStartParams>(
                json!({"threadId":"thread","input":[],"ownerInputs":prefix})
            )
            .is_err()
        );
    }
    let legacy: TurnStartParams =
        serde_json::from_value(json!({"threadId":"thread","input":[]})).unwrap();
    assert!(legacy.owner_inputs.is_none());
    legacy.validate_owner_input_batch().unwrap();
}
