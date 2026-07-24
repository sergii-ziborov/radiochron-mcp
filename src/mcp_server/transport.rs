use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use serde_json::{json, Value};

use super::protocol::{RegisteredRequest, Server};

pub(crate) struct RequestContext {
    cancelled: Arc<AtomicBool>,
    progress_token: Option<Value>,
    output: Option<mpsc::Sender<String>>,
}

impl RequestContext {
    pub(crate) fn idle() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            progress_token: None,
            output: None,
        }
    }

    pub(crate) fn check_cancelled(&self) -> anyhow::Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            anyhow::bail!("request cancelled by client");
        }
        Ok(())
    }

    pub(crate) fn progress(&self, progress: u128, total: u128, message: &str) {
        let (Some(token), Some(output)) = (&self.progress_token, &self.output) else {
            return;
        };
        let _ = output.send(
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/progress",
                "params": {
                    "progressToken": token,
                    "progress": progress,
                    "total": total,
                    "message": message
                }
            })
            .to_string(),
        );
    }
}

pub fn serve_stdio() -> anyhow::Result<()> {
    let server = Arc::new(Server::new());
    let (output_tx, output_rx) = mpsc::channel::<String>();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        for frame in output_rx {
            writeln!(stdout, "{frame}")?;
            stdout.flush()?;
        }
        Ok(())
    });

    let stdin = std::io::stdin();
    let mut workers = Vec::new();
    let mut request_worker_panicked = false;
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        request_worker_panicked |= reap_finished_workers(&mut workers);
        if let Some((id, progress_token)) = tool_call_metadata(&line) {
            let server = server.clone();
            let response_tx = output_tx.clone();
            let progress_tx = output_tx.clone();
            let cancelled = server.register_request(&id);
            workers.push(std::thread::spawn(move || {
                let _registration = RegisteredRequest {
                    server: server.clone(),
                    id,
                };
                let context = RequestContext {
                    cancelled,
                    progress_token,
                    output: Some(progress_tx),
                };
                if let Some(response) = server.handle_line(&line, &context) {
                    let _ = response_tx.send(response);
                }
            }));
        } else if let Some(response) = server.handle_line(&line, &RequestContext::idle()) {
            output_tx
                .send(response)
                .map_err(|_| anyhow::anyhow!("stdout writer stopped"))?;
        }
    }

    for worker in workers {
        request_worker_panicked |= worker.join().is_err();
    }
    let _ = server.chronicle.stop();
    drop(output_tx);
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("stdout writer panicked"))??;
    if request_worker_panicked {
        anyhow::bail!("an MCP request worker panicked");
    }
    Ok(())
}

fn reap_finished_workers(workers: &mut Vec<std::thread::JoinHandle<()>>) -> bool {
    let mut panicked = false;
    let mut index = 0;
    while index < workers.len() {
        if workers[index].is_finished() {
            panicked |= workers.swap_remove(index).join().is_err();
        } else {
            index += 1;
        }
    }
    panicked
}

fn tool_call_metadata(line: &str) -> Option<(Value, Option<Value>)> {
    let message: Value = serde_json::from_str(line.trim_start_matches('\u{feff}')).ok()?;
    if message.get("method")?.as_str()? != "tools/call" {
        return None;
    }
    let id = message.get("id")?.clone();
    let progress_token = message
        .pointer("/params/_meta/progressToken")
        .filter(|value| value.is_string() || value.is_number())
        .cloned();
    Some((id, progress_token))
}
