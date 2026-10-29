//! Costs distinguish evidence payloads from complete model request estimates.

use super::*;
use crate::Budgeted;
use crate::composition::SectionOutput;
use crate::composition::user_message as message;
use codex_protocol::models::ImageReference;
use pretty_assertions::assert_eq;

#[test]
fn request_token_ranges_keep_boundary_values_in_the_lower_bucket() {
    let tokens = [
        0,
        19_999,
        20_000,
        20_001,
        40_000,
        40_001,
        240_000,
        240_001,
        256_000,
        256_001,
        usize::MAX,
    ];
    assert_eq!(
        tokens.map(request_token_bucket),
        [
            "01_0-20k",
            "01_0-20k",
            "01_0-20k",
            "02_20k-40k",
            "02_20k-40k",
            "03_40k-60k",
            "12_220k-240k",
            "13_240k-256k",
            "13_240k-256k",
            "14_256k-max",
            "14_256k-max"
        ]
    );
}

#[test]
fn section_costs_keep_multimodal_payloads_separate() {
    let context = ComposedContext {
        sections: vec![
            SectionOutput {
                id: "transcript",
                delivery: SectionDelivery::user_content(vec![
                    Budgeted::required(ContentItem::InputText {
                        text: "évidence".to_owned(),
                    }),
                    Budgeted::required(ContentItem::InputImage {
                        image: ImageReference::Inline {
                            image_url: "data:image/png;base64,AAAA".to_owned(),
                        },
                        detail: None,
                    }),
                    Budgeted::required(ContentItem::InputImage {
                        image: ImageReference::File {
                            file_id: "file_123".to_owned(),
                        },
                        detail: None,
                    }),
                ]),
            },
            SectionOutput {
                id: "trusted",
                delivery: SectionDelivery::Message(crate::Budgeted::required(Box::new(
                    message(vec![ContentItem::InputText {
                        text: "verified".to_owned(),
                    }])
                    .into(),
                ))),
            },
        ],
        truncations: Vec::new(),
    };
    assert_eq!(
        context.section_costs().collect::<Vec<_>>(),
        vec![
            (
                "transcript",
                SectionCost {
                    text_bytes: "évidence".len(),
                    image_bytes: "data:image/png;base64,AAAA".len(),
                    image_count: 2
                }
            ),
            (
                "trusted",
                SectionCost {
                    text_bytes: "verified".len(),
                    ..SectionCost::default()
                }
            ),
        ]
    );
}

#[test]
fn request_estimate_reserves_images_independently_of_encoded_size() {
    let image = |payload: &str| {
        message(vec![ContentItem::InputImage {
            image: ImageReference::Inline {
                image_url: format!("data:image/png;base64,{payload}"),
            },
            detail: None,
        }])
    };
    let short = estimate_input_tokens(&image("AAAA"));
    assert_eq!(short, estimate_input_tokens(&image(&"A".repeat(200_000))));
    assert!(short >= IMAGE_TOKEN_RESERVATION);

    let file = message(vec![ContentItem::InputImage {
        image: ImageReference::File {
            file_id: "file_123".to_owned(),
        },
        detail: Some(codex_protocol::models::ImageDetail::Original),
    }]);
    assert!(estimate_input_tokens(&file) >= IMAGE_TOKEN_RESERVATION);
}

