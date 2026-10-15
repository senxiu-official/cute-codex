//! Status line configuration view for customizing the TUI status bar.
//!
//! This module provides an interactive picker for selecting which items appear
//! in the status line at the bottom of the terminal. Users can:
//!
//! - **Select items**: Toggle which information is displayed
//! - **Reorder items**: Use left/right arrows to change display order
//! - **Preview changes**: See a live preview of the configured status line
//!
//! # Available Status Line Items
//!
//! - Model information (name, reasoning level)
//! - Directory paths (current dir, project root)
//! - Machine hostname
//! - Git information (branch name)
//! - Permissions profile
//! - Approval mode
//! - Context usage (remaining %, used %, window size)
//! - Usage limits (primary, secondary)
//! - Session info (thread title, thread ID, tokens used)
//! - Application version

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::collections::HashSet;
use strum::IntoEnumIterator;
use strum_macros::Display;
use strum_macros::EnumIter;
use strum_macros::EnumString;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::CancellationEvent;
use crate::bottom_pane::bottom_pane_view::BottomPaneView;
use crate::bottom_pane::multi_select_picker::MultiSelectItem;
use crate::bottom_pane::multi_select_picker::MultiSelectPicker;
use crate::bottom_pane::status_surface_preview::StatusSurfacePreviewData;
use crate::bottom_pane::status_surface_preview::StatusSurfacePreviewItem;
use crate::keymap::ListKeymap;
use crate::render::renderable::Renderable;

const STATUS_LINE_USE_THEME_COLORS_ITEM_ID: &str = "status-line-use-theme-colors";

