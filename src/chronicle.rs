//! Process-local controller for the durable RadioChron change journal.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use radiochron::chronicle::{
    read_recent_jsonl, JsonlSink, Recorder, RecorderOptions, RotationPolicy,
};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct ChronicleService {
    control: Arc<Mutex<Option<Worker>>>,
    status: Arc<Mutex<Status>>,
    path: Arc<PathBuf>,
}

struct Worker {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
struct Status {
    running: bool,
    started_at_epoch_seconds: Option<i64>,
    stopped_at_epoch_seconds: Option<i64>,
    entries_written: usize,
    last_error: Option<String>,
}

impl ChronicleService {
    pub fn new() -> Self {
        let path = std::env::var_os("RADIOCHRON_CHRONICLE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let base = std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir);
                base.join("RadioChron").join("chronicle.jsonl")
            });
        Self::with_path(path)
    }

    fn with_path(path: PathBuf) -> Self {
        Self {
            control: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(Status::default())),
            path: Arc::new(path),
        }
    }

    pub fn start(&self, interval: Duration, threshold_db: i32) -> anyhow::Result<Value> {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if control.is_some() {
            return Ok(self.status());
        }

        let sink = JsonlSink::open(&*self.path, RotationPolicy::default())?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_status = self.status.clone();
        let options = RecorderOptions {
            interval,
            signal_threshold_db: threshold_db,
        };

        {
            let mut current = self.status.lock().unwrap_or_else(|e| e.into_inner());
            current.running = true;
            current.started_at_epoch_seconds = Some(radiochron::time::now_epoch_seconds());
            current.stopped_at_epoch_seconds = None;
            current.entries_written = 0;
            current.last_error = None;
        }

        let handle = match std::thread::Builder::new()
            .name("radiochron-recorder".to_string())
            .spawn(move || {
                let mut recorder = Recorder::new(sink, options);
                while !worker_stop.load(Ordering::Acquire) {
                    match recorder.step() {
                        Ok(written) => {
                            let mut current =
                                worker_status.lock().unwrap_or_else(|e| e.into_inner());
                            current.entries_written =
                                current.entries_written.saturating_add(written);
                            current.last_error = None;
                        }
                        Err(error) => {
                            worker_status
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .last_error = Some(error.to_string());
                        }
                    }
                    std::thread::park_timeout(interval);
                }

                let mut current = worker_status.lock().unwrap_or_else(|e| e.into_inner());
                current.running = false;
                current.stopped_at_epoch_seconds = Some(radiochron::time::now_epoch_seconds());
            }) {
            Ok(handle) => handle,
            Err(error) => {
                let mut current = self.status.lock().unwrap_or_else(|e| e.into_inner());
                current.running = false;
                current.stopped_at_epoch_seconds = Some(radiochron::time::now_epoch_seconds());
                current.last_error = Some(error.to_string());
                return Err(error.into());
            }
        };

        *control = Some(Worker { stop, handle });
        drop(control);
        Ok(self.status())
    }

    pub fn stop(&self) -> anyhow::Result<Value> {
        let worker = self
            .control
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(worker) = worker {
            worker.stop.store(true, Ordering::Release);
            worker.handle.thread().unpark();
            if worker.handle.join().is_err() {
                let mut current = self.status.lock().unwrap_or_else(|e| e.into_inner());
                current.running = false;
                current.stopped_at_epoch_seconds = Some(radiochron::time::now_epoch_seconds());
                current.last_error = Some("chronicle worker panicked".to_string());
                anyhow::bail!("chronicle worker panicked");
            }
        }
        Ok(self.status())
    }

    pub fn status(&self) -> Value {
        let current = self.status.lock().unwrap_or_else(|e| e.into_inner());
        json!({
            "running": current.running,
            "path": self.path.to_string_lossy(),
            "started_at_epoch_seconds": current.started_at_epoch_seconds,
            "stopped_at_epoch_seconds": current.stopped_at_epoch_seconds,
            "entries_written_this_run": current.entries_written,
            "last_error": current.last_error,
        })
    }

    pub fn recent(&self, max: usize) -> anyhow::Result<Value> {
        let max = max.clamp(1, 1000);
        let read = read_recent_jsonl(&self.path, RotationPolicy::default().max_files, max)?;
        Ok(json!({
            "path": self.path.to_string_lossy(),
            "count": read.entries.len(),
            "invalid_lines": read.invalid_lines,
            "entries": read.entries,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_reads_rotated_files_in_chronological_order() {
        let dir =
            std::env::temp_dir().join(format!("radiochron-mcp-chronicle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chronicle.jsonl");
        let mut rotated = path.as_os_str().to_os_string();
        rotated.push(".1");
        std::fs::write(PathBuf::from(rotated), "{\"n\":1}\n{\"n\":2}\n").unwrap();
        std::fs::write(&path, "{\"n\":3}\n").unwrap();

        let service = ChronicleService::with_path(path);
        let result = service.recent(2).unwrap();
        assert_eq!(result["entries"][0]["n"], 2);
        assert_eq!(result["entries"][1]["n"], 3);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
