//! Thin UI adapter. Cutex owns persistence and label configuration.
use super::*;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::{self};

#[derive(Default)]
pub(super) struct NotificationControl {
    pub(super) thread: Option<String>,
    pub(super) ack: super::notification_ack::NotificationAck,
    pub(super) label: Option<String>,
    pub(super) style: Option<ratatui::style::Style>,
    pub(super) custom_limit: Option<crate::cutex_custom_limit::Display>,
    queued_cycles: usize,
    response: Option<Receiver<Result<NotificationDisplay, String>>>,
    next_read: Option<Instant>,
}

impl ChatWidget {
    pub(super) fn refresh_notification_control(&mut self, cycle: bool) {
        let thread = self.thread_id().map(|id| id.to_string());
        if self.notification_control.thread != thread {
            self.notification_control = NotificationControl {
                thread: thread.clone(),
                ..Default::default()
            };
        }
        let Some(thread) = thread else { return };
        self.notification_control
            .ack
            .poll(&thread, &self.frame_requester);
        if cycle {
            self.notification_control.queued_cycles =
                (self.notification_control.queued_cycles + 1) % 3;
        }
        if let Some(response) = self.notification_control.response.as_ref() {
            match response.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(display) => {
                            self.notification_control
                                .ack
                                .observe_at(display.reminder_id, display.reminder_at);
                            self.notification_control
                                .ack
                                .poll(&thread, &self.frame_requester);
                            self.notification_control.custom_limit = display.custom_limit;
                            self.notification_control.label = Some(display.label);
                            self.notification_control.style = Some(display.style);
                        }
                        Err(error) => {
                            if let Some(limit) = &mut self.notification_control.custom_limit {
                                limit.stale = true;
                            }
                            tracing::warn!(%error, "Cutex notification preference unavailable");
                            self.notification_control.label = Some("NOTIFY?".into());
                            self.notification_control.style = None;
                        }
                    }
                    self.notification_control.response = None;
                    self.notification_control.next_read =
                        Some(Instant::now() + Duration::from_secs(5));
                    self.refresh_status_line();
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if let Some(limit) = &mut self.notification_control.custom_limit {
                        limit.stale = true;
                    }
                    self.notification_control.response = None;
                    self.notification_control.label = Some("NOTIFY?".into());
                    self.notification_control.style = None;
                    self.notification_control.next_read =
                        Some(Instant::now() + Duration::from_secs(5));
                    self.refresh_status_line();
                }
            }
        }
        let cycle = self.notification_control.queued_cycles > 0;
        if !cycle
            && let Some(due) = self.notification_control.next_read
            && due > Instant::now()
        {
            self.frame_requester
                .schedule_frame_in(due.saturating_duration_since(Instant::now()));
            return;
        }
        // Receipt polling belongs to the session, independently of item visibility.
        if cycle {
            self.notification_control.queued_cycles -= 1;
        }
        let (tx, rx) = mpsc::channel();
        self.notification_control.response = Some(rx);
        let frame = self.frame_requester.clone();
        tokio::spawn(async move {
            let result = query(&thread, cycle).await;
            let _ = tx.send(result);
            frame.schedule_frame();
        });
    }
}

async fn query(thread: &str, cycle: bool) -> Result<NotificationDisplay, String> {
    let helper = std::env::var_os("CUTEX_NOTIFICATION_CONTROL").unwrap_or_else(|| "cutex".into());
    let mut command = tokio::process::Command::new(helper);
    command
        .args(["notify", "session", thread, "--json"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    if cycle {
        command.arg("--cycle");
    }
    let output = tokio::time::timeout(Duration::from_secs(12), command.output())
        .await
        .map_err(|_| "preference helper timed out".to_string())?
        .map_err(|error| error.to_string())?;
    if !output.status.success() || output.stdout.len() > 8192 {
        return Err("preference helper failed".into());
    }
    Ok(NotificationDisplay {
        custom_limit: crate::cutex_custom_limit::parse_response(&output.stdout),
        label: parse_response(thread, &output.stdout)?,
        style: parse_style(&output.stdout)?,
        reminder_id: parse_reminder(&output.stdout)?,
        reminder_at: serde_json::from_slice::<serde_json::Value>(&output.stdout)
            .map_err(|e| e.to_string())?["reminder_at"]
            .as_str()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.with_timezone(&chrono::Utc)),
    })
}

struct NotificationDisplay {
    custom_limit: Option<crate::cutex_custom_limit::Display>,
    reminder_id: Option<String>,
    reminder_at: Option<chrono::DateTime<chrono::Utc>>,
    label: String,
    style: ratatui::style::Style,
}

fn parse_style(bytes: &[u8]) -> Result<ratatui::style::Style, String> {
    #[derive(Default, serde::Deserialize)]
    struct ItemStyle {
        fg: Option<String>,
        #[serde(default)]
        bold: bool,
    }
    #[derive(serde::Deserialize)]
    struct Response {
        #[serde(default)]
        style: ItemStyle,
    }
    let response: Response = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let mut style = ratatui::style::Style::default();
    if let Some(fg) = response.style.fg {
        style = style
            .fg(crate::custom_status_items::parse_color(&fg).map_err(|error| error.to_string())?);
    }
    if response.style.bold {
        style = style.add_modifier(ratatui::style::Modifier::BOLD);
    }
    Ok(style)
}

fn parse_response(thread: &str, bytes: &[u8]) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Response {
        thread_id: String,
        label: String,
    }
    let response: Response = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if response.thread_id != thread
        || response.label.trim().is_empty()
        || response.label.chars().count() > 32
        || response.label.chars().any(char::is_control)
    {
        return Err("invalid notification preference response".into());
    }
    Ok(response.label)
}

#[cfg(test)]
#[path = "notification_control_tests.rs"]
mod tests;

fn parse_reminder(bytes: &[u8]) -> Result<Option<String>, String> {
    #[derive(serde::Deserialize)]
    struct Response {
        reminder_id: Option<String>,
    }
    let response: Response = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if response
        .reminder_id
        .as_ref()
        .is_some_and(|id| id.is_empty() || id.len() > 512 || id.chars().any(char::is_control))
    {
        return Err("invalid reminder id".into());
    }
    Ok(response.reminder_id)
}
