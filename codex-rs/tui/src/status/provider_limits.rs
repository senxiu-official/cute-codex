//! Optional provider quota observations use the existing status card rows.
//! Percentages are never inferred from an allowed/rejected state or reset time.
use super::rate_limits::StatusRateLimitData;
use super::rate_limits::StatusRateLimitRow;
use super::rate_limits::StatusRateLimitValue;
use chrono::DateTime;
use chrono::Local;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    provider_rate_limits: Option<Observations>,
    #[serde(flatten)]
    standard: codex_app_server_protocol::GetAccountRateLimitsResponse,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Observations {
    version: u32,
    rows: Vec<Observation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    label: String,
    status: String,
    used_percent: Option<f64>,
    resets_at: Option<i64>,
    observed_at: i64,
}

pub(crate) async fn fetch(
    handle: AppServerRequestHandle,
) -> (
    StatusRateLimitData,
    Vec<codex_app_server_protocol::RateLimitSnapshot>,
) {
    let result = tokio::time::timeout(
        Duration::from_secs(/*secs*/ 10),
        handle.request_typed::<Response>(ClientRequest::GetAccountRateLimits {
            request_id: RequestId::String(uuid::Uuid::new_v4().to_string()),
            params: None,
        }),
    )
    .await;
    match result {
        Ok(Ok(Response {
            provider_rate_limits: Some(value),
            standard,
        })) => (
            value.display(Local::now()),
            crate::app_server_session::app_server_rate_limit_snapshots(standard),
        ),
        _ => (StatusRateLimitData::Unavailable, Vec::new()),
    }
}

// Claude reports the whole context, including its own prompt overhead.
// Codex's fixed 12k baseline must not be subtracted from these observations.
pub(crate) fn context_remaining(used: i64, maximum: i64) -> i64 {
    if maximum <= 0 {
        return 0;
    }
    (100.0 * (1.0 - used.max(0) as f64 / maximum as f64))
        .clamp(0.0, 100.0)
        .round() as i64
}

impl Observations {
    fn display(self, now: DateTime<Local>) -> StatusRateLimitData {
        if self.version != 1 {
            return StatusRateLimitData::Unavailable;
        }
        if self.rows.is_empty() {
            return StatusRateLimitData::Available(vec![StatusRateLimitRow {
                label: "Claude limits".into(),
                value: StatusRateLimitValue::Text("Awaiting quota data from Claude".into()),
            }]);
        }
        let mut stale = false;
        let mut rows = Vec::new();
        for row in self.rows {
            let status = match row.status.as_str() {
                "observed" => "Observed",
                "allowed" => "Allowed",
                "allowed_warning" => "Near limit",
                "rejected" => "Limit reached",
                _ => "Unknown",
            };
            let local_time = |seconds| {
                DateTime::from_timestamp(seconds, 0)
                    .map(|time| time.with_timezone(&Local).format("%m-%d %H:%M").to_string())
            };
            stale |= now.timestamp().saturating_sub(row.observed_at) > 15 * 60
                || row.resets_at.is_some_and(|reset| reset <= now.timestamp());
            let updated = local_time(row.observed_at).unwrap_or_else(|| "unknown".into());
            let reset = row.resets_at.and_then(local_time);
            let details = format!("{status} · updated {updated}");
            let value = match row
                .used_percent
                .filter(|p| p.is_finite() && (0.0..=100.0).contains(p))
            {
                Some(percent_used) => StatusRateLimitValue::Window {
                    percent_used,
                    resets_at: reset,
                    details: Some(details),
                },
                None => {
                    let reset = reset.map(|s| format!(" · resets {s}")).unwrap_or_default();
                    StatusRateLimitValue::Text(format!(
                        "{status} · percentage unavailable{reset} · updated {updated}"
                    ))
                }
            };
            rows.push(StatusRateLimitRow {
                label: row.label,
                value,
            });
        }
        if stale {
            StatusRateLimitData::Stale(rows)
        } else {
            StatusRateLimitData::Available(rows)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_percent_is_text_and_expired_reset_stays_stale() {
        let observations: Observations = serde_json::from_value(serde_json::json!({
            "version":1,"rows":[{"label":"Claude 5h","status":"allowed","usedPercent":null,"resetsAt":1000,"observedAt":900}]
        })).unwrap();
        let StatusRateLimitData::Stale(rows) = observations.display(
            DateTime::from_timestamp(1100, 0)
                .unwrap()
                .with_timezone(&Local),
        ) else {
            panic!("expired snapshot must be stale")
        };
        let StatusRateLimitValue::Text(text) = &rows[0].value else {
            panic!("must not invent a percentage")
        };
        assert!(text.contains("percentage unavailable"));
        assert!(text.contains("updated"));
    }
    #[test]
    fn supplied_percent_uses_existing_window_renderer() {
        let observations: Observations = serde_json::from_value(serde_json::json!({
            "version":1,"rows":[{"label":"Claude 7d","status":"allowed_warning","usedPercent":81.5,"resetsAt":null,"observedAt":900}]
        })).unwrap();
        let StatusRateLimitData::Available(rows) = observations.display(
            DateTime::from_timestamp(1000, 0)
                .unwrap()
                .with_timezone(&Local),
        ) else {
            panic!("recent snapshot expected")
        };
        let StatusRateLimitValue::Window { percent_used, .. } = rows[0].value else {
            panic!("expected window")
        };
        assert_eq!(percent_used, 81.5);
    }
}
