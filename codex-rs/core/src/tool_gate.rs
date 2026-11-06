//! cutex fork: agent pause at the next tool boundary.
//!
//! cutex closes the gate of the bound thread while the agent is paused. Every
//! locally dispatched tool call then waits in `ToolRegistry::dispatch_any_with_state`
//! after its PreToolUse hooks and before it starts; a call already running
//! finishes. Opening the gate lets the waiting calls run in the same turn.
//! Interrupt cancels the call (`or_cancel_tool` drops the wait) and the tool is
//! not run. Provider-run tools (web search, image generation) are not dispatched
//! here and are not held. The gate lives in memory: cutex sets it again after
//! every reconnect.
//!
//! Sub-agent threads follow their root: a thread's calls wait on the gate of its
//! top-most loaded ancestor (resolved through `parent_thread_id` at each call, so
//! load order does not matter) and are listed there with their own thread id.
use crate::CodexThread;
use codex_protocol::ThreadId;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::Weak;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::sync::watch;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolGateState {
    pub held: bool,
    /// Calls waiting at the gate, oldest first (the thread's own and those of
    /// its sub-agent threads).
    pub waiting: Vec<HeldToolCall>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldToolCall {
    pub call_id: String,
    pub tool: String,
    pub turn_id: String,
    /// The thread the call belongs to (a sub-agent thread for its calls).
    pub thread_id: String,
    pub since_ms: i64,
}

type Shared = Arc<watch::Sender<ToolGateState>>;
type Registry = HashMap<ThreadId, (Weak<watch::Sender<ToolGateState>>, Option<ThreadId>)>;

/// Loaded gates by thread, with each thread's parent.
static GATES: LazyLock<Mutex<Registry>> = LazyLock::new(Default::default);
const MAX_ANCESTORS: usize = 32;

pub(crate) struct ToolGate {
    state: Shared,
    thread: Option<(ThreadId, Option<ThreadId>)>,
}

impl Default for ToolGate {
    /// An unregistered gate (tests): only its own state.
    fn default() -> Self {
        Self {
            state: Arc::new(watch::Sender::new(ToolGateState::default())),
            thread: None,
        }
    }
}

impl Drop for ToolGate {
    fn drop(&mut self) {
        let Some((thread_id, _)) = self.thread else {
            return;
        };
        if let Ok(mut gates) = GATES.lock()
            && gates
                .get(&thread_id)
                .is_some_and(|(gate, _)| gate.as_ptr() == Arc::as_ptr(&self.state))
        {
            gates.remove(&thread_id);
        }
    }
}

struct Waiting<'a> {
    gate: &'a watch::Sender<ToolGateState>,
    call_id: &'a str,
    thread_id: &'a str,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.gate.send_if_modified(|state| {
            let before = state.waiting.len();
            state
                .waiting
                .retain(|call| call.call_id != self.call_id || call.thread_id != self.thread_id);
            state.waiting.len() != before
        });
    }
}

impl ToolGate {
    /// The gate of a loaded thread; a sub-agent thread names its parent.
    pub(crate) fn register(thread_id: ThreadId, parent: Option<ThreadId>) -> Self {
        let mut gate = Self::default();
        gate.thread = Some((thread_id, parent));
        if let Ok(mut gates) = GATES.lock() {
            gates.insert(thread_id, (Arc::downgrade(&gate.state), parent));
        }
        gate
    }

    pub(crate) fn set(&self, held: bool) -> ToolGateState {
        self.state.send_if_modified(|state| {
            let changed = state.held != held;
            state.held = held;
            changed
        });
        self.snapshot()
    }

    pub(crate) fn snapshot(&self) -> ToolGateState {
        self.state.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<ToolGateState> {
        self.state.subscribe()
    }

    /// The gate this thread's calls wait on: its top-most loaded ancestor's.
    fn effective(&self) -> Shared {
        let mut effective = Arc::clone(&self.state);
        let Some((_, mut parent)) = self.thread else {
            return effective;
        };
        let Ok(gates) = GATES.lock() else {
            return effective;
        };
        for _ in 0..MAX_ANCESTORS {
            let Some(id) = parent else { break };
            let Some((gate, grandparent)) = gates.get(&id) else {
                break;
            };
            let Some(gate) = gate.upgrade() else { break };
            effective = gate;
            parent = *grandparent;
        }
        effective
    }

    /// Holds one call while the effective gate is closed. Dropping the future
    /// (cancel) removes the call from the waiting list.
    pub(crate) async fn wait(&self, call_id: &str, tool: &str, turn_id: &str) {
        let gate = self.effective();
        let mut changes = gate.subscribe();
        if !changes.borrow_and_update().held {
            return;
        }
        let thread_id = self
            .thread
            .map(|(thread_id, _)| thread_id.to_string())
            .unwrap_or_default();
        let since_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or_default();
        gate.send_modify(|state| {
            state.waiting.push(HeldToolCall {
                call_id: call_id.to_string(),
                tool: tool.to_string(),
                turn_id: turn_id.to_string(),
                thread_id: thread_id.clone(),
                since_ms,
            });
        });
        let _waiting = Waiting {
            gate: &gate,
            call_id,
            thread_id: &thread_id,
        };
        // The sender lives as long as its session; an error ends the wait.
        let _ = changes.wait_for(|state| !state.held).await;
    }
}

impl CodexThread {
    /// Closes (`true`) or opens the tool gate of this thread (and with it the
    /// gate its sub-agent threads wait on).
    pub fn set_tool_gate(&self, held: bool) -> ToolGateState {
        self.session.tool_gate.set(held)
    }

