use super::parse_saved_image_path;
use super::put_away_image_placeholder;
use crate::context_manager::ContextManager;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ImageReference;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::default_input_modalities;
use codex_utils_output_truncation::TruncationPolicy;
use pretty_assertions::assert_eq;

fn image(tag: &str) -> FunctionCallOutputContentItem {
    FunctionCallOutputContentItem::InputImage {
        image: ImageReference::Inline {
            image_url: format!("data:image/png;base64,{tag}"),
        },
        detail: None,
    }
}

fn saved_path(call_id: &str) -> String {
    format!("/home/u/.codex/generated_images/thread/{call_id}.png")
}

fn hint(call_id: &str) -> FunctionCallOutputContentItem {
    FunctionCallOutputContentItem::InputText {
        text: format!(
            "Generated images are saved to /home/u/.codex/generated_images/thread as {} by default.\nIf you need to use a generated image at another path, copy it.",
            saved_path(call_id)
        ),
    }
}

fn function_call(call_id: &str, name: &str) -> ResponseItem {
    ResponseItem::FunctionCall {
        id: None,
        name: name.to_string(),
        namespace: None,
        arguments: "{}".to_string(),
        encrypted_function_args: None,
        call_id: call_id.to_string(),
        encrypted_content: None,
        status: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn function_output(call_id: &str, parts: Vec<FunctionCallOutputContentItem>) -> ResponseItem {
    ResponseItem::FunctionCallOutput {
        id: None,
        call_id: Some(call_id.to_string()),
        name: None,
        namespace: None,
        output: FunctionCallOutputPayload::from_content_items(parts),
        internal_chat_message_metadata_passthrough: None,
    }
}

/// An image tool call with a saved file.
fn generated(call_id: &str) -> Vec<ResponseItem> {
    vec![
        function_call(call_id, "image_gen"),
        function_output(call_id, vec![image(call_id), hint(call_id)]),
    ]
}

/// A view_image result: an image without the saved-file hint.
fn viewed(call_id: &str) -> Vec<ResponseItem> {
    vec![
        function_call(call_id, "view_image"),
        function_output(call_id, vec![image(call_id)]),
    ]
}

fn user_image_message() -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![
            ContentItem::InputText {
                text: "look".to_string(),
            },
            ContentItem::InputImage {
                image: ImageReference::Inline {
                    image_url: "data:image/png;base64,user".to_string(),
                },
                detail: None,
            },
        ],
        phase: None,
        encrypted_content: None,
        status: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn history_with(items: &[ResponseItem]) -> ContextManager {
    let mut history = ContextManager::new();
    history.record_items(items.iter(), TruncationPolicy::Tokens(10_000));
    history
}

fn record(history: &mut ContextManager, items: &[ResponseItem]) {
    history.record_items(items.iter(), TruncationPolicy::Tokens(10_000));
}

fn generated_calls(range: std::ops::Range<usize>) -> Vec<ResponseItem> {
    range.flat_map(|n| generated(&format!("img{n}"))).collect()
}

/// Call ids whose image is a placeholder in the prompt, in order.
fn placeholders_in_prompt(history: &ContextManager) -> Vec<String> {
    history
        .clone()
        .for_prompt(&default_input_modalities())
        .iter()
        .filter_map(|item| match item {
            ResponseItem::FunctionCallOutput {
                call_id: Some(call_id),
                output,
                ..
            }
            | ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => Some((call_id.clone(), output)),
            _ => None,
        })
        .filter(|(call_id, output)| match &output.body {
            FunctionCallOutputBody::ContentItems(parts) => {
                parts.first()
                    == Some(&FunctionCallOutputContentItem::InputText {
                        text: put_away_image_placeholder(&saved_path(call_id)),
                    })
            }
            FunctionCallOutputBody::Text(_) => false,
        })
        .map(|(call_id, _)| call_id)
        .collect()
}

fn base_instructions() -> BaseInstructions {
    BaseInstructions {
        text: String::new(),
        provenance: None,
    }
}

fn names(range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|n| format!("img{n}")).collect()
}

#[test]
fn parses_saved_path_from_hint() {
    let FunctionCallOutputContentItem::InputText { text } = hint("a") else {
        unreachable!()
    };
    assert_eq!(
        parse_saved_image_path(&text),
        Some(saved_path("a").as_str())
    );
    assert_eq!(
        parse_saved_image_path(
            r"Generated images are saved to C:\Users\A B\gen as C:\Users\A B\gen\x as y.png by default."
        ),
        Some(r"C:\Users\A B\gen\x as y.png")
    );
    assert_eq!(parse_saved_image_path("some other text"), None);
}

#[test]
fn below_threshold_nothing_changes() {
    let mut history = history_with(&generated_calls(0..20));
    assert_eq!(history.put_away_generated_images(20, 5), 0);
    assert_eq!(placeholders_in_prompt(&history), Vec::<String>::new());
}

#[test]
fn above_threshold_puts_away_all_but_newest_keep_and_keeps_raw_items() {
    let items = generated_calls(0..21);
    let mut history = history_with(&items);
    let tokens_before = history
        .estimate_token_count_with_base_instructions(&base_instructions())
        .unwrap();

    assert_eq!(history.put_away_generated_images(20, 5), 16);
    assert_eq!(placeholders_in_prompt(&history), names(0..16));
    // Raw history keeps every image.
    assert_eq!(history.raw_items().cloned().collect::<Vec<_>>(), items);
    // The hint after a put-away image is untouched.
    let prompt = history.clone().for_prompt(&default_input_modalities());
    let ResponseItem::FunctionCallOutput { output, .. } = &prompt[1] else {
        panic!("expected function output, got {:?}", prompt[1]);
    };
    assert_eq!(
        output.body,
        FunctionCallOutputBody::ContentItems(vec![
            FunctionCallOutputContentItem::InputText {
                text: put_away_image_placeholder(&saved_path("img0")),
            },
            hint("img0"),
        ])
    );
    let tokens_after = history
        .estimate_token_count_with_base_instructions(&base_instructions())
        .unwrap();
    assert!(
        tokens_after < tokens_before,
        "{tokens_after} < {tokens_before}"
    );
}

#[test]
fn other_images_are_never_put_away_but_count_toward_newest_keep() {
    let mut items = generated_calls(0..21);
    items.extend(viewed("view0"));
    items.push(user_image_message());
    items.extend(viewed("view1"));
    let mut history = history_with(&items);

    // The newest 5 images are view1, the user image, view0, img20 and img19.
    assert_eq!(history.put_away_generated_images(20, 5), 19);
    assert_eq!(placeholders_in_prompt(&history), names(0..19));
}

#[test]
fn second_batch_waits_for_threshold_so_prefix_stays_stable() {
    let mut history = history_with(&generated_calls(0..21));
    assert_eq!(history.put_away_generated_images(20, 5), 16);
    let first_prompt = history.clone().for_prompt(&default_input_modalities());

    // One more image: 6 inline, below the threshold. Nothing changes, the prefix is identical.
    record(&mut history, &generated("img21"));
    assert_eq!(history.put_away_generated_images(20, 5), 0);
    assert_eq!(history.put_away_generated_images(20, 5), 0);
    let second_prompt = history.clone().for_prompt(&default_input_modalities());
    assert_eq!(second_prompt[..first_prompt.len()], first_prompt[..]);

    // Up to 21 inline again (img16..img36): the next batch.
    record(&mut history, &generated_calls(22..36));
    assert_eq!(history.put_away_generated_images(20, 5), 0);
    record(&mut history, &generated("img36"));
    assert_eq!(history.put_away_generated_images(20, 5), 16);
    assert_eq!(placeholders_in_prompt(&history), names(0..32));

    // A clone carries the put-away set.
    assert_eq!(placeholders_in_prompt(&history.clone()), names(0..32));
}

#[test]
fn code_mode_custom_tool_outputs_are_put_away() {
    let items: Vec<ResponseItem> = (0..4)
        .flat_map(|n| {
            let call_id = format!("img{n}");
            vec![
                ResponseItem::CustomToolCall {
                    id: None,
                    status: None,
                    call_id: call_id.clone(),
                    name: "exec".to_string(),
                    namespace: None,
                    input: "generate()".to_string(),
                    encrypted_content: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::CustomToolCallOutput {
                    id: None,
                    call_id: call_id.clone(),
                    name: None,
                    output: FunctionCallOutputPayload::from_content_items(vec![
                        FunctionCallOutputContentItem::InputText {
                            text: "Script completed".to_string(),
                        },
                        image(&call_id),
                        hint(&call_id),
                    ]),
                    internal_chat_message_metadata_passthrough: None,
                },
            ]
        })
        .collect();
    let mut history = history_with(&items);

    assert_eq!(history.put_away_generated_images(3, 1), 3);
    let prompt = history.clone().for_prompt(&default_input_modalities());
    let ResponseItem::CustomToolCallOutput { output, .. } = &prompt[1] else {
        panic!("expected custom tool output, got {:?}", prompt[1]);
    };
    assert_eq!(
        output.body,
        FunctionCallOutputBody::ContentItems(vec![
            FunctionCallOutputContentItem::InputText {
                text: "Script completed".to_string(),
            },
            FunctionCallOutputContentItem::InputText {
                text: put_away_image_placeholder(&saved_path("img0")),
            },
            hint("img0"),
        ])
    );
    let ResponseItem::CustomToolCallOutput { output, .. } = &prompt[7] else {
        panic!("expected custom tool output, got {:?}", prompt[7]);
    };
    assert!(matches!(
        &output.body,
        FunctionCallOutputBody::ContentItems(parts)
            if matches!(parts[1], FunctionCallOutputContentItem::InputImage { .. })
    ));
}