#[test]
fn section_estimate_bounds_the_delivered_message() {
    // Account for both JSON escaping layers and UTF-8-safe chunk boundaries.
    for format in [crate::TranscriptFormat::Line, crate::TranscriptFormat::Json] {
        for (text, max_count) in [
            ("x".to_owned(), 40),
            ("quoted \"text\"".to_owned(), 40),
            ("é🙂\n\0\\".repeat(20_000), 1),
        ] {
            let mut record = crate::TranscriptRecord::new(
                &crate::ConversationTranscriptEntryKind::Assistant,
                /*index*/ 7,
                text,
                /*retained_source_order*/ None,
                format,
                /*suffix*/ "",
            );
            // Composition changes framing after preparation has populated the cache.
            record.frame_for_sync("\n");
            let expected = ContentItem::InputText {
                text: record.to_string(),
            };
            assert_eq!(record.rendered().tokens, content_tokens(&expected));
            assert_eq!(record.clone().into_content_item(), expected);
            // Individually rounded costs must still leave room for separators.
            for count in 1..=max_count {
                let context = ComposedContext {
                    sections: vec![SectionOutput {
                        id: "transcript",
                        delivery: SectionDelivery::UserContent(vec![
                            Budgeted::required(
                                record.clone().into()
                            );
                            count
                        ]),
                    }],
                    truncations: Vec::new(),
                };
                assert_eq!(
                    context.section_costs().collect::<Vec<_>>(),
                    vec![(
                        "transcript",
                        SectionCost {
                            text_bytes: record.to_string().len() * count,
                            ..SectionCost::default()
                        }
                    )]
                );
                let estimate = context.estimated_tokens();
                assert!(estimate >= estimate_input_tokens(&context.into_messages()[0]));
            }
        }
    }
}

fn saved_budget_image(byte_size: u64) -> ImageReference {
    ImageReference::Saved {
        saved_image: codex_protocol::saved_image::SavedImage {
            saved_path: "/missing/never-read/quoted-\"-image.png".into(),
            mime_type: "image/png".into(),
            sha256: "0".repeat(64),
            byte_size,
            width: 384,
            height: 768,
        },
    }
}

#[test]
fn saved_image_message_budget_and_section_cost_equal_inline_without_file_io() {
    use codex_protocol::models::ImageDetail;
    for size in 1u64..=9 {
        let payload = format!(
            "{}{}",
            "AAAA".repeat((size / 3) as usize),
            match size % 3 {
                1 => "AA==",
                2 => "AAA=",
                _ => "",
            }
        );
        let url = format!("data:image/png;base64,{payload}");
        for detail in [None, Some(ImageDetail::Original), Some(ImageDetail::High)] {
            let inline = ContentItem::InputImage {
                image: ImageReference::Inline {
                    image_url: url.clone(),
                },
                detail,
            };
            let saved = ContentItem::InputImage {
                image: saved_budget_image(size),
                detail,
            };
            assert_eq!(content_tokens(&saved), content_tokens(&inline));
            assert_eq!(
                estimate_input_tokens(&message(vec![saved.clone()])),
                estimate_input_tokens(&message(vec![inline.clone()]))
            );
            let context = |image| ComposedContext {
                sections: vec![SectionOutput {
                    id: "images",
                    delivery: SectionDelivery::user_content(vec![Budgeted::required(image)]),
                }],
                truncations: Vec::new(),
            };
            assert_eq!(
                context(saved).section_costs().collect::<Vec<_>>(),
                context(inline).section_costs().collect::<Vec<_>>()
            );
            assert_eq!(image_reference_bytes(&saved_budget_image(size)), url.len());
        }
    }
}

#[test]
fn saved_tool_output_budget_preserves_inline_wire_cost() {
    use serde_json::json;
    for kind in ["function_call_output", "custom_tool_call_output"] {
        let inline: ResponseItem = serde_json::from_value(json!({
            "type":kind,"call_id":"c","output":[{
                "type":"input_image","image_url":"data:image/png;base64,AA==","detail":"original"
            }]
        }))
        .unwrap();
        let mut saved = serde_json::to_value(&inline).unwrap();
        saved["output"][0]
            .as_object_mut()
            .unwrap()
            .remove("image_url");
        let ImageReference::Saved { saved_image } = saved_budget_image(1) else {
            unreachable!()
        };
        saved["output"][0]["saved_image"] = serde_json::to_value(saved_image).unwrap();
        let saved: ResponseItem = serde_json::from_value(saved).unwrap();
        assert_eq!(
            estimate_input_tokens(&saved),
            estimate_input_tokens(&inline)
        );
        assert_eq!(
            estimate_input_tokens_from_serialized_len(
                &saved,
                serde_json::to_vec(&saved).unwrap().len()
            ),
            estimate_input_tokens(&inline)
        );
    }
}

#[test]
fn saved_image_url_size_saturates_instead_of_wrapping() {
    assert_eq!(
        image_reference_bytes(&saved_budget_image(u64::MAX)),
        usize::MAX
    );
}