/// Available items that can be displayed in the status line.
///
/// Each variant represents a piece of information that can be shown at the
/// bottom of the TUI. Items are serialized to kebab-case for configuration
/// storage (e.g., `ModelWithReasoning` becomes `model-with-reasoning`).
///
/// Some items are conditionally displayed based on availability:
/// - Git-related items only show when in a git repository
/// - Context/limit items only show when data is available from the API
/// - Thread ID only shows after a session has started
#[derive(EnumIter, EnumString, Display, Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
#[strum(serialize_all = "kebab_case")]
pub(crate) enum StatusLineItem {
    /// Inert reviewed launch label, never account identity.
    #[strum(to_string = "cutex_profile", serialize = "custom:profile")]
    CustomProfile,
    /// Inert reviewed static greeting.
    #[strum(
        to_string = "cutex_welcome",
        serialize = "custom:bon-voyage",
        serialize = "cutex_bon_voyage"
    )]
    CustomBonVoyage,
    /// Per-session Cutex external notification priority.
    #[strum(
        to_string = "cutex_notification",
        serialize = "notification",
        serialize = "custom:notification"
    )]
    Notification,
    /// The current model name.
    #[strum(to_string = "model", serialize = "model-name")]
    ModelName,

    /// Model name with reasoning level suffix.
    ModelWithReasoning,

    /// Current reasoning level.
    Reasoning,

    /// Current working directory path.
    CurrentDir,
    /// Working directory with display-only path aliases.
    #[strum(to_string = "cutex_workdir")]
    CutexWorkdir,
    #[strum(to_string = "cutex_agent_name")]
    CutexAgentName,
    #[strum(to_string = "cutex_context_remaining")]
    CutexContextRemaining,
    #[strum(to_string = "cutex_fable_limit_bar")]
    CutexFableLimitBar,
    #[strum(to_string = "cutex_simple_five_hour_limit")]
    CutexSimpleFiveHourLimit,
    #[strum(to_string = "cutex_aligned_profile")]
    CutexAlignedProfile,
    #[strum(to_string = "cutex_fable_limit")]
    CutexFableLimit,
    #[strum(to_string = "cutex_customized_limit")]
    CutexCustomizedLimit,
    #[strum(to_string = "cutex_customized_limit_bar")]
    CutexCustomizedLimitBar,

    /// Project root directory (if detected).
    #[strum(
        to_string = "project-name",
        serialize = "project",
        serialize = "project-root"
    )]
    ProjectRoot,

    /// Hostname of the machine running Codex.
    Hostname,

    /// Current git branch name (if in a repository).
    GitBranch,

    /// Open pull request number for the current branch.
    PullRequestNumber,

    /// Committed branch diff stats relative to the default branch.
    BranchChanges,

    /// Compact runtime run-state text.
    #[strum(to_string = "run-state", serialize = "status")]
    Status,

    /// Active permission profile or sandbox summary.
    Permissions,

    /// Active command approval mode.
    #[strum(to_string = "approval-mode", serialize = "approval")]
    ApprovalMode,

    /// Percentage of context window remaining.
    ContextRemaining,

    /// Percentage of context window used.
    ///
    /// Also accepts the legacy `context-usage` config value.
    #[strum(to_string = "context-used", serialize = "context-usage")]
    ContextUsed,

    /// Remaining usage on the primary rate limit.
    FiveHourLimit,

    /// Remaining usage on the secondary rate limit.
    WeeklyLimit,
    #[strum(to_string = "cutex_five_hour_limit")]
    CutexFiveHourLimit,
    #[strum(to_string = "cutex_weekly_limit")]
    CutexWeeklyLimit,
    #[strum(to_string = "cutex_simple_weekly_limit")]
    CutexSimpleWeeklyLimit,
    #[strum(to_string = "cutex_newline")]
    CutexNewline,
    #[strum(to_string = "cutex_welcome_indentation")]
    CutexWelcomeIndentation,
    #[strum(to_string = "cutex_space_2")]
    CutexSpace2,
    #[strum(to_string = "cutex_space_4")]
    CutexSpace4,
    #[strum(to_string = "cutex_space_6")]
    CutexSpace6,
    #[strum(to_string = "cutex_space_8")]
    CutexSpace8,
    #[strum(to_string = "cutex_space_10")]
    CutexSpace10,

    /// Codex application version.
    CodexVersion,

    /// Cutex release version, shown only when newer than the local registry.
    #[strum(to_string = "cutex_version")]
    CutexVersion,

    /// Total context window size in tokens.
    ContextWindowSize,

    /// Total tokens used in the current session.
    UsedTokens,

    /// Total input tokens consumed.
    TotalInputTokens,

    /// Total output tokens generated.
    TotalOutputTokens,

    /// Estimated credits attributed directly to the current enterprise thread.
    ThreadCredits,

    /// Estimated dollar cost attributed directly to the current enterprise thread.
    EstimatedThreadCost,

    /// Full thread UUID.
    #[strum(to_string = "thread-id", serialize = "session-id")]
    SessionId,

    /// Whether Fast mode is currently active.
    FastMode,

    /// Whether raw scrollback mode is currently active.
    RawOutput,

    /// Current thread name, omitted when unnamed.
    ThreadName,

    /// Current thread title, falling back to its identifier when unnamed.
    ThreadTitle,

    /// Current workspace notification headline.
    WorkspaceHeadline,

    /// Latest checklist task progress from `update_plan` (if available).
    TaskProgress,
}

