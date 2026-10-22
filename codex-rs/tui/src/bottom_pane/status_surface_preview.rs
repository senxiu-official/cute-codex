use std::collections::BTreeMap;

use codex_protocol::ThreadId;
use ratatui::text::Line;

use super::status_line_setup::StatusLineItem;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum StatusSurfacePreviewItem {
    CustomProfile,
    CustomBonVoyage,
    Notification,
    AppName,
    ProjectName,
    ProjectRoot,
    CurrentDir,
    CutexWorkdir,
    CutexAgentName,
    CutexContextRemaining,
    CutexFableLimitBar,
    CutexSimpleFiveHourLimit,
    CutexAlignedProfile,
    CutexFableLimit,
    CutexCustomizedLimit,
    CutexCustomizedLimitBar,
    Hostname,
    Status,
    ThreadName,
    ThreadTitle,
    GitBranch,
    PullRequestNumber,
    BranchChanges,
    Permissions,
    ApprovalMode,
    ContextRemaining,
    ContextUsed,
    FiveHourLimit,
    WeeklyLimit,
    CutexFiveHourLimit,
    CutexWeeklyLimit,
    CutexSimpleWeeklyLimit,
    CutexNewline,
    CutexSpacing,
    CodexVersion,
    CutexVersion,
    ContextWindowSize,
    UsedTokens,
    TotalInputTokens,
    TotalOutputTokens,
    ThreadCredits,
    EstimatedThreadCost,
    SessionId,
    FastMode,
    Daybreak,
    RawOutput,
    WorkspaceHeadline,
    Model,
    ModelWithReasoning,
    Reasoning,
    TaskProgress,
}

