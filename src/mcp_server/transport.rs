use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use blazingly_json::{json, JsonCursor, Value};

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
        match mcport_route(&line) {
            Some(McportRoute::ToolCall { id, progress_token }) => {
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
                    if let Some(response) =
                        super::tools::handle_mcport_line(&server, &line, &context)
                    {
                        let _ = response_tx.send(response);
                    }
                }));
            }
            Some(McportRoute::Immediate) => {
                if let Some(response) =
                    super::tools::handle_mcport_line(&server, &line, &RequestContext::idle())
                {
                    output_tx
                        .send(response)
                        .map_err(|_| anyhow::anyhow!("stdout writer stopped"))?;
                }
            }
            None => {
                if let Some(response) = server.handle_line(&line, &RequestContext::idle()) {
                    output_tx
                        .send(response)
                        .map_err(|_| anyhow::anyhow!("stdout writer stopped"))?;
                }
            }
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

enum McportRoute {
    ToolCall {
        id: Value,
        progress_token: Option<Value>,
    },
    Immediate,
}

fn mcport_route(line: &str) -> Option<McportRoute> {
    let mut method = None;
    let mut id = None;
    let mut progress_token = None;
    let mut cursor = JsonCursor::from_str(line.trim_start_matches('\u{feff}'));
    cursor
        .object(|request| {
            while let Some(field) = request.next_field()? {
                match field.name() {
                    "method" => method = Some(field.deserialize::<String>()?),
                    "id" => id = Some(field.deserialize::<Value>()?),
                    "params" => field.object(|params| {
                        while let Some(field) = params.next_field()? {
                            if field.name() == "_meta" {
                                field.object(|metadata| {
                                    while let Some(field) = metadata.next_field()? {
                                        if field.name() == "progressToken" {
                                            progress_token = Some(field.deserialize::<Value>()?);
                                        } else {
                                            field.skip()?;
                                        }
                                    }
                                    Ok(())
                                })?;
                            } else {
                                field.skip()?;
                            }
                        }
                        Ok(())
                    })?,
                    _ => field.skip()?,
                }
            }
            Ok(())
        })
        .ok()?;
    cursor.end().ok()?;

    match method.as_deref()? {
        "tools/call" => Some(McportRoute::ToolCall {
            id: id?,
            progress_token: progress_token.filter(|value| value.is_string() || value.is_number()),
        }),
        "tools/list" | "resources/list" | "resources/templates/list" | "resources/read" => {
            Some(McportRoute::Immediate)
        }
        _ => None,
    }
}