impl StatusLineItem {
    /// User-visible description shown in the popup.
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::CutexAgentName => "Agent name and assigned project badge",
            Self::CutexContextRemaining => "Remaining context percentage",
            Self::CutexFableLimitBar => "Fable-only quota and reset countdown with progress bar",
            Self::CutexSimpleFiveHourLimit => "Time until reset and remaining 5h quota",
            Self::CutexAlignedProfile => "Profile centered in the Welcome display width",
            Self::CutexCustomizedLimit => "Profile endpoint remaining quota and reset countdown",
            Self::CutexCustomizedLimitBar => "Profile endpoint remaining quota with progress bar",
            Self::CutexFableLimit => "Fable-specific remaining weekly quota and reset countdown",
            Self::CutexWelcomeIndentation => {
                "Blank columns matching Welcome; animated greeting uses its reserved width"
            }
            Self::CutexSpace2
            | Self::CutexSpace4
            | Self::CutexSpace6
            | Self::CutexSpace8
            | Self::CutexSpace10 => "Fixed blank terminal columns; no separator dots",
            Self::CutexNewline => "Start a second status line; no blank rows",
            Self::CutexFiveHourLimit | Self::CutexWeeklyLimit => {
                "Remaining quota and time until reset; quarter-cell progress bar"
            }
            Self::CutexSimpleWeeklyLimit => {
                "Time until weekly reset and remaining quota; no progress bar"
            }
            Self::CustomProfile => "Reviewed launch display label (not account identity)",
            Self::CustomBonVoyage => "Reviewed static greeting",
            Self::Notification => "Cutex notification priority (Alt+N to cycle)",
            StatusLineItem::ModelName => "Current model name",
            StatusLineItem::ModelWithReasoning => "Current model name with reasoning level",
            StatusLineItem::Reasoning => "Current reasoning level",
            StatusLineItem::CurrentDir => "Current working directory",
            StatusLineItem::CutexWorkdir => "Working directory with configurable display aliases",
            StatusLineItem::ProjectRoot => "Project name (omitted when unavailable)",
            StatusLineItem::Hostname => "Current machine hostname (omitted when unavailable)",
            StatusLineItem::GitBranch => "Current Git branch (omitted when unavailable)",
            StatusLineItem::PullRequestNumber => {
                "Open pull request number for the current branch (omitted when unavailable)"
            }
            StatusLineItem::BranchChanges => {
                "Committed branch changes against the default branch (omitted when unavailable)"
            }
            StatusLineItem::Status => "Compact session run-state text (Ready, Working, Thinking)",
            StatusLineItem::Permissions => "Active permission profile or sandbox mode",
            StatusLineItem::ApprovalMode => "Active command approval mode",
            StatusLineItem::ContextRemaining => {
                "Percentage of context window remaining (omitted when unknown)"
            }
            StatusLineItem::ContextUsed => {
                "Percentage of context window used (omitted when unknown)"
            }
            StatusLineItem::FiveHourLimit => {
                "Remaining usage on the primary usage limit (omitted when unavailable)"
            }
            StatusLineItem::WeeklyLimit => {
                "Remaining usage on the secondary usage limit (omitted when unavailable)"
            }
            StatusLineItem::CodexVersion => "Codex application version",
            StatusLineItem::CutexVersion => {
                "Cute-codex release (hidden when current; newer release shown when known)"
            }
            StatusLineItem::ContextWindowSize => {
                "Total context window size in tokens (omitted when unknown)"
            }
            StatusLineItem::UsedTokens => "Total tokens used in session (omitted when zero)",
            StatusLineItem::TotalInputTokens => "Total input tokens used in session",
            StatusLineItem::TotalOutputTokens => "Total output tokens used in session",
            StatusLineItem::ThreadCredits => {
                "Estimated current-thread credits (Enterprise workspaces only; omitted when unavailable)"
            }
            StatusLineItem::EstimatedThreadCost => {
                "Estimated current-thread cost in USD (Enterprise workspaces only; omitted when unavailable)"
            }
            StatusLineItem::SessionId => "Current thread identifier (omitted until thread starts)",
            StatusLineItem::FastMode => "Whether Fast mode is currently active",
            StatusLineItem::RawOutput => "Whether raw scrollback mode is active",
            StatusLineItem::ThreadName => "Current thread name (omitted when unnamed)",
            StatusLineItem::ThreadTitle => {
                "Current thread title, or thread identifier when unnamed"
            }
            StatusLineItem::WorkspaceHeadline => {
                "Workspace notification headline (Enterprise workspaces only; omitted when unavailable)"
            }
            StatusLineItem::TaskProgress => {
                "Latest task progress from update_plan (omitted until available)"
            }
        }
    }

    pub(crate) fn preview_item(self) -> StatusSurfacePreviewItem {
        match self {
            Self::CutexWelcomeIndentation
            | Self::CutexSpace2
            | Self::CutexSpace4
            | Self::CutexSpace6
            | Self::CutexSpace8
            | Self::CutexSpace10 => StatusSurfacePreviewItem::CutexSpacing,
            Self::CutexNewline => StatusSurfacePreviewItem::CutexNewline,
            Self::CustomProfile => StatusSurfacePreviewItem::CustomProfile,
            Self::CustomBonVoyage => StatusSurfacePreviewItem::CustomBonVoyage,
            Self::Notification => StatusSurfacePreviewItem::Notification,
            StatusLineItem::ModelName => StatusSurfacePreviewItem::Model,
            StatusLineItem::ModelWithReasoning => StatusSurfacePreviewItem::ModelWithReasoning,
            StatusLineItem::Reasoning => StatusSurfacePreviewItem::Reasoning,
            StatusLineItem::CurrentDir => StatusSurfacePreviewItem::CurrentDir,
            StatusLineItem::CutexWorkdir => StatusSurfacePreviewItem::CutexWorkdir,
            StatusLineItem::CutexAgentName => StatusSurfacePreviewItem::CutexAgentName,
            StatusLineItem::CutexContextRemaining => {
                StatusSurfacePreviewItem::CutexContextRemaining
            }
            StatusLineItem::CutexFableLimitBar => StatusSurfacePreviewItem::CutexFableLimitBar,
            StatusLineItem::CutexSimpleFiveHourLimit => {
                StatusSurfacePreviewItem::CutexSimpleFiveHourLimit
            }
            StatusLineItem::CutexAlignedProfile => StatusSurfacePreviewItem::CutexAlignedProfile,
            StatusLineItem::CutexFableLimit => StatusSurfacePreviewItem::CutexFableLimit,
            StatusLineItem::CutexCustomizedLimit => StatusSurfacePreviewItem::CutexCustomizedLimit,
            StatusLineItem::CutexCustomizedLimitBar => {
                StatusSurfacePreviewItem::CutexCustomizedLimitBar
            }
            StatusLineItem::ProjectRoot => StatusSurfacePreviewItem::ProjectRoot,
            StatusLineItem::Hostname => StatusSurfacePreviewItem::Hostname,
            StatusLineItem::GitBranch => StatusSurfacePreviewItem::GitBranch,
            StatusLineItem::PullRequestNumber => StatusSurfacePreviewItem::PullRequestNumber,
            StatusLineItem::BranchChanges => StatusSurfacePreviewItem::BranchChanges,
            StatusLineItem::Status => StatusSurfacePreviewItem::Status,
            StatusLineItem::Permissions => StatusSurfacePreviewItem::Permissions,
            StatusLineItem::ApprovalMode => StatusSurfacePreviewItem::ApprovalMode,
            StatusLineItem::ContextRemaining => StatusSurfacePreviewItem::ContextRemaining,
            StatusLineItem::ContextUsed => StatusSurfacePreviewItem::ContextUsed,
            StatusLineItem::FiveHourLimit => StatusSurfacePreviewItem::FiveHourLimit,
            StatusLineItem::WeeklyLimit => StatusSurfacePreviewItem::WeeklyLimit,
            StatusLineItem::CutexFiveHourLimit => StatusSurfacePreviewItem::CutexFiveHourLimit,
            StatusLineItem::CutexWeeklyLimit => StatusSurfacePreviewItem::CutexWeeklyLimit,
            StatusLineItem::CutexSimpleWeeklyLimit => {
                StatusSurfacePreviewItem::CutexSimpleWeeklyLimit
            }
            StatusLineItem::CodexVersion => StatusSurfacePreviewItem::CodexVersion,
            StatusLineItem::CutexVersion => StatusSurfacePreviewItem::CutexVersion,
            StatusLineItem::ContextWindowSize => StatusSurfacePreviewItem::ContextWindowSize,
            StatusLineItem::UsedTokens => StatusSurfacePreviewItem::UsedTokens,
            StatusLineItem::TotalInputTokens => StatusSurfacePreviewItem::TotalInputTokens,
            StatusLineItem::TotalOutputTokens => StatusSurfacePreviewItem::TotalOutputTokens,
            StatusLineItem::ThreadCredits => StatusSurfacePreviewItem::ThreadCredits,
            StatusLineItem::EstimatedThreadCost => StatusSurfacePreviewItem::EstimatedThreadCost,
            StatusLineItem::SessionId => StatusSurfacePreviewItem::SessionId,
            StatusLineItem::FastMode => StatusSurfacePreviewItem::FastMode,
            StatusLineItem::RawOutput => StatusSurfacePreviewItem::RawOutput,
            StatusLineItem::ThreadName => StatusSurfacePreviewItem::ThreadName,
            StatusLineItem::ThreadTitle => StatusSurfacePreviewItem::ThreadTitle,
            StatusLineItem::WorkspaceHeadline => StatusSurfacePreviewItem::WorkspaceHeadline,
            StatusLineItem::TaskProgress => StatusSurfacePreviewItem::TaskProgress,
        }
    }
}

