//! One acknowledgement per current reminder, with bounded asynchronous retries.
use super::*;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::{self};

#[derive(Default)]
pub(super) struct NotificationAck {
    current: Option<String>,
    submitted: Option<String>,
    pending: Option<String>,
    response: Option<Receiver<Result<(), String>>>,
    retry_at: Option<Instant>,
    attempts: u8,
    last_interaction: Option<chrono::DateTime<chrono::Utc>>,
}

impl NotificationAck {
    pub(super) fn observe(&mut self, reminder: Option<String>) {
        if self.current != reminder {
            self.current = reminder;
            self.pending = None;
            self.retry_at = None;
            self.attempts = 0;
        }
    }

    pub(super) fn observe_at(
        &mut self,
        reminder: Option<String>,
        at: Option<chrono::DateTime<chrono::Utc>>,
    ) {
        self.observe(reminder);
        if at
            .zip(self.last_interaction)
            .is_some_and(|(reminder, input)| reminder <= input)
        {
            self.queue_current();
        }
    }

    fn interact(&mut self) {
        self.last_interaction = Some(chrono::Utc::now());
        self.queue_current();
    }

    fn queue_current(&mut self) {
        if self.current.is_some() && self.current != self.submitted && self.pending.is_none() {
            self.pending = self.current.clone();
            self.attempts = 0;
        }
    }

    pub(super) fn poll(&mut self, thread: &str, frame: &crate::tui::FrameRequester) {
        if let Some(rx) = &self.response {
            match rx.try_recv() {
                Ok(result) => {
                    self.response = None;
                    if result.is_err() && self.current == self.submitted && self.attempts < 3 {
                        self.pending = self.current.clone();
                        self.retry_at = Some(Instant::now() + Duration::from_secs(5));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.response = None;
                    // A crashed task is retried by the same bounded policy.
                    if self.current == self.submitted && self.attempts < 3 {
                        self.pending = self.current.clone();
                        self.retry_at = Some(Instant::now() + Duration::from_secs(5));
                    }
                }
            }
        }
        if let Some(due) = self.retry_at
            && due > Instant::now()
        {
            frame.schedule_frame_in(due.saturating_duration_since(Instant::now()));
            return;
        }
        let Some(reminder) = self.pending.take() else {
            return;
        };
        self.submitted = Some(reminder.clone());
        self.retry_at = None;
        self.attempts += 1;
        let (tx, rx) = mpsc::channel();
        self.response = Some(rx);
        let thread = thread.to_owned();
        let frame = frame.clone();
        tokio::spawn(async move {
            let result = acknowledge(&thread, &reminder).await;
            let _ = tx.send(result);
            frame.schedule_frame();
        });
    }
}

impl ChatWidget {
    pub(crate) fn acknowledge_notification_interaction(&mut self) {
        let Some(thread) = self.thread_id().map(|id| id.to_string()) else {
            return;
        };
        if self.notification_control.thread.as_deref() != Some(thread.as_str()) {
            return;
        }
        self.notification_control.ack.interact();
        self.notification_control
            .ack
            .poll(&thread, &self.frame_requester);
    }
}

async fn acknowledge(thread: &str, reminder: &str) -> Result<(), String> {
    let helper = std::env::var_os("CUTEX_NOTIFICATION_CONTROL").unwrap_or_else(|| "cutex".into());
    let mut command = tokio::process::Command::new(helper);
    command
        .args(["notify", "ack", thread, reminder])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let status = tokio::time::timeout(Duration::from_secs(3), command.status())
        .await
        .map_err(|_| "ack helper timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("ack helper failed".to_owned())
    }
}

#[cfg(test)]
#[path = "notification_ack_tests.rs"]
mod tests;