impl StatusSurfacePreviewItem {
    fn placeholder(self) -> &'static str {
        match self {
            Self::CustomProfile => "[cutex_profile unavailable]",
            Self::CustomBonVoyage => "[cutex_welcome unavailable]",
            Self::Notification => "OFF",
            Self::CutexNewline | Self::CutexSpacing => "",
            Self::CutexSimpleWeeklyLimit => "W — —%",
            Self::CutexFiveHourLimit => "5[          ]—% —",
            Self::CutexWeeklyLimit => "W[          ]—% —",
            StatusSurfacePreviewItem::AppName => "codex",
            StatusSurfacePreviewItem::ProjectName => "my-project",
            StatusSurfacePreviewItem::ProjectRoot => "my-project",
            StatusSurfacePreviewItem::CurrentDir => "~/my-project/subdir",
            Self::CutexSimpleFiveHourLimit => "5 3h20m 98%",
            Self::CutexFableLimitBar => "F[███████   ]70% 1d03h",
            Self::CutexContextRemaining => "C 72%",
            Self::CutexAgentName => "IF director",
            Self::CutexWorkdir => "$DEV/my-project",
            Self::CutexAlignedProfile => "  profile   ",
            Self::CutexCustomizedLimit => "Q 1d03h 75%",
            Self::CutexCustomizedLimitBar => "Q[███████▌  ]75% 1d03h",
            Self::CutexFableLimit => "F 1d03h 16%",
            StatusSurfacePreviewItem::Hostname => "my-host",
            StatusSurfacePreviewItem::Status => "Working",
            StatusSurfacePreviewItem::ThreadName => "thread name",
            StatusSurfacePreviewItem::ThreadTitle => "thread title",
            StatusSurfacePreviewItem::GitBranch => "feat/awesome-feature",
            StatusSurfacePreviewItem::PullRequestNumber => "PR #123",
            StatusSurfacePreviewItem::BranchChanges => "+12 -3",
            StatusSurfacePreviewItem::Permissions => "Workspace",
            StatusSurfacePreviewItem::ApprovalMode => "on-request",
            StatusSurfacePreviewItem::ContextRemaining => "Context 0% left",
            StatusSurfacePreviewItem::ContextUsed => "Context 0% used",
            StatusSurfacePreviewItem::FiveHourLimit => "5h N/A",
            StatusSurfacePreviewItem::WeeklyLimit => "weekly N/A",
            StatusSurfacePreviewItem::CodexVersion => "0.0.0",
            StatusSurfacePreviewItem::CutexVersion => "0.0.0 (N: r1)",
            StatusSurfacePreviewItem::ContextWindowSize => "0 window",
            StatusSurfacePreviewItem::UsedTokens => "0 used",
            StatusSurfacePreviewItem::TotalInputTokens => "0 in",
            StatusSurfacePreviewItem::TotalOutputTokens => "0 out",
            StatusSurfacePreviewItem::ThreadCredits => "5.2 credits",
            StatusSurfacePreviewItem::EstimatedThreadCost => "~$1.82",
            StatusSurfacePreviewItem::SessionId => "550e8400-e29b-41d4",
            StatusSurfacePreviewItem::FastMode => "Fast on",
            StatusSurfacePreviewItem::Daybreak => "Daybreak off",
            StatusSurfacePreviewItem::RawOutput => "raw output",
            StatusSurfacePreviewItem::WorkspaceHeadline => "Workspace headline",
            StatusSurfacePreviewItem::Model => "gpt-5.2-codex",
            StatusSurfacePreviewItem::ModelWithReasoning => "gpt-5.2-codex medium",
            StatusSurfacePreviewItem::Reasoning => "medium",
            StatusSurfacePreviewItem::TaskProgress => "Tasks 0/0",
        }
    }

    pub(crate) fn iter() -> impl Iterator<Item = Self> {
        [
            Self::CustomProfile,
            Self::CustomBonVoyage,
            Self::Notification,
            Self::AppName,
            Self::ProjectName,
            Self::ProjectRoot,
            Self::CurrentDir,
            Self::CutexWorkdir,
            Self::CutexAgentName,
            Self::CutexContextRemaining,
            Self::CutexFableLimitBar,
            Self::CutexSimpleFiveHourLimit,
            Self::CutexAlignedProfile,
            Self::CutexFableLimit,
            Self::CutexCustomizedLimit,
            Self::CutexCustomizedLimitBar,
            Self::Hostname,
            Self::Status,
            Self::ThreadName,
            Self::ThreadTitle,
            Self::GitBranch,
            Self::PullRequestNumber,
            Self::BranchChanges,
            Self::Permissions,
            Self::ApprovalMode,
            Self::ContextRemaining,
            Self::ContextUsed,
            Self::FiveHourLimit,
            Self::WeeklyLimit,
            Self::CutexFiveHourLimit,
            Self::CutexWeeklyLimit,
            Self::CutexSimpleWeeklyLimit,
            Self::CodexVersion,
            Self::CutexVersion,
            Self::ContextWindowSize,
            Self::UsedTokens,
            Self::TotalInputTokens,
            Self::TotalOutputTokens,
            Self::ThreadCredits,
            Self::EstimatedThreadCost,
            Self::SessionId,
            Self::FastMode,
            Self::Daybreak,
            Self::RawOutput,
            Self::WorkspaceHeadline,
            Self::Model,
            Self::ModelWithReasoning,
            Self::Reasoning,
            Self::TaskProgress,
        ]
        .into_iter()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreviewValue {
    text: String,
    is_placeholder: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StatusSurfacePreviewData {
    pub(crate) notification_style: Option<ratatui::style::Style>,
    pub(crate) thread_id: Option<ThreadId>,
    values: BTreeMap<StatusSurfacePreviewItem, PreviewValue>,
}

impl Default for StatusSurfacePreviewData {
    fn default() -> Self {
        let mut data = Self {
            thread_id: None,
            values: BTreeMap::new(),
            notification_style: None,
        };
        for item in StatusSurfacePreviewItem::iter() {
            data.set_placeholder(item, item.placeholder());
        }
        data
    }
}

impl StatusSurfacePreviewData {
    pub(crate) fn from_iter<I, V>(values: I) -> Self
    where
        I: IntoIterator<Item = (StatusSurfacePreviewItem, V)>,
        V: Into<String>,
    {
        let mut data = Self::default();
        for (item, value) in values {
            data.set_live(item, value);
        }
        data
    }

    pub(crate) fn set_live<V>(&mut self, item: StatusSurfacePreviewItem, value: V)
    where
        V: Into<String>,
    {
        self.values.insert(
            item,
            PreviewValue {
                text: value.into(),
                is_placeholder: false,
            },
        );
    }

    pub(crate) fn set_placeholder<V>(&mut self, item: StatusSurfacePreviewItem, value: V)
    where
        V: Into<String>,
    {
        if self
            .values
            .get(&item)
            .is_some_and(|value| !value.is_placeholder)
        {
            return;
        }
        self.values.insert(
            item,
            PreviewValue {
                text: value.into(),
                is_placeholder: true,
            },
        );
    }

    pub(crate) fn suppress_placeholder(&mut self, item: StatusSurfacePreviewItem) {
        if self
            .values
            .get(&item)
            .is_some_and(|value| value.is_placeholder)
        {
            self.values.remove(&item);
        }
    }

    pub(crate) fn rate_limit_item_name(
        &self,
        item: StatusSurfacePreviewItem,
        fallback: &str,
    ) -> String {
        self.live_value_for(item)
            .and_then(rate_limit_preview_copy)
            .map(|copy| copy.name.to_string())
            .unwrap_or_else(|| fallback.to_string())
    }

    pub(crate) fn rate_limit_item_description(
        &self,
        item: StatusSurfacePreviewItem,
        fallback: &str,
    ) -> String {
        self.live_value_for(item)
            .and_then(rate_limit_preview_copy)
            .map(|copy| copy.description.to_string())
            .unwrap_or_else(|| fallback.to_string())
    }

    pub(crate) fn value_for(&self, item: StatusSurfacePreviewItem) -> Option<&str> {
        self.values.get(&item).map(|value| value.text.as_str())
    }

    fn live_value_for(&self, item: StatusSurfacePreviewItem) -> Option<&str> {
        self.values
            .get(&item)
            .filter(|value| !value.is_placeholder)
            .map(|value| value.text.as_str())
    }

    pub(crate) fn status_lines_for_items<I>(
        &self,
        items: I,
        use_theme_colors: bool,
    ) -> Vec<Line<'static>>
    where
        I: IntoIterator<Item = StatusLineItem>,
    {
        super::status_line_layout::split(items)
            .into_iter()
            .filter_map(|row| self.status_line_for_items(row, use_theme_colors))
            .collect()
    }

    pub(crate) fn status_line_for_items<I>(
        &self,
        items: I,
        use_theme_colors: bool,
    ) -> Option<Line<'static>>
    where
        I: IntoIterator<Item = StatusLineItem>,
    {
        let segments = items.into_iter().filter_map(|item| {
            if let Some(value) = super::status_line_layout::spacing(item) {
                return Some((item, value));
            }
            if crate::custom_status_items::is_custom(item) {
                return Some((item, crate::custom_status_items::value(item)));
            }
            self.value_for(item.preview_item()).map(|value| {
                (
                    item,
                    crate::cutex_status_presentation::text(item, value.to_string()),
                )
            })
        });
        super::status_line_style::status_line_with_notification_style(
            segments.collect(),
            use_theme_colors,
            self.notification_style,
            self.thread_id,
        )
    }
}

struct RateLimitPreviewCopy {
    name: &'static str,
    description: &'static str,
}

fn rate_limit_preview_copy(value: &str) -> Option<RateLimitPreviewCopy> {
    let value = value.trim_start();
    if value.starts_with("secondary usage ") {
        Some(RateLimitPreviewCopy {
            name: "secondary-usage-limit",
            description: "Remaining usage on the secondary usage limit (omitted when unavailable)",
        })
    } else if value.starts_with("usage ") {
        Some(RateLimitPreviewCopy {
            name: "usage-limit",
            description: "Remaining usage on the primary usage limit (omitted when unavailable)",
        })
    } else if value.starts_with("5h ") {
        Some(RateLimitPreviewCopy {
            name: "five-hour-limit",
            description: "Remaining usage on the 5-hour usage limit (omitted when unavailable)",
        })
    } else if value.starts_with("daily ") {
        Some(RateLimitPreviewCopy {
            name: "daily-limit",
            description: "Remaining usage on the daily usage limit (omitted when unavailable)",
        })
    } else if value.starts_with("weekly ") {
        Some(RateLimitPreviewCopy {
            name: "weekly-limit",
            description: "Remaining usage on the weekly usage limit (omitted when unavailable)",
        })
    } else if value.starts_with("monthly ") {
        Some(RateLimitPreviewCopy {
            name: "monthly-limit",
            description: "Remaining usage on the monthly usage limit (omitted when unavailable)",
        })
    } else if value.starts_with("annual ") {
        Some(RateLimitPreviewCopy {
            name: "annual-limit",
            description: "Remaining usage on the annual usage limit (omitted when unavailable)",
        })
    } else {
        None
    }
}
