//! Local control socket.
//!
//! Lets external scripts (e.g. the `sendgs` shell script) inject commands into a
//! running VellumFE client over a per-character Unix domain socket, WITHOUT
//! opening a second connection to Lich (which would disrupt the live game
//! session). A command received on the socket is forwarded to the main event
//! loop, which echoes it (marked as a remote command) and sends it on the single
//! existing game connection — exactly as if it had been typed into the command
//! input.
//!
//! Phase 2 (request/response): the main loop also captures the command's output
//! (text routed to the main window, up to the next game prompt) and returns it
//! over the same socket connection, so `sendgs look` prints the result directly.
//!
//! Loopback-only by construction: a Unix socket has no network surface, just
//! filesystem permissions. Gated behind the `--control` CLI flag (testbuild).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

/// How long to wait for the client to capture a command's output before giving
/// up (e.g. a command that never produces a prompt).
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

/// A command received on the control socket, plus a one-shot channel the main
/// loop uses to return the command's captured output (text up to the next
/// game prompt).
pub struct ControlRequest {
    pub command: String,
    pub responder: oneshot::Sender<String>,
}

/// Bind the control socket and service one request per connection.
///
/// Intended to be spawned as a background task. Runs until the listener errors.
pub async fn run(
    socket_path: PathBuf,
    control_tx: mpsc::UnboundedSender<ControlRequest>,
) -> Result<()> {
    // A stale socket file from a previous run would make bind() fail with
    // EADDRINUSE, so remove it first. (Harmless if it doesn't exist.)
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path)?;
    tracing::info!("Control socket listening at {:?}", socket_path);

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let tx = control_tx.clone();
                // One request per connection: read a line, inject it, return the
                // captured output, then close. (`sendgs` opens a fresh
                // connection per command.)
                tokio::spawn(async move {
                    let (read_half, mut write_half) = stream.into_split();
                    let mut lines = BufReader::new(read_half).lines();

                    let cmd = match lines.next_line().await {
                        Ok(Some(line)) => line.trim().to_string(),
                        _ => return, // client closed or read error
                    };
                    if cmd.is_empty() {
                        return;
                    }

                    let (resp_tx, resp_rx) = oneshot::channel();
                    if tx
                        .send(ControlRequest { command: cmd, responder: resp_tx })
                        .is_err()
                    {
                        return; // main loop is gone
                    }

                    let reply = match timeout(RESPONSE_TIMEOUT, resp_rx).await {
                        Ok(Ok(output)) => output,
                        Ok(Err(_)) => return, // responder dropped without sending
                        Err(_) => {
                            "(sendgs: timed out waiting for command output)\n".to_string()
                        }
                    };

                    let _ = write_half.write_all(reply.as_bytes()).await;
                    let _ = write_half.flush().await;
                });
            }
            Err(e) => {
                tracing::error!("Control socket accept error: {}", e);
                return Err(e.into());
            }
        }
    }
}
