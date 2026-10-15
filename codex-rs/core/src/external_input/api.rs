use crate::CodexThread;
use anyhow::Result;
use codex_extension_api::ThreadIdleCause;
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input_record::MessageKey;
use codex_protocol::external_input_status::BusyReason;
use codex_protocol::external_input_status::Status;
use codex_protocol::protocol::TurnAbortReason;
use uuid::Uuid;

impl CodexThread {
    pub async fn subscribe_external_input_status(
        &self,
    ) -> Result<tokio::sync::broadcast::Receiver<String>> {
        let state = self.session.external_input.lock().await;
        let runtime = state
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("external input disabled"))?;
        Ok(runtime.changed.subscribe())
    }

    pub async fn submit_external_input(&self, envelope: Envelope) -> Result<Status> {
        let active = envelope.message.delivery.is_active();
        let (status, inserted) = self.session.admit_external_input(envelope).await?;
        if inserted && active {
            self.session
                .emit_thread_idle_lifecycle_if_idle(ThreadIdleCause::Completed)
                .await;
            self.session.dispatch_external_input().await;
        }
        Ok(status)
    }

    pub async fn external_input_status(
        &self,
        owner: &str,
        message_id: &str,
        digest: &str,
    ) -> Result<Status> {
        self.session
            .external_input_status(owner, message_id, digest)
            .await
    }

    pub async fn retry_external_input(
        &self,
        owner: &str,
        key: MessageKey,
        expected: Option<Uuid>,
        retry_id: &str,
    ) -> Result<()> {
        if self
            .session
            .retry_external_input(owner, key, expected, retry_id)
            .await?
        {
            self.session
                .emit_thread_idle_lifecycle_if_idle(ThreadIdleCause::Completed)
                .await;
            self.session.dispatch_external_input().await;
        }
        Ok(())
    }

    pub async fn pause_external_input(&self) -> Result<()> {
        self.session.external_input_gate(true).await.map(|_| ())
    }

    /// Exclusive admission: starts a turn for this item at once, or refuses
    /// without queueing. Returns the item status and the turn that carries it;
    /// an exact replay returns the known status and turn without a new turn.
    pub async fn submit_external_input_exclusive(
        &self,
        envelope: Envelope,
    ) -> Result<std::result::Result<(Status, Option<String>), BusyReason>> {
        self.session.submit_exclusive_external_input(envelope).await
    }

    /// Reopens the interruption gate without a human turn. Returns whether it
    /// was closed.
    pub async fn resume_external_input(&self) -> Result<bool> {
        let reopened = self.session.external_input_gate(false).await?;
        if reopened {
            self.session
                .emit_thread_idle_lifecycle_if_idle(ThreadIdleCause::Completed)
                .await;
            self.session.dispatch_external_input().await;
        }
        Ok(reopened)
    }

    /// Interrupts the named active turn without closing the interruption gate.
    /// Returns false if that turn is not active.
    pub async fn interrupt_turn_keeping_external_input(&self, turn_id: &str) -> bool {
        let aborted = self
            .session
            .abort_turn_if_active(turn_id, TurnAbortReason::Interrupted, /*error*/ None)
            .await;
        if aborted {
            // Aborted tasks skip the completion path that normally dispatches.
            self.session.dispatch_external_input().await;
        }
        aborted
    }
}
