//! Debug-build synchronization for the owned persistence fault/crash probe.
//! The environment variable is supplied by the private process launcher, never
//! by an RPC or model tool. No hook is compiled into release builds.
use anyhow::Result;
use codex_thread_store::LiveThread;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

pub(crate) async fn barrier(live: &LiveThread, phase: &str, message_id: &str) -> Result<()> {
    let Some(path) = std::env::var_os("CODEX_PRIVATE_EXTERNAL_INPUT_PROBE_SOCKET") else {
        return Ok(());
    };
    // Establish the exact pre-pair file boundary for the OS file-size fault.
    if phase == "before_pair" {
        live.flush().await?;
    }
    let mut socket = tokio::net::UnixStream::connect(path).await?;
    let mut event = serde_json::to_vec(&serde_json::json!({
        "phase": phase,
        "messageId": message_id,
    }))?;
    event.push(b'\n');
    socket.write_all(&event).await?;
    anyhow::ensure!(
        socket.read_u8().await? == 1,
        "private probe barrier aborted"
    );
    Ok(())
}
