use std::borrow::Cow;

use crate::RolloutItem;
use crate::protocol::EventMsg;
use codex_extension_items::ExtensionItem;
use codex_extension_items::image_generation::GeneratedImageRef;
use codex_protocol::items::CommandExecutionItem;
use codex_protocol::items::McpToolCallItem;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::SubAgentActivityKind;
use codex_protocol::protocol::ThreadHistoryMode;
use codex_utils_output_truncation::truncate_mcp_tool_result;
use codex_utils_string::truncate_middle_with_marker;

const PERSISTED_MCP_RESULT_MAX_BYTES: usize = 64 * 1024;
pub(super) const PERSISTED_COMMAND_OUTPUT_MAX_BYTES: usize = 64 * 1024;
const PERSISTED_COMMAND_OUTPUT_TRUNCATION_MARKER: &str =
    "\n... command output truncated for persistence ...\n";

/// Returns the authoritative durable representation of a rollout item.
pub fn persisted_rollout_item(
    item: &RolloutItem,
    history_mode: ThreadHistoryMode,
) -> Option<Cow<'_, RolloutItem>> {
    match item {
        RolloutItem::ResponseItem(response_item) => {
            should_persist_response_item(&response_item.item).then_some(Cow::Borrowed(item))
        }
        RolloutItem::InterAgentCommunication(_)
        | RolloutItem::InterAgentCommunicationMetadata { .. } => Some(Cow::Borrowed(item)),
        RolloutItem::EventMsg(ev) => {
            persisted_event_msg(ev, history_mode).map(|event| match event {
                Cow::Borrowed(_) => Cow::Borrowed(item),
                Cow::Owned(event) => Cow::Owned(RolloutItem::EventMsg(event)),
            })
        }
        RolloutItem::ExternalInput(_) => Some(Cow::Borrowed(item)),
        RolloutItem::RealtimeItem(_) => {
            matches!(history_mode, ThreadHistoryMode::Paginated).then_some(Cow::Borrowed(item))
        }
        // Persist Codex executive markers so we can analyze flows (e.g., compaction, API turns).
        RolloutItem::Compacted(_)
        | RolloutItem::TurnContext(_)
        | RolloutItem::TokenUsageRecord(_)
        | RolloutItem::WorldState(_)
        | RolloutItem::RetainedContext(_)
        | RolloutItem::SecurityRiskScore(_)
        | RolloutItem::SessionMeta(_) => Some(Cow::Borrowed(item)),
    }
}

/// [`persisted_rollout_item`] for a writer: a generated image already saved to disk is
/// written by path only (see [`strip_saved_generated_image`]). Readers keep using
/// [`persisted_rollout_item`], so older rollouts still return their inline bytes.
pub fn persisted_rollout_item_for_write(
    item: &RolloutItem,
    history_mode: ThreadHistoryMode,
) -> Option<Cow<'_, RolloutItem>> {
    let persisted = persisted_rollout_item(item, history_mode)?;
    if !has_saved_generated_image_bytes(persisted.as_ref()) {
        return Some(persisted);
    }
    let mut item = persisted.into_owned();
    strip_saved_generated_image(&mut item);
    Some(Cow::Owned(item))
}

/// Return the rollout items that should be persisted for a live append.
pub fn persisted_rollout_items(
    items: &[RolloutItem],
    history_mode: ThreadHistoryMode,
) -> Vec<RolloutItem> {
    items
        .iter()
        .filter_map(|item| {
            persisted_rollout_item_for_write(item, history_mode).map(Cow::into_owned)
        })
        .collect()
}

/// Return the owned rollout items that should be persisted for a live append.
pub fn into_persisted_rollout_items(
    items: Vec<RolloutItem>,
    history_mode: ThreadHistoryMode,
) -> Vec<RolloutItem> {
    items
        .into_iter()
        .filter_map(
            |item| match persisted_rollout_item_for_write(&item, history_mode) {
                Some(Cow::Borrowed(_)) => Some(item),
                Some(Cow::Owned(item)) => Some(item),
                None => None,
            },
        )
        .collect()
}

fn has_saved_generated_image_bytes(item: &RolloutItem) -> bool {
    match item {
        RolloutItem::EventMsg(EventMsg::ItemCompleted(event)) => matches!(
            &event.item,
            TurnItem::Extension(ExtensionItem::ImageGeneration(image))
                if image.saved_path.is_some() && matches!(&image.image, GeneratedImageRef::Inline { result } if !result.is_empty())
        ),
        RolloutItem::EventMsg(EventMsg::ImageGenerationEnd(event)) => {
            event.saved_path.is_some()
                && matches!(&event.image, GeneratedImageRef::Inline { result } if !result.is_empty())
        }
        _ => false,
    }
}