/// Interactive view for configuring which items appear in the status line.
///
/// Wraps a [`MultiSelectPicker`] with status-line-specific behavior:
/// - Pre-populates items from current configuration
/// - Shows a live preview of the configured status line
/// - Emits [`AppEvent::StatusLineSetup`] on confirmation
/// - Emits [`AppEvent::StatusLineSetupCancelled`] on cancellation
pub(crate) struct StatusLineSetupView {
    /// The underlying multi-select picker widget.
    picker: MultiSelectPicker,
}

impl StatusLineSetupView {
    /// Creates a new status line setup view.
    ///
    /// # Arguments
    ///
    /// * `status_line_items` - Currently configured item IDs (in display order),
    ///   or `None` to start with all items disabled
    /// * `use_theme_colors` - Whether the preview and saved status line use colors from
    ///   the active theme
    /// * `app_event_tx` - Event sender for dispatching configuration changes
    ///
    /// Items from `status_line_items` are shown first (in order) and marked as
    /// enabled. Remaining items are appended and marked as disabled.
    pub(crate) fn new(
        status_line_items: Option<&[String]>,
        use_theme_colors: bool,
        preview_data: StatusSurfacePreviewData,
        app_event_tx: AppEventSender,
        list_keymap: ListKeymap,
    ) -> Self {
        let mut used_ids = HashSet::new();
        let mut items = vec![MultiSelectItem {
            id: STATUS_LINE_USE_THEME_COLORS_ITEM_ID.to_string(),
            name: "Use theme colors".to_string(),
            description: Some("Apply colors from the active /theme".to_string()),
            enabled: use_theme_colors,
            orderable: false,
            section_break_after: true,
        }];

        if let Some(selected_items) = status_line_items.as_ref() {
            for id in *selected_items {
                let Ok(item) = id.parse::<StatusLineItem>() else {
                    continue;
                };
                let item_id = item.to_string();
                if !used_ids.insert(item_id.clone()) {
                    continue;
                }
                items.push(Self::status_line_select_item(
                    item,
                    /*enabled*/ true,
                    &preview_data,
                ));
            }
        }

        let mut remaining: Vec<_> = StatusLineItem::iter().collect();
        remaining.sort_by_key(|item| !item.to_string().starts_with("cutex_"));
        for item in remaining {
            let item_id = item.to_string();
            if used_ids.contains(&item_id)
                || (crate::custom_status_items::is_custom(item)
                    && crate::custom_status_items::style(item).is_none())
            {
                continue;
            }
            items.push(Self::status_line_select_item(
                item,
                /*enabled*/ false,
                &preview_data,
            ));
        }

        Self {
            picker: MultiSelectPicker::builder(
                "Configure Status Line".to_string(),
                Some("Select which items to display in the status line.".to_string()),
                app_event_tx,
            )
            .list_keymap(list_keymap)
            .items(items)
            .enable_ordering()
            .on_preview_lines(move |items| {
                let use_theme_colors = items
                    .iter()
                    .find(|item| item.id == STATUS_LINE_USE_THEME_COLORS_ITEM_ID)
                    .map(|item| item.enabled)
                    .unwrap_or(true);
                preview_data.status_lines_for_items(
                    items
                        .iter()
                        .filter(|item| item.enabled)
                        .filter_map(|item| item.id.parse::<StatusLineItem>().ok()),
                    use_theme_colors,
                )
            })
            .on_confirm(|ids, app_event| {
                let use_theme_colors = ids
                    .iter()
                    .any(|id| id == STATUS_LINE_USE_THEME_COLORS_ITEM_ID);
                let items = ids
                    .iter()
                    .filter_map(|id| id.parse::<StatusLineItem>().ok())
                    .collect::<Vec<_>>();
                app_event.send(AppEvent::StatusLineSetup {
                    items,
                    use_theme_colors,
                });
            })
            .on_cancel(|app_event| {
                app_event.send(AppEvent::StatusLineSetupCancelled);
            })
            .build(),
        }
    }

