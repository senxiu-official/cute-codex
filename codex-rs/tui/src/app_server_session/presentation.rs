//! Read-only reconciliation of the receiver's display timeline. No input submission.
use super::*;
use codex_app_server_protocol::ThreadTimelineEntry;
use codex_app_server_protocol::ThreadTimelineListParams;
use codex_app_server_protocol::ThreadTimelineListResponse;

impl AppServerSession {
    pub(crate) async fn presentation_timeline(
        &mut self,
        thread_id: ThreadId,
    ) -> Result<Option<Vec<ThreadTimelineEntry>>> {
        if self.client.presentation_version() != Some(1) {
            return Ok(None);
        }
        Ok(Some(
            load_presentation_timeline(self.request_handle(), thread_id).await?,
        ))
    }

    pub(crate) fn request_presentation_reconcile(
        &self,
        thread_id: ThreadId,
        tx: crate::app_event_sender::AppEventSender,
    ) {
        if self.client.presentation_version() != Some(1) {
            return;
        }
        let handle = self.request_handle();
        tokio::spawn(async move {
            let result = load_presentation_timeline(handle, thread_id)
                .await
                .map_err(|e| e.to_string());
            tx.send(crate::app_event::AppEvent::PresentationTimelineLoaded { thread_id, result });
        });
    }
}

async fn load_presentation_timeline(
    handle: codex_app_server_client::AppServerRequestHandle,
    thread_id: ThreadId,
) -> Result<Vec<ThreadTimelineEntry>> {
    let mut cursor = None;
    let mut seen = HashSet::new();
    let mut pages = Vec::new();
    loop {
        let request_id = codex_app_server_protocol::RequestId::String(format!(
            "presentation-{}",
            uuid::Uuid::new_v4()
        ));
        let page: ThreadTimelineListResponse = handle
            .request_typed(ClientRequest::ThreadTimelineList {
                request_id,
                params: ThreadTimelineListParams {
                    thread_id: thread_id.to_string(),
                    cursor,
                    limit: Some(100),
                },
            })
            .await?;
        for entry in &page.data {
            if let ThreadTimelineEntry::Presentation { item, .. } = entry {
                item.validate().map_err(color_eyre::eyre::Report::msg)?;
            }
        }
        pages.push(page.data);
        cursor = page.next_cursor;
        let Some(next) = &cursor else {
            break;
        };
        if !seen.insert(next.clone()) {
            return Err(color_eyre::eyre::eyre!(
                "presentation timeline cursor did not advance"
            ));
        }
    }
    Ok(pages.into_iter().rev().flatten().collect())
}