/// The persisted form of a completed generated image whose bytes were saved to
/// `saved_path`: the path is kept and the inline base64 `result` is dropped.
///
/// The image-generation item is a UI/history item only; the model sees the image
/// through its prepared tool output. The writer also externalizes those pixels;
/// request construction restores their exact bytes. A single generated image is
/// several MB of base64, so a thread that generates many images
/// otherwise grows its rollout and thread history by gigabytes. Readers that need
/// the pixels load `saved_path`. Live notifications still carry `result`.
/// Returns whether anything was stripped.
pub fn strip_saved_generated_image(item: &mut RolloutItem) -> bool {
    match item {
        RolloutItem::EventMsg(EventMsg::ItemCompleted(event)) => match &mut event.item {
            TurnItem::Extension(ExtensionItem::ImageGeneration(image))
                if image.saved_path.is_some()
                    && matches!(&image.image, GeneratedImageRef::Inline { result } if !result.is_empty()) =>
            {
                if let GeneratedImageRef::Inline { result } = &mut image.image {
                    result.clear();
                }
                true
            }
            _ => false,
        },
        RolloutItem::EventMsg(EventMsg::ImageGenerationEnd(event))
            if event.saved_path.is_some()
                && matches!(&event.image, GeneratedImageRef::Inline { result } if !result.is_empty()) =>
        {
            if let GeneratedImageRef::Inline { result } = &mut event.image {
                result.clear();
            }
            true
        }
        _ => false,
    }
}

/// Replace generated tool images in the writer's copy only. Unknown hints or
/// sidecar I/O failure retain inline bytes rather than lose data.
pub(crate) fn externalize_saved_tool_images(item: &mut RolloutItem, codex_home: &std::path::Path) {
    match item {
        RolloutItem::ResponseItem(envelope) => {
            externalize_response_images(&mut envelope.item, codex_home, false)
        }
        RolloutItem::Compacted(compacted) => {
            for envelope in compacted.replacement_history.iter_mut().flatten() {
                externalize_response_images(&mut envelope.item, codex_home, false);
            }
            if let Some(checkpoint) = &mut compacted.guardian_history {
                for envelope in &mut checkpoint.0 {
                    externalize_response_images(&mut envelope.item, codex_home, true);
                }
            }
        }
        _ => {}
    }
}

fn externalize_one(
    image: &mut codex_protocol::models::ImageReference,
    path: &str,
    home: &std::path::Path,
) {
    use codex_protocol::models::ImageReference;
    let ImageReference::Inline { image_url } = image else {
        return;
    };
    match codex_protocol::saved_image::externalize(image_url, std::path::Path::new(path), home) {
        Ok(Some(saved)) => *image = saved,
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, "could not save generated image reference; retaining inline history")
        }
    }
}

fn externalize_response_images(item: &mut ResponseItem, home: &std::path::Path, guardian: bool) {
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::FunctionCallOutputContentItem;
    use codex_protocol::saved_image::parse_saved_image_path;
    match item {
        ResponseItem::FunctionCallOutput { output, .. }
        | ResponseItem::CustomToolCallOutput { output, .. } => {
            let Some(parts) = output.content_items_mut() else {
                return;
            };
            for index in 0..parts.len().saturating_sub(1) {
                let path = match &parts[index + 1] {
                    FunctionCallOutputContentItem::InputText { text } => {
                        parse_saved_image_path(text).map(str::to_owned)
                    }
                    _ => None,
                };
                if let (Some(path), FunctionCallOutputContentItem::InputImage { image, .. }) =
                    (path, &mut parts[index])
                {
                    externalize_one(image, &path, home);
                }
            }
        }
        // Guardian checkpoints represent trusted tool evidence as user messages.
        ResponseItem::Message { content, .. } if guardian => {
            for index in 0..content.len().saturating_sub(1) {
                let path = match &content[index + 1] {
                    ContentItem::InputText { text } => {
                        parse_saved_image_path(text).map(str::to_owned)
                    }
                    _ => None,
                };
                if let (Some(path), ContentItem::InputImage { image, .. }) =
                    (path, &mut content[index])
                {
                    externalize_one(image, &path, home);
                }
            }
        }
        _ => {}
    }
}

