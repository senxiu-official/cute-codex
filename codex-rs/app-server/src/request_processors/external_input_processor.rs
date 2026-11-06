use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use crate::external_input_binding::ExternalInputBinding;
use crate::outgoing_message::OutgoingMessageSender;
use codex_app_server_protocol::ExternalInputAdmission;
use codex_app_server_protocol::ExternalInputInterruptParams;
use codex_app_server_protocol::ExternalInputResponse;
use codex_app_server_protocol::ExternalInputResumeParams;
use codex_app_server_protocol::ExternalInputResumeResponse;
use codex_app_server_protocol::ExternalInputRetryDisposition;
use codex_app_server_protocol::ExternalInputRetryParams;
use codex_app_server_protocol::ExternalInputRetryResponse;
use codex_app_server_protocol::ExternalInputStatusChangedNotification;
use codex_app_server_protocol::ExternalInputStatusParams;
use codex_app_server_protocol::ExternalInputSubmitParams;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ToolGateChangedNotification;
use codex_app_server_protocol::ToolGateHeldCall;
use codex_app_server_protocol::ToolGateSetParams;
use codex_app_server_protocol::ToolGateSetResponse;
use codex_core::CodexThread;
use codex_core::ThreadManager;
use codex_protocol::ThreadId;
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input_record::MessageKey;
use codex_protocol::external_input_status::BusyReason;
use std::sync::Arc;
use std::sync::Weak;
use tokio::sync::Mutex;

pub(crate) struct ExternalInputRequestProcessor {
    binding: Option<ExternalInputBinding>,
    threads: Arc<ThreadManager>,
    pub(super) outgoing: Arc<OutgoingMessageSender>,
    listener: Mutex<Option<(Weak<CodexThread>, tokio::task::JoinHandle<()>)>>,
    tool_gate_listener: Mutex<Option<(Weak<CodexThread>, tokio::task::JoinHandle<()>)>>,
}

fn tool_gate_waiting(state: codex_core::ToolGateState) -> Vec<ToolGateHeldCall> {
    state
        .waiting
        .into_iter()
        .map(|call| ToolGateHeldCall {
            call_id: call.call_id,
            tool: call.tool,
            turn_id: call.turn_id,
            thread_id: call.thread_id,
            since_ms: call.since_ms,
        })
        .collect()
}

impl ExternalInputRequestProcessor {
    pub(crate) fn new(
        binding: Option<ExternalInputBinding>,
        threads: Arc<ThreadManager>,
        outgoing: Arc<OutgoingMessageSender>,
    ) -> Self {
        Self {
            binding,
            threads,
            outgoing,
            listener: Mutex::new(None),
            tool_gate_listener: Mutex::new(None),
        }
    }

