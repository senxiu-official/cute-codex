//! Display-only live and replay consumer. Never submits an operation or model item.
use super::*;
use codex_app_server_protocol::ThreadTimelineEntry;
use codex_protocol::presentation::PresentationAppended;

impl ChatWidget {
    pub(super) fn group_pending_presentation(
        &mut self,
        cell: Box<dyn HistoryCell>,
    ) -> Box<dyn HistoryCell> {
        if self
            .transcript
            .pending_presentation_group
            .as_ref()
            .is_some_and(|group| group.matches(cell.as_ref()))
        {
            if let Some(mcp) = cell.as_any().downcast_ref::<McpToolCallCell>() {
                self.transcript
                    .grouped_mcp_seen
                    .insert(mcp.call_id().into(), mcp.has_result());
            }
            if let Some(group) = self.transcript.pending_presentation_group.take() {
                return match group.combine(cell) {
                    Ok(cell) => cell,
                    Err((cell, error)) => {
                        self.add_error_message(format!("Invalid display group: {error}"));
                        cell
                    }
                };
            }
        }
        cell
    }

    pub(super) fn on_presentation(&mut self, record: PresentationAppended) {
        if let Err(error) = record.validate() {
            self.add_error_message(format!("Invalid durable notice: {error}"));
            return;
        }
        let identity = (
            record.origin_thread_id.clone(),
            record.presentation.id.clone(),
        );
        if let Some(previous) = self.transcript.presentations_seen.get(&identity) {
            if previous != &record {
                self.add_error_message(
                    "Conflicting durable notice identity; original notice retained.".into(),
                );
            }
            return;
        }
        let group = history_cell::PendingPresentationGroup {
            record: record.clone(),
            counterpart_first: true,
        };
        if self
            .thread_id
            .is_some_and(|id| id.to_string() == record.origin_thread_id)
            && self.transcript.pending_presentation_group.is_none()
            && self
                .transcript
                .active_cell
                .as_ref()
                .is_some_and(|cell| group.matches(cell.as_ref()))
        {
            self.transcript.presentations_seen.insert(identity, record);
            self.transcript.pending_presentation_group = Some(group);
            self.bump_active_cell_revision();
            return;
        }
        match history_cell::PresentationHistoryCell::new(record.clone()) {
            Ok(cell) => {
                self.transcript.presentations_seen.insert(identity, record);
                self.add_to_history(cell);
            }
            Err(error) => self.add_error_message(format!("Invalid durable notice: {error}")),
        }
    }