/// Whether a `ResponseItem` should be persisted in rollout files.
#[inline]
pub fn should_persist_response_item(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::AdditionalTools { .. }
        | ResponseItem::Message { .. }
        | ResponseItem::AgentMessage { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::ConfigurationUpdate { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ContextCompaction { .. } => true,
        ResponseItem::CompactionTrigger { .. } | ResponseItem::Other => false,
    }
}

/// Whether a `ResponseItem` should be persisted for the memories.
#[inline]
pub fn should_persist_response_item_for_memories(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::Message { role, .. } => role != "developer",
        ResponseItem::AgentMessage { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::WebSearchCall { .. } => true,
        ResponseItem::AdditionalTools { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::ConfigurationUpdate { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::CompactionTrigger { .. }
        | ResponseItem::ContextCompaction { .. }
        | ResponseItem::Other => false,
    }
}

fn persisted_event_msg(
    ev: &EventMsg,
    history_mode: ThreadHistoryMode,
) -> Option<Cow<'_, EventMsg>> {
    match ev {
        EventMsg::ItemCompleted(event) => persisted_item_completed_event(ev, event, history_mode),
        EventMsg::PresentationAppended(_)
        | EventMsg::TokenCount(_)
        | EventMsg::ThreadGoalUpdated(_)
        | EventMsg::ThreadRolledBack(_)
        | EventMsg::TurnAborted(_)
        | EventMsg::TurnStarted(_)
        | EventMsg::TurnComplete(_)
        | EventMsg::ThreadSettingsApplied(_) => Some(Cow::Borrowed(ev)),

        // Only persist these legacy events when the thread's history mode is Legacy.
        // New, paginated rollouts persist ItemCompleted events with TurnItems.
        EventMsg::UserMessage(_)
        | EventMsg::AgentMessage(_)
        | EventMsg::AgentReasoning(_)
        | EventMsg::AgentReasoningRawContent(_)
        | EventMsg::EnteredReviewMode(_)
        | EventMsg::ExitedReviewMode(_)
        | EventMsg::PatchApplyEnd(_)
        | EventMsg::ContextCompacted(_)
        | EventMsg::McpToolCallEnd(_)
        | EventMsg::WebSearchEnd(_)
        | EventMsg::ImageGenerationEnd(_) => {
            matches!(history_mode, ThreadHistoryMode::Legacy).then_some(Cow::Borrowed(ev))
        }
        EventMsg::SubAgentActivity(event) => (matches!(history_mode, ThreadHistoryMode::Legacy)
            && event.kind != SubAgentActivityKind::Completed)
            .then_some(Cow::Borrowed(ev)),

        // Transient, non-durable events.
        EventMsg::Error(_)
        | EventMsg::ThreadQueueChanged(_)
        | EventMsg::GuardianAssessment(_)
        | EventMsg::ExecCommandEnd(_)
        | EventMsg::ViewImageToolCall(_)
        | EventMsg::CollabAgentSpawnEnd(_)
        | EventMsg::CollabAgentInteractionEnd(_)
        | EventMsg::CollabWaitingEnd(_)
        | EventMsg::CollabCloseEnd(_)
        | EventMsg::CollabResumeEnd(_)
        | EventMsg::DynamicToolCallRequest(_)
        | EventMsg::DynamicToolCallResponse(_)
        | EventMsg::Warning(_)
        | EventMsg::AuthRecoveryStarted(_)
        | EventMsg::AuthRecoveryCompleted(_)
        | EventMsg::GuardianWarning(_)
        | EventMsg::RealtimeConversationStarted(_)
        | EventMsg::RealtimeConversationSdp(_)
        | EventMsg::RealtimeConversationRealtime(_)
        | EventMsg::RealtimeConversationClosed(_)
        | EventMsg::SafetyBuffering(_)
        | EventMsg::ModelReroute(_)
        | EventMsg::ModelVerification(_)
        | EventMsg::TurnModerationMetadata(_)
        | EventMsg::AgentReasoningSectionBreak(_)
        | EventMsg::RawResponseItem(_)
        | EventMsg::RawResponseCompleted(_)
        | EventMsg::SessionConfigured(_)
        | EventMsg::EnvironmentConnected(_)
        | EventMsg::EnvironmentDisconnected(_)
        | EventMsg::McpToolCallBegin(_)
        | EventMsg::ExecCommandBegin(_)
        | EventMsg::TerminalInteraction(_)
        | EventMsg::ExecCommandOutputDelta(_)
        | EventMsg::ExecApprovalRequest(_)
        | EventMsg::RequestPermissions(_)
        | EventMsg::RequestUserInput(_)
        | EventMsg::ElicitationRequest(_)
        | EventMsg::ElicitationAbandoned(_)
        | EventMsg::ApplyPatchApprovalRequest(_)
        | EventMsg::StreamError(_)
        | EventMsg::PatchApplyBegin(_)
        | EventMsg::PatchApplyUpdated(_)
        | EventMsg::TurnDiff(_)
        | EventMsg::RealtimeConversationListVoicesResponse(_)
        | EventMsg::McpStartupUpdate(_)
        | EventMsg::McpStartupComplete(_)
        | EventMsg::WebSearchBegin(_)
        | EventMsg::PlanUpdate(_)
        | EventMsg::ShutdownComplete
        | EventMsg::DeprecationNotice(_)
        | EventMsg::ItemStarted(_)
        | EventMsg::HookStarted(_)
        | EventMsg::HookCompleted(_)
        | EventMsg::AgentMessageContentDelta(_)
        | EventMsg::PlanDelta(_)
        | EventMsg::ReasoningContentDelta(_)
        | EventMsg::ReasoningRawContentDelta(_)
        | EventMsg::ImageGenerationBegin(_)
        | EventMsg::CollabAgentSpawnBegin(_)
        | EventMsg::CollabAgentInteractionBegin(_)
        | EventMsg::CollabWaitingBegin(_)
        | EventMsg::CollabCloseBegin(_)
        | EventMsg::CollabResumeBegin(_) => None,
    }
}

/// Returns the persisted representation of an `ItemCompleted` event.
fn persisted_item_completed_event<'a>(
    ev: &'a EventMsg,
    event: &ItemCompletedEvent,
    history_mode: ThreadHistoryMode,
) -> Option<Cow<'a, EventMsg>> {
    match history_mode {
        ThreadHistoryMode::Legacy => {
            // Keep full text terminals in both modes: they are display-only
            // snapshots and also retain partial reasoning without forging model input.
            // Other legacy items retain their existing raw/flattened representation.
            (matches!(
                event.item,
                TurnItem::AgentMessage(_) | TurnItem::Reasoning(_)
            ) || matches!(
                event.item,
                TurnItem::FunctionCallOutput(_)
                    | TurnItem::Plan(_)
                    | TurnItem::Extension(ExtensionItem::Sleep(_))
            ) || matches!(
                &event.item,
                TurnItem::SubAgentActivity(item)
                    if item.kind == SubAgentActivityKind::Completed
            ))
            .then_some(Cow::Borrowed(ev))
        }
        ThreadHistoryMode::Paginated => match &event.item {
            TurnItem::CommandExecution(CommandExecutionItem {
                aggregated_output: Some(aggregated_output),
                ..
            }) if aggregated_output.len() > PERSISTED_COMMAND_OUTPUT_MAX_BYTES => {
                let aggregated_output = truncate_middle_with_marker(
                    aggregated_output,
                    PERSISTED_COMMAND_OUTPUT_MAX_BYTES,
                    PERSISTED_COMMAND_OUTPUT_TRUNCATION_MARKER,
                );
                let mut event = event.clone();
                if let TurnItem::CommandExecution(command) = &mut event.item {
                    command.aggregated_output = Some(aggregated_output);
                }
                Some(Cow::Owned(EventMsg::ItemCompleted(event)))
            }
            TurnItem::McpToolCall(McpToolCallItem {
                result: Some(result),
                ..
            }) => {
                let Cow::Owned(result) =
                    truncate_mcp_tool_result(result, PERSISTED_MCP_RESULT_MAX_BYTES)
                else {
                    return Some(Cow::Borrowed(ev));
                };
                let mut event = event.clone();
                if let TurnItem::McpToolCall(tool_call) = &mut event.item {
                    tool_call.result = Some(result);
                }
                Some(Cow::Owned(EventMsg::ItemCompleted(event)))
            }
            _ => Some(Cow::Borrowed(ev)),
        },
    }
}

