use super::session::Session;
use crate::external_input::Runtime;
use codex_protocol::error::CodexErr;
use codex_protocol::external_input::Processing;
use tokio::sync::MutexGuard;

impl Session {
    /// Hold through migration so a concurrent admission cannot create unfinished
    /// work between the check and the history snapshot or rollback marker.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "migration must exclude admission while validating its exact history snapshot"
    )]
    pub(crate) async fn external_input_migration_guard(
        &self,
    ) -> Result<Option<MutexGuard<'_, Option<Runtime>>>, CodexErr> {
        let guard = self.external_input.lock().await;
        if let Some(runtime) = guard.as_ref()
            && (runtime.poisoned
                || !runtime.pending.is_empty()
                || runtime.recovery.messages.values().any(|found| {
                    !matches!(
                        found.processing,
                        Processing::None | Processing::OutputObserved(_)
                    )
                }))
        {
            return Err(CodexErr::InvalidRequest(
                "cannot migrate unfinished or uncertain ExternalInput history".into(),
            ));
        }
        if let Some(live) = self.services.live_thread.as_ref()
            && let Some(path) = live
                .local_rollout_path()
                .await
                .map_err(|err| CodexErr::InvalidRequest(err.to_string()))?
        {
            // Fresh threads allocate a rollout path before writing the file.
            // Materialize under the admission guard before strictly reading it.
            self.try_ensure_rollout_materialized(codex_thread_store::PersistContext::Standard)
                .await?;
            let (items, _, parse_errors) =
                codex_rollout::RolloutRecorder::load_rollout_items(&path).await?;
            if parse_errors != 0 {
                return Err(CodexErr::InvalidRequest(
                    "unreadable migration source history".into(),
                ));
            }
            crate::external_input::recovery::ensure_migration_allowed(&items)
                .map_err(|err| CodexErr::InvalidRequest(err.to_string()))?;
        }
        if guard.is_some() {
            Ok(Some(guard))
        } else {
            Ok(None)
        }
    }

    /// Hold admission while a conversation fork snapshots this thread. Unlike migration, a fork
    /// leaves every ExternalInput obligation with this thread: the child is never bound to the
    /// owner and copies no ledger facts, so pending, claimed or held messages do not block it.
    // Fork excludes admission while it copies the source history: the caller holds the guard.
    pub(crate) async fn external_input_fork_guard(
        &self,
    ) -> Result<Option<MutexGuard<'_, Option<Runtime>>>, CodexErr> {
        let guard = self.external_input.lock().await;
        if guard.is_some() {
            Ok(Some(guard))
        } else {
            Ok(None)
        }
    }
}