    /// Converts a [`StatusLineItem`] into a [`MultiSelectItem`] for the picker.
    fn status_line_select_item(
        item: StatusLineItem,
        enabled: bool,
        preview_data: &StatusSurfacePreviewData,
    ) -> MultiSelectItem {
        let default_name = item.to_string();
        let default_description = item.description();
        let (name, description) = match item {
            StatusLineItem::CutexCustomizedLimit => {
                ("C+ Customized Limit".into(), default_description.into())
            }
            StatusLineItem::CutexCustomizedLimitBar => {
                ("C+ Customized Limit Bar".into(), default_description.into())
            }
            StatusLineItem::CutexAgentName => ("C+ Agent Name".into(), default_description.into()),
            StatusLineItem::CutexContextRemaining => {
                ("C+ Context Remaining".into(), default_description.into())
            }
            StatusLineItem::CutexFableLimitBar => {
                ("C+ Fable Limit Bar".into(), default_description.into())
            }
            StatusLineItem::CutexSimpleFiveHourLimit => {
                ("C+ 5h Limit".into(), default_description.into())
            }
            StatusLineItem::CutexAlignedProfile => {
                ("C+ Profile Aligned".into(), default_description.into())
            }
            StatusLineItem::CutexFableLimit => {
                ("C+ Fable Limit".into(), default_description.into())
            }
            StatusLineItem::CutexWorkdir => {
                ("C+ Working Directory".into(), default_description.into())
            }
            StatusLineItem::CutexVersion => ("C+ Cutex Version".into(), default_description.into()),
            StatusLineItem::CutexFiveHourLimit => {
                ("C+ 5h Limit Bar".into(), default_description.into())
            }
            StatusLineItem::CutexWeeklyLimit => {
                ("C+ Weekly Limit Bar".into(), default_description.into())
            }
            StatusLineItem::CutexSimpleWeeklyLimit => {
                ("C+ Weekly Limit".into(), default_description.into())
            }
            StatusLineItem::CutexWelcomeIndentation => {
                ("C+ Welcome Indentation".into(), default_description.into())
            }
            StatusLineItem::CutexSpace2 => ("C+ 2 Spaces".into(), default_description.into()),
            StatusLineItem::CutexSpace4 => ("C+ 4 Spaces".into(), default_description.into()),
            StatusLineItem::CutexSpace6 => ("C+ 6 Spaces".into(), default_description.into()),
            StatusLineItem::CutexSpace8 => ("C+ 8 Spaces".into(), default_description.into()),
            StatusLineItem::CutexSpace10 => ("C+ 10 Spaces".into(), default_description.into()),
            StatusLineItem::CutexNewline => ("C+ Newline".into(), default_description.into()),
            StatusLineItem::CustomBonVoyage => ("C+ Welcome".into(), default_description.into()),
            StatusLineItem::CustomProfile => ("C+ Profile".into(), default_description.into()),
            StatusLineItem::Notification => ("C+ Notification".into(), default_description.into()),
            StatusLineItem::FiveHourLimit | StatusLineItem::WeeklyLimit => (
                preview_data.rate_limit_item_name(item.preview_item(), &default_name),
                preview_data.rate_limit_item_description(item.preview_item(), default_description),
            ),
            _ => (default_name, default_description.to_string()),
        };

        MultiSelectItem {
            id: item.to_string(),
            name,
            description: Some(description),
            enabled,
            orderable: true,
            section_break_after: false,
        }
    }
}

