use super::external_input_processor::ExternalInputRequestProcessor;
use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::PresentationAppendParams;
use codex_app_server_protocol::PresentationAppendedNotification;
use codex_app_server_protocol::PresentationResponse;
use codex_app_server_protocol::PresentationStatusParams;
use codex_app_server_protocol::ServerNotification;
use codex_protocol::presentation::PresentationAppended;

impl ExternalInputRequestProcessor {
    pub(crate) async fn presentation_append(
        &self,
        params: PresentationAppendParams,
    ) -> Result<PresentationResponse, JSONRPCErrorError> {
        let mut record = PresentationAppended {
            version: params.version,
            owner_id: params.owner_id.clone(),
            origin_thread_id: params.thread_id.clone(),
            presentation: params.presentation,
            semantic_sha256: params.semantic_sha256,
            receipt_id: String::new(),
        };
        record.receipt_id = record.receipt_digest();
        record.validate().map_err(invalid_request)?;
        let thread = self
            .require_thread(
                params.version,
                &params.owner_id,
                &params.thread_id,
                params.runtime_generation,
            )
            .await?;
        let (receipt, first) = thread
            .append_presentation(record)
            .await
            .map_err(|error| internal_error(error.to_string()))?;
        if first {
            let position = thread
                .presentation_position(&receipt.presentation.id)
                .await
                .map_err(|error| internal_error(error.to_string()))?;
            self.outgoing
                .send_server_notification(ServerNotification::ThreadPresentationAppended(
                    PresentationAppendedNotification {
                        thread_id: params.thread_id.clone(),
                        position,
                        item: receipt.clone(),
                    },
                ))
                .await;
        }
        Ok(PresentationResponse {
            version: 1,
            owner_id: params.owner_id,
            thread_id: params.thread_id,
            runtime_generation: params.runtime_generation,
            receipt: Some(receipt),
        })
    }

    pub(crate) async fn presentation_status(
        &self,
        params: PresentationStatusParams,
    ) -> Result<PresentationResponse, JSONRPCErrorError> {
        if params.presentation_id.is_empty()
            || params.presentation_id.len() > 256
            || params.semantic_sha256.len() != 64
            || !params
                .semantic_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid_request("invalid presentation identity"));
        }
        let thread = self
            .require_thread(
                params.version,
                &params.owner_id,
                &params.thread_id,
                params.runtime_generation,
            )
            .await?;
        let receipt = thread
            .presentation_status(
                &params.owner_id,
                &params.presentation_id,
                &params.semantic_sha256,
            )
            .await
            .map_err(|error| internal_error(error.to_string()))?;
        Ok(PresentationResponse {
            version: 1,
            owner_id: params.owner_id,
            thread_id: params.thread_id,
            runtime_generation: params.runtime_generation,
            receipt,
        })
    }
}
