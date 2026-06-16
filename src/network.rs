use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub enum ServerMessage {
    Text(String),
    Connected,
    Disconnected,
}

pub struct LichConnection;

impl LichConnection {
    pub async fn start(
        host: &str,
        port: u16,
        server_tx: mpsc::UnboundedSender<ServerMessage>,
        mut command_rx: mpsc::UnboundedReceiver<String>,
        feed_path: Option<std::path::PathBuf>,
    ) -> Result<()> {
        info!("Connecting to Lich at {}:{}...", host, port);

        // Retry the initial connect: on a restart, Lich needs a moment to tear
        // down the old session and re-open its detachable listener, so a single
        // attempt often loses the race. Retry with backoff for ~30s rather than
        // failing immediately (which left the client sitting disconnected until
        // a manual relaunch).
        let addr = format!("{}:{}", host, port);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let stream = loop {
            match TcpStream::connect(&addr).await {
                Ok(s) => break s,
                Err(e) => {
                    if std::time::Instant::now() >= deadline {
                        return Err(e).context("Failed to connect to Lich");
                    }
                    debug!("Connect attempt failed ({}); retrying in 500ms...", e);
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        };

        info!("Connected successfully");

        // Optional tee of the post-hook stream to ~/.vellum-fe/<char>/feed.log,
        // consumed by `sendgs`. A dedicated writer task owns the file so disk
        // I/O never stalls the read loop; the file is truncated on each connect.
        let feed_tx = feed_path.map(|path| {
            let (tx, mut rx) = mpsc::unbounded_channel::<String>();
            tokio::spawn(async move {
                let mut f = match tokio::fs::File::create(&path).await {
                    Ok(f) => f,
                    Err(e) => {
                        error!("Failed to open feed log {:?}: {}", path, e);
                        return;
                    }
                };
                // Cap the file so a long session can't grow unbounded; roll over
                // by truncating. `sendgs` notices the shrink and re-syncs from
                // the top, and since it tails the end the reset is invisible.
                use tokio::io::AsyncSeekExt;
                const FEED_CAP: u64 = 3 * 1024 * 1024;
                let mut written: u64 = 0;
                while let Some(line) = rx.recv().await {
                    if written > FEED_CAP {
                        if f.set_len(0).await.is_ok() {
                            let _ = f.seek(std::io::SeekFrom::Start(0)).await;
                            written = 0;
                        }
                    }
                    if f.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                    let _ = f.write_all(b"\n").await;
                    let _ = f.flush().await;
                    written += line.len() as u64 + 1;
                }
            });
            tx
        });

        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = BufReader::new(reader);

        // Send frontend PID
        let pid = std::process::id();
        let msg = format!("SET_FRONTEND_PID:{}\n", pid);
        writer.write_all(msg.as_bytes()).await?;
        writer.flush().await?;
        debug!("Sent frontend PID: {}", pid);

        let _ = server_tx.send(ServerMessage::Connected);

        // Spawn reader task
        let server_tx_clone = server_tx.clone();
        let read_handle = tokio::spawn(async move {
            let feed_tx = feed_tx;
            loop {
                let mut buf = Vec::new();
                match reader.read_until(b'\n', &mut buf).await {
                    Ok(0) => {
                        info!("Connection closed by server");
                        let _ = server_tx_clone.send(ServerMessage::Disconnected);
                        break;
                    }
                    Ok(n) => {
                        // Lossy conversion keeps all valid UTF-8 (including
                        // multi-byte chars) and replaces invalid bytes with
                        // U+FFFD, so one bad byte can't mangle the line
                        let line = match String::from_utf8(buf) {
                            Ok(line) => line,
                            Err(e) => {
                                debug!("Invalid UTF-8 in {} byte message, using lossy conversion", n);
                                String::from_utf8_lossy(&e.into_bytes()).into_owned()
                            }
                        };
                        // Strip only the trailing newline, preserve blank lines
                        let line = line.trim_end_matches(&['\r', '\n']);
                        if let Some(tx) = &feed_tx {
                            let _ = tx.send(line.to_string());
                        }
                        let _ = server_tx_clone.send(ServerMessage::Text(line.to_string()));
                    }
                    Err(e) => {
                        error!("Error reading from server: {}", e);
                        let _ = server_tx_clone.send(ServerMessage::Disconnected);
                        break;
                    }
                }
            }
        });

        // Writer task (runs in this function)
        // A write failure means commands are silently lost while the UI still
        // looks connected, so treat it as a disconnect rather than soldiering on
        let _write_result = async {
            while let Some(cmd) = command_rx.recv().await {
                debug!("Sending command: {}", cmd);
                if let Err(e) = writer.write_all(cmd.as_bytes()).await {
                    error!("Failed to write command: {}", e);
                    let _ = server_tx.send(ServerMessage::Disconnected);
                    break;
                }
                if let Err(e) = writer.write_all(b"\n").await {
                    error!("Failed to write newline: {}", e);
                    let _ = server_tx.send(ServerMessage::Disconnected);
                    break;
                }
                if let Err(e) = writer.flush().await {
                    error!("Failed to flush: {}", e);
                    let _ = server_tx.send(ServerMessage::Disconnected);
                    break;
                }
            }
        }
        .await;

        // Wait for reader to finish
        let _ = read_handle.await;

        Ok(())
    }
}