    pub(super) async fn require_thread(
        &self,
        version: u32,
        owner: &str,
        thread: &str,
        generation: u64,
    ) -> Result<Arc<CodexThread>, JSONRPCErrorError> {
        let binding = self
            .binding
            .as_ref()
            .ok_or_else(|| invalid_request("ExternalInput capability disabled"))?;
        if version != 1
            || version != binding.version
            || owner != binding.owner_id
            || thread != binding.thread_id
            || generation != binding.runtime_generation
        {
            return Err(invalid_request("ExternalInput launch binding mismatch"));
        }
        let id = ThreadId::from_string(thread)
            .map_err(|_| invalid_request("invalid ExternalInput thread id"))?;
        // get_thread only looks up a loaded thread. Never use resume/create here.
        let loaded = self
            .threads
            .get_thread(id)
            .await
            .map_err(|_| invalid_request("ExternalInput requires the loaded bound thread"))?;
        self.listen(&loaded, thread).await?;
        Ok(loaded)
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "only one status subscription may be installed for the loaded thread"
    )]
    async fn listen(
        &self,
        thread: &Arc<CodexThread>,
        thread_id: &str,
    ) -> Result<(), JSONRPCErrorError> {
        let mut listener = self.listener.lock().await;
        let weak = Arc::downgrade(thread);
        if listener
            .as_ref()
            .is_some_and(|(previous, task)| previous.ptr_eq(&weak) && !task.is_finished())
        {
            return Ok(());
        }
        let mut changed = thread
            .subscribe_external_input_status()
            .await
            .map_err(|error| internal_error(error.to_string()))?;
        if let Some((_, task)) = listener.take() {
            task.abort();
        }
        let outgoing = Arc::clone(&self.outgoing);
        let thread_id = thread_id.to_string();
        let task = tokio::spawn(async move {
            loop {
                match changed.recv().await {
                    Ok(message_id) => {
                        outgoing
                            .send_server_notification(
                                ServerNotification::ThreadExternalInputStatusChanged(
                                    ExternalInputStatusChangedNotification {
                                        thread_id: thread_id.clone(),
                                        message_id,
                                    },
                                ),
                            )
                            .await
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        *listener = Some((weak, task));
        Ok(())
    }

    pub(crate) async fn validate_owner_carries(
        &self,
        thread_id: &str,
        envelopes: &[Envelope],
    ) -> Result<(), JSONRPCErrorError> {
        if envelopes.len() > 100 {
            return Err(invalid_request(
                "ownerCarries accepts at most 100 envelopes",
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        for envelope in envelopes {
            validate_owner_carry(self.binding.as_ref(), envelope)?;
            if envelope.thread_id != thread_id || !ids.insert(&envelope.message.id) {
                return Err(invalid_request(
                    "ownerCarries thread mismatch or duplicate identity",
                ));
            }
            // Control binding uses v1; envelope.validate checks the independent
            // envelope version and digest (including v2 structured views).
            self.require_thread(
                1,
                &envelope.owner_id,
                &envelope.thread_id,
                envelope.runtime_generation,
            )
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn submit(
        &self,
        params: ExternalInputSubmitParams,
    ) -> Result<ExternalInputResponse, JSONRPCErrorError> {
        let admission = params.admission.unwrap_or_default();
        let envelope: Envelope = params.into();
        envelope
            .validate()
            .map_err(|error| invalid_request(error.to_string()))?;
        if admission == ExternalInputAdmission::ExclusiveTurn
            && !envelope.message.delivery.is_active()
        {
            return Err(invalid_request(
                "ExternalInput exclusive_turn admission requires an active delivery",
            ));
        }
        let thread = self
            .require_thread(
                /*version*/ 1,
                &envelope.owner_id,
                &envelope.thread_id,
                envelope.runtime_generation,
            )
            .await?;
        let (status, turn_id) = match admission {
            ExternalInputAdmission::Queue => (
                thread
                    .submit_external_input(envelope.clone())
                    .await
                    .map_err(|error| internal_error(error.to_string()))?,
                None,
            ),
            ExternalInputAdmission::ExclusiveTurn => thread
                .submit_external_input_exclusive(envelope.clone())
                .await
                .map_err(|error| internal_error(error.to_string()))?
                .map_err(busy_error)?,
        };
        Ok(ExternalInputResponse {
            version: 1,
            owner_id: envelope.owner_id,
            thread_id: envelope.thread_id,
            runtime_generation: envelope.runtime_generation,
            statuses: vec![status],
            turn_id,
        })
    }

    /// cutex fork: `thread/toolGate/set` (agent pause at the tool boundary).
    pub(crate) async fn set_tool_gate(
        &self,
        params: ToolGateSetParams,
    ) -> Result<ToolGateSetResponse, JSONRPCErrorError> {
        let thread = self
            .require_thread(
                params.version,
                &params.owner_id,
                &params.thread_id,
                params.runtime_generation,
            )
            .await?;
        self.listen_tool_gate(&thread, &params.thread_id).await;
        let state = thread.set_tool_gate(params.held);
        Ok(ToolGateSetResponse {
            thread_id: params.thread_id,
            held: state.held,
            waiting: tool_gate_waiting(state),
        })
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "only one tool gate subscription may be installed for the loaded thread"
    )]
    async fn listen_tool_gate(&self, thread: &Arc<CodexThread>, thread_id: &str) {
        let mut listener = self.tool_gate_listener.lock().await;
        let weak = Arc::downgrade(thread);
        if listener
            .as_ref()
            .is_some_and(|(previous, task)| previous.ptr_eq(&weak) && !task.is_finished())
        {
            return;
        }
        if let Some((_, task)) = listener.take() {
            task.abort();
        }
        let mut changes = thread.subscribe_tool_gate();
        changes.mark_unchanged();
        let outgoing = Arc::clone(&self.outgoing);
        let thread_id = thread_id.to_string();
        let task = tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                let state = changes.borrow_and_update().clone();
                outgoing
                    .send_server_notification(ServerNotification::ThreadToolGateChanged(
                        ToolGateChangedNotification {
                            thread_id: thread_id.clone(),
                            held: state.held,
                            waiting: tool_gate_waiting(state),
                        },
                    ))
                    .await;
            }
        });
        *listener = Some((weak, task));
    }

    pub(crate) async fn resume(
        &self,
        params: ExternalInputResumeParams,
    ) -> Result<ExternalInputResumeResponse, JSONRPCErrorError> {
        let thread = self
            .require_thread(
                params.version,
                &params.owner_id,
                &params.thread_id,
                params.runtime_generation,
            )
            .await?;
        let reopened = thread
            .resume_external_input()
            .await
            .map_err(|error| internal_error(error.to_string()))?;
        Ok(ExternalInputResumeResponse {
            version: 1,
            owner_id: params.owner_id,
            thread_id: params.thread_id,
            runtime_generation: params.runtime_generation,
            reopened,
        })
    }

    /// Checks the launch binding for `thread/externalInput/interrupt`; the turn
    /// processor owns the interrupt response.
    pub(crate) async fn check_interrupt_binding(
        &self,
        params: &ExternalInputInterruptParams,
    ) -> Result<(), JSONRPCErrorError> {
        if params.turn_id.is_empty() || params.turn_id.len() > 256 {
            return Err(invalid_request("invalid ExternalInput turn id"));
        }
        self.require_thread(
            params.version,
            &params.owner_id,
            &params.thread_id,
            params.runtime_generation,
        )
        .await
        .map(|_| ())
    }

    pub(crate) async fn status(
        &self,
        params: ExternalInputStatusParams,
    ) -> Result<ExternalInputResponse, JSONRPCErrorError> {
        if params.messages.is_empty() || params.messages.len() > 100 {
            return Err(invalid_request("ExternalInput status requires 1..100 keys"));
        }
        for key in &params.messages {
            validate_key(key)?;
        }
        let thread = self
            .require_thread(
                params.version,
                &params.owner_id,
                &params.thread_id,
                params.runtime_generation,
            )
            .await?;
        let mut statuses = Vec::with_capacity(params.messages.len());
        for key in params.messages {
            statuses.push(
                thread
                    .external_input_status(&params.owner_id, &key.message_id, &key.semantic_sha256)
                    .await
                    .map_err(|error| internal_error(error.to_string()))?,
            );
        }
        Ok(ExternalInputResponse {
            version: 1,
            owner_id: params.owner_id,
            thread_id: params.thread_id,
            runtime_generation: params.runtime_generation,
            statuses,
            turn_id: None,
        })
    }

    pub(crate) async fn retry(
        &self,
        params: ExternalInputRetryParams,
    ) -> Result<ExternalInputRetryResponse, JSONRPCErrorError> {
        let key = MessageKey {
            message_id: params.message_id.clone(),
            semantic_sha256: params.semantic_sha256.clone(),
        };
        validate_key(&key)?;
        let thread = self
            .require_thread(
                params.version,
                &params.owner_id,
                &params.thread_id,
                params.runtime_generation,
            )
            .await?;
        thread
            .retry_external_input(
                &params.owner_id,
                key,
                params.expected_attempt_id,
                &params.retry_id,
            )
            .await
            .map_err(|error| invalid_request(error.to_string()))?;
        Ok(ExternalInputRetryResponse {
            version: 1,
            owner_id: params.owner_id,
            thread_id: params.thread_id,
            runtime_generation: params.runtime_generation,
            message_id: params.message_id,
            semantic_sha256: params.semantic_sha256,
            expected_attempt_id: params.expected_attempt_id,
            retry_id: params.retry_id,
            disposition: ExternalInputRetryDisposition::Released,
        })
    }
}

fn validate_owner_carry(
    binding: Option<&ExternalInputBinding>,
    envelope: &Envelope,
) -> Result<(), JSONRPCErrorError> {
    envelope
        .validate()
        .map_err(|error| invalid_request(error.to_string()))?;
    let binding = binding.ok_or_else(|| invalid_request("ExternalInput capability disabled"))?;
    if binding.version != 1
        || envelope.owner_id != binding.owner_id
        || envelope.thread_id != binding.thread_id
        || envelope.runtime_generation != binding.runtime_generation
    {
        return Err(invalid_request("ExternalInput launch binding mismatch"));
    }
    let item = envelope
        .response_item()
        .map_err(|error| invalid_request(error.to_string()))?;
    let bytes = serde_json::to_vec(&item).map_err(|error| invalid_request(error.to_string()))?;
    if !binding.canonical_byte_limit.permits(bytes.len()) {
        return Err(invalid_request("receiver canonical byte policy exceeded"));
    }
    Ok(())
}

/// `data.externalInputError` value of a refused exclusive admission.
pub(crate) const EXTERNAL_INPUT_BUSY_ERROR: &str = "busy";

fn busy_error(reason: BusyReason) -> JSONRPCErrorError {
    let reason = serde_json::to_value(reason).unwrap_or_default();
    let mut error = invalid_request(format!(
        "ExternalInput exclusive admission refused: busy ({})",
        reason.as_str().unwrap_or("unknown")
    ));
    error.data = Some(serde_json::json!({
        "externalInputError": EXTERNAL_INPUT_BUSY_ERROR,
        "reason": reason,
    }));
    error
}

fn validate_key(key: &MessageKey) -> Result<(), JSONRPCErrorError> {
    if key.message_id.is_empty()
        || key.message_id.len() > 256
        || key.semantic_sha256.len() != 64
        || !key
            .semantic_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid_request("invalid ExternalInput identity"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn owner_envelope() -> Envelope {
        use codex_protocol::external_input::Delivery;
        use codex_protocol::external_input::Message;
        use codex_protocol::external_input::Source;
        use codex_protocol::external_input::SourceKind;
        let mut value = Envelope {
            version: 1,
            owner_id: "owner".into(),
            thread_id: "thread".into(),
            runtime_generation: 7,
            message: Message {
                id: "notice".into(),
                source: Source {
                    kind: SourceKind::Service,
                    id: "alarm".into(),
                },
                event_type: "alarm".into(),
                delivery: Delivery::Soon,
                text: "notice".into(),
            },
            semantic_sha256: String::new(),
            view: None,
        };
        value.semantic_sha256 = value.digest();
        value
    }

    #[test]
    fn owner_carries_validate_trusted_launch_generation_digest_and_canonical_size() {
        let mut binding = ExternalInputBinding {
            version: 1,
            owner_id: "owner".into(),
            thread_id: "thread".into(),
            runtime_generation: 7,
            canonical_byte_limit: Default::default(),
        };
        let envelope = owner_envelope();
        assert!(validate_owner_carry(Some(&binding), &envelope).is_ok());
        assert!(validate_owner_carry(None, &envelope).is_err());
        let mut changed = envelope.clone();
        changed.runtime_generation += 1;
        // Generation is excluded from the semantic digest but must match launch authority.
        assert!(changed.validate().is_ok());
        assert!(validate_owner_carry(Some(&binding), &changed).is_err());
        changed = envelope.clone();
        changed.message.text.push('!');
        assert!(validate_owner_carry(Some(&binding), &changed).is_err());
        changed = envelope.clone();
        changed.owner_id = "other-owner".into();
        changed.semantic_sha256 = changed.digest();
        assert!(validate_owner_carry(Some(&binding), &changed).is_err());
        changed = envelope.clone();
        changed.thread_id = "other-thread".into();
        changed.semantic_sha256 = changed.digest();
        assert!(validate_owner_carry(Some(&binding), &changed).is_err());
        let canonical_size = serde_json::to_vec(&envelope.response_item().unwrap())
            .unwrap()
            .len();
        binding.canonical_byte_limit = codex_core::context::CanonicalBytePolicy::Limit(
            std::num::NonZeroU32::new(canonical_size as u32).unwrap(),
        );
        assert!(validate_owner_carry(Some(&binding), &envelope).is_ok());
        binding.canonical_byte_limit = codex_core::context::CanonicalBytePolicy::Limit(
            std::num::NonZeroU32::new(canonical_size as u32 - 1).unwrap(),
        );
        assert!(validate_owner_carry(Some(&binding), &envelope).is_err());
        binding.canonical_byte_limit = codex_core::context::CanonicalBytePolicy::Off;
        assert!(validate_owner_carry(Some(&binding), &envelope).is_ok());
    }

    #[test]
    fn busy_refusal_is_a_typed_invalid_request() {
        let error = busy_error(BusyReason::GateClosed);
        assert_eq!(error.code, crate::error_code::INVALID_REQUEST_ERROR_CODE);
        assert_eq!(
            error.message,
            "ExternalInput exclusive admission refused: busy (gate_closed)"
        );
        assert_eq!(
            error.data,
            Some(serde_json::json!({
                "externalInputError": "busy",
                "reason": "gate_closed",
            }))
        );
    }
}