#[cfg(test)]
mod generated_image_persistence_tests {
    use super::*;
    use codex_extension_items::image_generation::ImageGenerationItem;
    use codex_protocol::ThreadId;
    use codex_protocol::protocol::ImageGenerationEndEvent;
    use codex_protocol::protocol::ItemCompletedEvent;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;

    fn saved_path() -> AbsolutePathBuf {
        let path = std::env::temp_dir()
            .join("generated_images")
            .join("thread")
            .join("call-1.png");
        AbsolutePathBuf::from_absolute_path(path).expect("absolute temp path")
    }

    fn completed(saved_path: Option<AbsolutePathBuf>, result: &str) -> RolloutItem {
        RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".to_string(),
            item: TurnItem::Extension(ExtensionItem::ImageGeneration(ImageGenerationItem {
                id: "call-1".to_string(),
                status: "completed".to_string(),
                revised_prompt: Some("a cat".to_string()),
                image: GeneratedImageRef::Inline {
                    result: result.to_string(),
                },
                input_images: Vec::new(),
                transparent_background: None,
                failure: None,
                saved_path,
                imagegen_request_id: None,
                generation_id: None,
            })),
            started_at_ms: Some(1),
            completed_at_ms: 2,
        }))
    }

    fn image_of(item: &RolloutItem) -> &ImageGenerationItem {
        let RolloutItem::EventMsg(EventMsg::ItemCompleted(event)) = item else {
            panic!("item completed");
        };
        let TurnItem::Extension(ExtensionItem::ImageGeneration(image)) = &event.item else {
            panic!("image generation");
        };
        image
    }

    /// One owner machine (2026-10-22): 163 generated images made a 1.5 GB rollout, half of it
    /// the image items' base64 `result`. A saved image is persisted by path only;
    /// an unsaved one keeps its bytes (the only copy).
    #[test]
    fn saved_generated_images_are_persisted_by_path_only() {
        let big = "QUJD".repeat(1_000_000);
        let items = vec![completed(Some(saved_path()), &big), completed(None, &big)];
        let persisted = persisted_rollout_items(&items, ThreadHistoryMode::Paginated);
        assert_eq!(persisted.len(), 2);
        assert_eq!(
            image_of(&persisted[0]).image,
            GeneratedImageRef::Inline {
                result: String::new()
            }
        );
        assert_eq!(image_of(&persisted[0]).saved_path, Some(saved_path()));
        assert_eq!(
            image_of(&persisted[0]).revised_prompt.as_deref(),
            Some("a cat")
        );
        assert_eq!(
            &image_of(&persisted[1]).image,
            &GeneratedImageRef::Inline {
                result: big.clone()
            },
            "unsaved image keeps its bytes"
        );
        // The live (caller's) items are untouched: notifications still carry the image.
        assert_eq!(
            &image_of(&items[0]).image,
            &GeneratedImageRef::Inline {
                result: big.clone()
            }
        );
        let line = serde_json::to_string(&persisted[0]).expect("serialize");
        assert!(
            line.len() < 1_000,
            "persisted line is small: {} bytes",
            line.len()
        );
    }

    #[test]
    fn attachment_references_are_not_replaced_by_saved_path_stripping() {
        let mut item = completed(Some(saved_path()), "");
        let RolloutItem::EventMsg(EventMsg::ItemCompleted(event)) = &mut item else {
            panic!("completed image");
        };
        let TurnItem::Extension(ExtensionItem::ImageGeneration(image)) = &mut event.item else {
            panic!("generated image");
        };
        image.image = GeneratedImageRef::File {
            file_id: "opaque-attachment".into(),
        };
        let original = serde_json::to_value(&item).expect("serialize image");
        assert!(!strip_saved_generated_image(&mut item));
        assert_eq!(
            serde_json::to_value(&item).expect("serialize image"),
            original
        );
    }

    #[test]
    fn legacy_end_events_are_stripped_the_same_way_and_others_are_untouched() {
        let mut legacy =
            RolloutItem::EventMsg(EventMsg::ImageGenerationEnd(ImageGenerationEndEvent {
                call_id: "call-1".to_string(),
                status: "completed".to_string(),
                revised_prompt: None,
                image: GeneratedImageRef::Inline {
                    result: "QUJD".to_string(),
                },
                input_images: Vec::new(),
                transparent_background: None,
                failure: None,
                saved_path: Some(saved_path()),
            }));
        assert!(strip_saved_generated_image(&mut legacy));
        let RolloutItem::EventMsg(EventMsg::ImageGenerationEnd(event)) = &legacy else {
            panic!("legacy event");
        };
        assert_eq!(
            event.image,
            GeneratedImageRef::Inline {
                result: String::new()
            }
        );
        assert!(!strip_saved_generated_image(&mut legacy), "idempotent");
        let mut other = completed(None, "QUJD");
        assert!(!strip_saved_generated_image(&mut other));
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
