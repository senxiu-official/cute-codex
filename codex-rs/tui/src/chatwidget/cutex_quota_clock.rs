//! Refresh a local reset countdown without polling the quota API.
use super::*;
impl ChatWidget {
    pub(super) fn refresh_cutex_quota_clock(&mut self) {
        let enabled = self.status_line_items_with_invalids().0.iter().any(|item| {
            matches!(
                item,
                StatusLineItem::CutexFiveHourLimit
                    | StatusLineItem::CutexWeeklyLimit
                    | StatusLineItem::CutexSimpleWeeklyLimit
                    | StatusLineItem::CutexSimpleFiveHourLimit
                    | StatusLineItem::CutexCustomizedLimit
                    | StatusLineItem::CutexCustomizedLimitBar
                    | StatusLineItem::CutexFableLimitBar
                    | StatusLineItem::CutexFableLimit
            )
        });
        if !enabled {
            self.cutex_quota_minute = None;
            return;
        }
        let now = chrono::Utc::now().timestamp();
        let minute = now.div_euclid(60);
        if self.cutex_quota_minute != Some(minute) {
            self.cutex_quota_minute = Some(minute);
            self.refresh_status_line();
        }
        // Earlier animation/input frames coalesce away later requests; re-arm every draw.
        self.frame_requester
            .schedule_frame_in(std::time::Duration::from_secs(
                (60 - now.rem_euclid(60)) as u64,
            ));
    }
}