    pub fn subscribe_tool_gate(&self) -> watch::Receiver<ToolGateState> {
        self.session.tool_gate.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    const TIMEOUT: Duration = Duration::from_secs(5);

    async fn waiting(gate: &ToolGate, count: usize) {
        let mut changes = gate.subscribe();
        tokio::time::timeout(
            TIMEOUT,
            changes.wait_for(|state| state.waiting.len() == count),
        )
        .await
        .expect("waiting calls")
        .expect("gate alive");
    }

    #[tokio::test]
    async fn open_gate_does_not_wait() {
        let gate = ToolGate::default();
        tokio::time::timeout(TIMEOUT, gate.wait("c1", "shell", "t1"))
            .await
            .expect("no wait");
        assert_eq!(gate.snapshot(), ToolGateState::default());
    }

    #[tokio::test]
    async fn closed_gate_holds_calls_until_it_opens() {
        let gate = Arc::new(ToolGate::default());
        assert!(gate.set(true).held);
        let calls = ["c1", "c2"].map(|id| {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move { gate.wait(id, "exec_command", "t1").await })
        });
        waiting(&gate, 2).await;
        let held = gate.snapshot().waiting;
        assert_eq!(
            held.iter()
                .map(|call| call.call_id.as_str())
                .collect::<Vec<_>>(),
            ["c1", "c2"]
        );
        assert!(
            held.iter().all(|call| call.tool == "exec_command"
                && call.turn_id == "t1"
                && call.since_ms > 0)
        );
        assert!(calls.iter().all(|call| !call.is_finished()));
        gate.set(false);
        for call in calls {
            tokio::time::timeout(TIMEOUT, call).await.unwrap().unwrap();
        }
        assert_eq!(gate.snapshot(), ToolGateState::default());
    }

    #[tokio::test]
    async fn interrupt_ends_the_wait_and_the_gate_stays_closed() {
        let gate = Arc::new(ToolGate::default());
        gate.set(true);
        let cancel = CancellationToken::new();
        let call = {
            let (gate, cancel) = (Arc::clone(&gate), cancel.clone());
            tokio::spawn(async move {
                tokio::select! {
                    _ = gate.wait("c1", "apply_patch", "t1") => true,
                    _ = cancel.cancelled() => false,
                }
            })
        };
        waiting(&gate, 1).await;
        cancel.cancel();
        assert!(!tokio::time::timeout(TIMEOUT, call).await.unwrap().unwrap());
        assert_eq!(
            gate.snapshot(),
            ToolGateState {
                held: true,
                waiting: Vec::new()
            }
        );
    }

    #[tokio::test]
    async fn a_dropped_wait_leaves_no_entry() {
        let gate = Arc::new(ToolGate::default());
        gate.set(true);
        let call = {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move { gate.wait("c1", "mcp", "t1").await })
        };
        waiting(&gate, 1).await;
        call.abort();
        let _ = call.await;
        assert!(gate.snapshot().waiting.is_empty());
    }

    #[tokio::test]
    async fn sub_agent_threads_wait_on_their_root_gate() {
        let (root_id, child_id, grandchild_id) =
            (ThreadId::new(), ThreadId::new(), ThreadId::new());
        // A grandchild loaded before its parent still finds the root later.
        let grandchild = Arc::new(ToolGate::register(grandchild_id, Some(child_id)));
        let root = ToolGate::register(root_id, None);
        let child = Arc::new(ToolGate::register(child_id, Some(root_id)));
        root.set(true);
        let calls = [(&child, "child-call"), (&grandchild, "grandchild-call")].map(|(gate, id)| {
            let gate = Arc::clone(gate);
            tokio::spawn(async move { gate.wait(id, "exec_command", "child-turn").await })
        });
        waiting(&root, 2).await;
        let mut held: Vec<_> = root
            .snapshot()
            .waiting
            .into_iter()
            .map(|call| (call.call_id, call.thread_id))
            .collect();
        held.sort();
        assert_eq!(
            held,
            [
                ("child-call".to_string(), child_id.to_string()),
                ("grandchild-call".to_string(), grandchild_id.to_string())
            ]
        );
        assert!(
            child.snapshot().waiting.is_empty(),
            "listed on the root gate only"
        );
        assert!(calls.iter().all(|call| !call.is_finished()));
        root.set(false);
        for call in calls {
            tokio::time::timeout(TIMEOUT, call).await.unwrap().unwrap();
        }
        assert!(root.snapshot().waiting.is_empty());
        // An unloaded root no longer holds its former sub-agent threads.
        root.set(true);
        drop(root);
        tokio::time::timeout(TIMEOUT, child.wait("after-unload", "shell", "t"))
            .await
            .expect("root gone");
    }
}