    pub(super) fn replay_presentation_timeline(
        &mut self,
        turns: Vec<Turn>,
        timeline: Vec<ThreadTimelineEntry>,
        replay_kind: ReplayKind,
    ) {
        let hidden: std::collections::HashSet<String> = turns
            .windows(2)
            .filter(|pair| crate::app_backtrack::is_hidden_nested_review_turn(&pair[0], &pair[1]))
            .map(|pair| pair[1].id.clone())
            .collect();
        let mut completed_times = turns
            .iter()
            .map(|turn| (turn.id.clone(), turn.completed_at))
            .collect::<std::collections::HashMap<_, _>>();
        // A timeline page can carry completion timestamps even without Turn snapshots.
        for entry in &timeline {
            if let ThreadTimelineEntry::TurnCompleted {
                turn_id,
                completed_at: Some(time),
                ..
            } = entry
            {
                completed_times.insert(turn_id.clone(), Some(*time));
            }
        }
        let mut timeline = timeline.into_iter().peekable();
        while let Some(entry) = timeline.next() {
            let pair = match (&entry, timeline.peek()) {
                (
                    ThreadTimelineEntry::Item { item, .. },
                    Some(ThreadTimelineEntry::Presentation { item: record, .. }),
                ) => Some((item.as_ref(), record, true)),
                (
                    ThreadTimelineEntry::Presentation { item: record, .. },
                    Some(ThreadTimelineEntry::Item { item, .. }),
                ) => Some((item.as_ref(), record, false)),
                _ => None,
            };
            if let Some((item, record, counterpart_first)) = pair
                && record.validate().is_ok()
                && self
                    .thread_id
                    .is_some_and(|id| id.to_string() == record.origin_thread_id)
                && !self.transcript.presentations_seen.contains_key(&(
                    record.origin_thread_id.clone(),
                    record.presentation.id.clone(),
                ))
                && record.presentation.references.iter().any(|reference| {
                    match (reference.kind.clone(), item) {
                        (
                            codex_protocol::presentation::PresentationReferenceKind::ExternalInput,
                            ThreadItem::FunctionCallOutput {
                                id,
                                name,
                                namespace,
                                output,
                                external_input_view,
                                ..
                            },
                        ) => {
                            reference.id == *id
                                && history_cell::ExternalInputHistoryCell::parse(
                                    id,
                                    name,
                                    namespace.as_deref(),
                                    output,
                                    external_input_view.as_ref(),
                                )
                                .is_some()
                        }
                        (
                            codex_protocol::presentation::PresentationReferenceKind::McpInvocation,
                            ThreadItem::McpToolCall {
                                id,
                                server,
                                tool,
                                arguments,
                                ..
                            },
                        ) => {
                            reference.id == *id
                                && McpInvocation {
                                    server: server.clone(),
                                    tool: tool.clone(),
                                    arguments: Some(arguments.clone()),
                                }
                                .supports_compact_presentation()
                        }
                        _ => false,
                    }
                })
            {
                let record = record.clone();
                let Some(next) = timeline.next() else {
                    self.on_presentation(record);
                    break;
                };
                let item_entry = if counterpart_first { entry } else { next };
                if let ThreadTimelineEntry::Item { turn_id, item, .. } = item_entry {
                    self.transcript.pending_presentation_group =
                        Some(history_cell::PendingPresentationGroup {
                            record: record.clone(),
                            counterpart_first,
                        });
                    self.replay_thread_item(*item, turn_id, replay_kind);
                    if self.transcript.pending_presentation_group.take().is_some() {
                        // A duplicate/non-rendered counterpart cannot justify hiding this fact.
                        self.on_presentation(record);
                    } else {
                        self.transcript.presentations_seen.insert(
                            (
                                record.origin_thread_id.clone(),
                                record.presentation.id.clone(),
                            ),
                            record,
                        );
                    }
                }
                continue;
            }

            match entry {
                ThreadTimelineEntry::Presentation { item, .. } => self.on_presentation(item),
                ThreadTimelineEntry::Item { turn_id, item, .. } => {
                    if hidden.contains(&turn_id)
                        && matches!(item.as_ref(), ThreadItem::UserMessage { .. })
                    {
                        continue;
                    }
                    self.replay_thread_item(*item, turn_id, replay_kind);
                }
                ThreadTimelineEntry::TurnStarted { turn_id, .. } => {
                    if turns
                        .iter()
                        .any(|t| t.id == turn_id && t.status == TurnStatus::InProgress)
                    {
                        self.turn_lifecycle.last_turn_id = Some(turn_id);
                        self.last_non_retry_error = None;
                        self.on_task_started();
                    }
                }
                ThreadTimelineEntry::TurnCompleted {
                    turn_id,
                    status,
                    error,
                    started_at,
                    completed_at,
                    duration_ms,
                    ..
                } => {
                    self.handle_turn_completed_notification(
                        TurnCompletedNotification {
                            thread_id: self.thread_id.map(|id| id.to_string()).unwrap_or_default(),
                            turn: Turn {
                                id: turn_id.clone(),
                                items_view: codex_app_server_protocol::TurnItemsView::NotLoaded,
                                items: Vec::new(),
                                status: if hidden.contains(&turn_id) {
                                    TurnStatus::Completed
                                } else {
                                    status
                                },
                                error,
                                started_at,
                                completed_at,
                                duration_ms,
                                root_turn_id: turns
                                    .iter()
                                    .find(|turn| turn.id == turn_id)
                                    .and_then(|turn| turn.root_turn_id.clone()),
                            },
                        },
                        Some(replay_kind),
                    );
                }
                // Existing ordinary turn replay did not project realtime entries.
                ThreadTimelineEntry::Realtime { .. } => {}
            }
        }
    }
}