impl BottomPaneView for StatusLineSetupView {
    fn keymap_contexts(&self) -> crate::keymap::KeymapContextSet {
        crate::keymap::KeymapContextSet::new(crate::keymap::KeymapContext::List)
    }

    fn handle_key_event(&mut self, key_event: crossterm::event::KeyEvent) {
        self.picker.handle_key_event(key_event);
    }

    fn is_complete(&self) -> bool {
        self.picker.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.picker.close();
        CancellationEvent::Handled
    }
}

impl Renderable for StatusLineSetupView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.picker.render(area, buf)
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.picker.desired_height(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_event_sender::AppEventSender;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::text::Line;
    use tokio::sync::mpsc::unbounded_channel;

    use crate::app_event::AppEvent;

    #[test]
    fn context_used_accepts_context_usage_legacy_id() {
        assert_eq!(StatusLineItem::ContextUsed.to_string(), "context-used");
        assert_eq!(
            "context-used".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ContextUsed)
        );
        assert_eq!(
            "context-usage".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ContextUsed)
        );
    }

    #[test]
    fn context_remaining_is_selectable_id() {
        assert_eq!(
            "context-remaining".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ContextRemaining)
        );
        assert_eq!(
            StatusLineItem::ContextRemaining.to_string(),
            "context-remaining"
        );
    }

    #[test]
    fn thread_usage_items_are_independently_selectable() {
        assert_eq!(
            "thread-credits".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ThreadCredits)
        );
        assert_eq!(
            "estimated-thread-cost".parse::<StatusLineItem>(),
            Ok(StatusLineItem::EstimatedThreadCost)
        );
    }

    #[test]
    fn project_name_is_canonical_and_accepts_legacy_ids() {
        assert_eq!(StatusLineItem::ProjectRoot.to_string(), "project-name");
        assert_eq!(
            "project-name".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ProjectRoot)
        );
        assert_eq!(
            "project".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ProjectRoot)
        );
        assert_eq!(
            "project-root".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ProjectRoot)
        );
    }

    #[test]
    fn model_is_canonical_and_accepts_model_name_legacy_id() {
        assert_eq!(StatusLineItem::ModelName.to_string(), "model");
        assert_eq!(
            "model".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ModelName)
        );
        assert_eq!(
            "model-name".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ModelName)
        );
    }

    #[test]
    fn reasoning_is_selectable_id() {
        assert_eq!(StatusLineItem::Reasoning.to_string(), "reasoning");
        assert_eq!(
            "reasoning".parse::<StatusLineItem>(),
            Ok(StatusLineItem::Reasoning)
        );
    }

    #[test]
    fn run_state_is_canonical_and_accepts_status_legacy_id() {
        assert_eq!(StatusLineItem::Status.to_string(), "run-state");
        assert_eq!(
            "run-state".parse::<StatusLineItem>(),
            Ok(StatusLineItem::Status)
        );
        assert_eq!(
            "status".parse::<StatusLineItem>(),
            Ok(StatusLineItem::Status)
        );
    }

    #[test]
    fn git_summary_items_are_selectable_ids() {
        assert_eq!(
            "pull-request-number".parse::<StatusLineItem>(),
            Ok(StatusLineItem::PullRequestNumber)
        );
        assert_eq!(
            "branch-changes".parse::<StatusLineItem>(),
            Ok(StatusLineItem::BranchChanges)
        );
    }

    #[test]
    fn parse_status_line_items_accepts_title_only_variants() {
        let items = ["run-state", "task-progress"]
            .into_iter()
            .map(str::parse::<StatusLineItem>)
            .collect::<Result<Vec<_>, _>>();
        assert_eq!(
            items,
            Ok(vec![StatusLineItem::Status, StatusLineItem::TaskProgress,])
        );
    }

    #[test]
    fn preview_uses_runtime_values() {
        let preview_data = StatusSurfacePreviewData::from_iter([
            (
                StatusLineItem::ModelName.preview_item(),
                "gpt-5".to_string(),
            ),
            (
                StatusLineItem::CurrentDir.preview_item(),
                "/repo".to_string(),
            ),
        ]);
        let items = [
            MultiSelectItem {
                id: StatusLineItem::ModelName.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
            MultiSelectItem {
                id: StatusLineItem::CurrentDir.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
        ];

        assert_eq!(
            line_text(
                preview_data.status_line_for_items(
                    items
                        .iter()
                        .filter_map(|item| item.id.parse::<StatusLineItem>().ok()),
                    /*use_theme_colors*/ true,
                )
            ),
            Some("gpt-5 · /repo".to_string())
        );
    }

    #[test]
    fn preview_uses_placeholders_when_runtime_values_are_missing() {
        let preview_data = StatusSurfacePreviewData::from_iter([(
            StatusSurfacePreviewItem::Model,
            "gpt-5".to_string(),
        )]);
        let items = [
            MultiSelectItem {
                id: StatusLineItem::ModelName.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
            MultiSelectItem {
                id: StatusLineItem::GitBranch.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
        ];

        assert_eq!(
            line_text(
                preview_data.status_line_for_items(
                    items
                        .iter()
                        .filter_map(|item| item.id.parse::<StatusLineItem>().ok()),
                    /*use_theme_colors*/ true,
                )
            ),
            Some("gpt-5 · feat/awesome-feature".to_string())
        );
    }

    #[test]
    fn preview_includes_thread_title() {
        let preview_data = StatusSurfacePreviewData::from_iter([
            (
                StatusLineItem::ModelName.preview_item(),
                "gpt-5".to_string(),
            ),
            (
                StatusLineItem::ThreadTitle.preview_item(),
                "Roadmap cleanup".to_string(),
            ),
        ]);
        let items = [
            MultiSelectItem {
                id: StatusLineItem::ModelName.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
            MultiSelectItem {
                id: StatusLineItem::ThreadTitle.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
        ];

        assert_eq!(
            line_text(
                preview_data.status_line_for_items(
                    items
                        .iter()
                        .filter_map(|item| item.id.parse::<StatusLineItem>().ok()),
                    /*use_theme_colors*/ true,
                )
            ),
            Some("gpt-5 · Roadmap cleanup".to_string())
        );
    }

    #[test]
    fn setup_view_snapshot_uses_runtime_preview_values() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let view = StatusLineSetupView::new(
            Some(&[
                StatusLineItem::ModelName.to_string(),
                StatusLineItem::CurrentDir.to_string(),
                StatusLineItem::GitBranch.to_string(),
            ]),
            /*use_theme_colors*/ true,
            StatusSurfacePreviewData::from_iter([
                (
                    StatusLineItem::ModelName.preview_item(),
                    "gpt-5-codex".to_string(),
                ),
                (
                    StatusLineItem::CurrentDir.preview_item(),
                    "~/codex-rs".to_string(),
                ),
                (
                    StatusLineItem::GitBranch.preview_item(),
                    "jif/statusline-preview".to_string(),
                ),
                (
                    StatusLineItem::WeeklyLimit.preview_item(),
                    "weekly 82% left".to_string(),
                ),
            ]),
            AppEventSender::new(tx_raw),
            crate::keymap::RuntimeKeymap::defaults().list,
        );

        assert_snapshot!(render_lines(&view, /*width*/ 72));
    }

    #[test]
    fn setup_view_snapshot_includes_thread_usage_items() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let view = StatusLineSetupView::new(
            Some(&[
                StatusLineItem::ThreadCredits.to_string(),
                StatusLineItem::EstimatedThreadCost.to_string(),
            ]),
            /*use_theme_colors*/ true,
            StatusSurfacePreviewData::from_iter([
                (
                    StatusLineItem::ThreadCredits.preview_item(),
                    "5.2 credits".to_string(),
                ),
                (
                    StatusLineItem::EstimatedThreadCost.preview_item(),
                    "~$1.82".to_string(),
                ),
            ]),
            AppEventSender::new(tx_raw),
            crate::keymap::RuntimeKeymap::defaults().list,
        );

        assert_snapshot!(render_lines(&view, /*width*/ 100));
    }

    #[test]
    fn setup_view_snapshot_includes_hostname() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let view = StatusLineSetupView::new(
            Some(&[
                StatusLineItem::Hostname.to_string(),
                StatusLineItem::CurrentDir.to_string(),
            ]),
            /*use_theme_colors*/ true,
            StatusSurfacePreviewData::from_iter([
                (
                    StatusLineItem::Hostname.preview_item(),
                    "ssh-build-01.example.com".to_string(),
                ),
                (
                    StatusLineItem::CurrentDir.preview_item(),
                    "~/codex-rs".to_string(),
                ),
            ]),
            AppEventSender::new(tx_raw),
            crate::keymap::RuntimeKeymap::defaults().list,
        );

        assert_snapshot!(
            render_lines(&view, /*width*/ 100)
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    fn render_lines(view: &StatusLineSetupView, width: u16) -> String {
        let height = view.desired_height(width);
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        (0..area.height)
            .map(|row| {
                let mut line = String::new();
                for col in 0..area.width {
                    let symbol = buf[(area.x + col, area.y + row)].symbol();
                    if symbol.is_empty() {
                        line.push(' ');
                    } else {
                        line.push_str(symbol);
                    }
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn line_text(line: Option<Line<'static>>) -> Option<String> {
        line.map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
    }
}
