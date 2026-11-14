//! Engine child-process client.
//!
//! One child process, one reader task (stdout -> responses/progress), one stderr task
//! (-> `tracing::debug!` + last-20-lines ring). Calls are multiplexed by request id, so
//! several can be in flight; the engine itself processes them sequentially.
//!
//! Cancellation kills the engine process tree (protocol: cancellation = kill; chunk parts
//! make the work resumable). After a kill/crash the client is dead: every later call fails
//! fast with a retryable `Internal` error and the caller should `spawn` a new client.

use crate::protocol::{self, Incoming, STAGE};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{ProgressEvent, ProgressSink};
use rustler_tools::ToolPaths;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use tokio_util::sync::CancellationToken;

const STDERR_TAIL_LINES: usize = 20;

type Outcome = Result<Value, RustlerError>;

struct Pending {
    tx: oneshot::Sender<Outcome>,
    progress: ProgressSink,
}

#[derive(Default)]
struct State {
    pending: HashMap<u64, Pending>,
    /// Set once the engine's stdout closed; later calls fail with this error.
    closed: Option<RustlerError>,
}

struct Shared {
    state: Mutex<State>,
    stderr_tail: Mutex<VecDeque<String>>,
    child: AsyncMutex<Child>,
    /// The engine was killed on purpose (cancellation / `kill`); changes the wording only.
    killed_on_purpose: AtomicBool,
    /// Reader saw EOF and reaped the child.
    exited: AtomicBool,
}

impl Shared {
    fn tail_text(&self) -> String {
        let t = self.stderr_tail.lock().unwrap();
        t.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    fn crash_error(&self, status: &str) -> RustlerError {
        let tail = self.tail_text();
        let head = if self.killed_on_purpose.load(Ordering::SeqCst) {
            format!("engine stopped ({status}); it was killed after a cancel/kill request")
        } else {
            format!("engine crashed ({status})")
        };
        let msg = if tail.is_empty() {
            head
        } else {
            format!("{head}; last stderr lines:\n{tail}")
        };
        RustlerError::new(ErrorCode::Internal, STAGE, msg)
            .hint("retry the command; transcription resumes from saved chunk parts")
            .retryable()
    }
}

/// Child process, pending-request map and reader task.
pub struct EngineClient {
    inner: Arc<Inner>,
}

struct Inner {
    shared: Arc<Shared>,
    stdin: AsyncMutex<Option<ChildStdin>>,
    next_id: AtomicU64,
    pid: Option<u32>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(pid) = self.pid
            && !self.shared.exited.load(Ordering::SeqCst)
        {
            kill_tree_blocking(pid);
        }
    }
}

impl EngineClient {
    /// `<venv_python> -m rustler_engine`, cwd `engine_dir`, env `HF_HOME` (= `models/hf`);
    /// `DependencyMissing` if the venv or the engine package is missing.
    pub async fn spawn(paths: &ToolPaths) -> rustler_core::Result<Self> {
        let missing = |what: &str, p: &Path| {
            RustlerError::new(
                ErrorCode::DependencyMissing,
                STAGE,
                format!("engine {what} not found at {}", p.display()),
            )
            .hint("run `rustler setup`")
        };
        if !paths.venv_python.is_file() {
            return Err(missing("python (venv)", &paths.venv_python));
        }
        if !paths.engine_dir.join("rustler_engine").is_dir() {
            return Err(missing("package", &paths.engine_dir));
        }
        let env = vec![(
            "HF_HOME".to_string(),
            paths.models.join("hf").to_string_lossy().into_owned(),
        )];
        Self::spawn_with(&paths.venv_python, &paths.engine_dir, &env).await
    }

    /// Spawn with an explicit interpreter/engine dir: `<python> -m rustler_engine`.
    pub async fn spawn_with(
        python: &Path,
        engine_dir: &Path,
        env: &[(String, String)],
    ) -> rustler_core::Result<Self> {
        let args = vec!["-m".to_string(), "rustler_engine".to_string()];
        Self::spawn_command(python, &args, engine_dir, env).await
    }

    /// Spawn an arbitrary command that speaks the engine protocol on stdio
    /// (tests use `uv run --project <engine> python -m rustler_engine`).
    pub async fn spawn_command(
        program: &Path,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
    ) -> rustler_core::Result<Self> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in env {
            cmd.env(k, v);
        }
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd.spawn().map_err(|e| {
            let code = if e.kind() == std::io::ErrorKind::NotFound {
                ErrorCode::DependencyMissing
            } else {
                ErrorCode::Internal
            };
            let mut err = RustlerError::new(
                code,
                STAGE,
                format!("cannot start engine {}: {e}", program.display()),
            );
            if code == ErrorCode::DependencyMissing {
                err = err.hint("run `rustler setup`");
            }
            err
        })?;
        let pid = child.id();
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            stderr_tail: Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES + 1)),
            child: AsyncMutex::new(child),
            killed_on_purpose: AtomicBool::new(false),
            exited: AtomicBool::new(false),
        });

        // stderr -> tracing + ring buffer
        let sh = shared.clone();
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "rustler_engine", "{line}");
                let mut t = sh.stderr_tail.lock().unwrap();
                if t.len() >= STDERR_TAIL_LINES {
                    t.pop_front();
                }
                t.push_back(line);
            }
        });

        // stdout reader: dispatch responses/progress; on EOF fail everything pending.
        let sh = shared.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => dispatch_line(&sh, &line),
                    Ok(None) => break,
                    Err(e) => {
                        tracing::warn!("engine stdout read error: {e}");
                        break;
                    }
                }
            }
            // Let the stderr task drain so the crash message has the final lines.
            let _ = tokio::time::timeout(Duration::from_secs(1), stderr_task).await;
            let status = {
                let mut child = sh.child.lock().await;
                match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                    Ok(Ok(s)) => s.to_string(),
                    Ok(Err(e)) => format!("wait failed: {e}"),
                    Err(_) => {
                        let _ = child.start_kill();
                        "did not exit".to_string()
                    }
                }
            };
            sh.exited.store(true, Ordering::SeqCst);
            let err = sh.crash_error(&format!("exit: {status}"));
            let drained: Vec<Pending> = {
                let mut st = sh.state.lock().unwrap();
                st.closed = Some(err.clone());
                st.pending.drain().map(|(_, p)| p).collect()
            };
            for p in drained {
                let _ = p.tx.send(Err(err.clone()));
            }
        });

        Ok(Self {
            inner: Arc::new(Inner {
                shared,
                stdin: AsyncMutex::new(Some(stdin)),
                next_id: AtomicU64::new(1),
                pid,
            }),
        })
    }

    /// Child exit mid-call -> `Internal` "engine crashed" + last stderr lines, retryable.
    /// `cancel` firing kills the engine and returns `Cancelled`.
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> rustler_core::Result<R> {
        let params = serde_json::to_value(params).map_err(|e| {
            RustlerError::new(
                ErrorCode::Validation,
                STAGE,
                format!("{method}: cannot serialize params: {e}"),
            )
        })?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let shared = &self.inner.shared;
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut st = shared.state.lock().unwrap();
            if let Some(err) = &st.closed {
                return Err(err.clone());
            }
            st.pending.insert(
                id,
                Pending {
                    tx,
                    progress: progress.clone(),
                },
            );
        }

        let req = protocol::Request {
            jsonrpc: "2.0",
            id,
            method,
            params: &params,
        };
        let mut line = serde_json::to_vec(&req).expect("request serializes");
        line.push(b'\n');
        {
            let mut guard = self.inner.stdin.lock().await;
            let write = async {
                match guard.as_mut() {
                    Some(stdin) => {
                        stdin.write_all(&line).await?;
                        stdin.flush().await
                    }
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "engine stdin closed",
                    )),
                }
            };
            if let Err(e) = write.await {
                shared.state.lock().unwrap().pending.remove(&id);
                tracing::debug!("engine write failed: {e}");
                // Give the reader a moment to observe the exit so the message is complete.
                tokio::time::sleep(Duration::from_millis(100)).await;
                return Err(shared
                    .state
                    .lock()
                    .unwrap()
                    .closed
                    .clone()
                    .unwrap_or_else(|| shared.crash_error(&format!("write failed: {e}"))));
            }
        }

        let outcome = tokio::select! {
            r = rx => match r {
                Ok(o) => o,
                // Sender dropped without a value: treat as a crash.
                Err(_) => Err(shared.crash_error("connection lost")),
            },
            _ = cancel.cancelled() => {
                shared.state.lock().unwrap().pending.remove(&id);
                self.kill().await;
                return Err(cancelled());
            }
        };
        let value = outcome?;
        serde_json::from_value(value).map_err(|e| {
            RustlerError::new(
                ErrorCode::Internal,
                STAGE,
                format!("{method}: unexpected result shape: {e}"),
            )
        })
    }

    /// Kills the engine process tree (what cancellation does). Pending calls fail with a
    /// retryable `Internal` error; the client cannot be used afterwards.
    pub async fn kill(&self) {
        self.inner
            .shared
            .killed_on_purpose
            .store(true, Ordering::SeqCst);
        if let Some(pid) = self.inner.pid {
            let _ = tokio::task::spawn_blocking(move || kill_tree_blocking(pid)).await;
        }
        let _ = self
            .inner
            .shared
            .child
            .try_lock()
            .map(|mut c| c.start_kill());
    }

    /// True while the engine process has not exited.
    pub async fn is_alive(&self) -> bool {
        if self.inner.shared.exited.load(Ordering::SeqCst) {
            return false;
        }
        let mut child = self.inner.shared.child.lock().await;
        matches!(child.try_wait(), Ok(None))
    }

    /// OS process id of the spawned child (the launcher, for `uv run`).
    pub fn pid(&self) -> Option<u32> {
        self.inner.pid
    }

    /// Last (up to 20) stderr lines of the engine, oldest first.
    pub fn stderr_tail(&self) -> Vec<String> {
        self.inner
            .shared
            .stderr_tail
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect()
    }

    /// Clean shutdown: close stdin (EOF = exit 0), wait up to 5 s, then kill the tree.
    pub async fn shutdown(self) {
        self.inner.stdin.lock().await.take();
        let shared = self.inner.shared.clone();
        let exited = async {
            let mut child = shared.child.lock().await;
            child.wait().await
        };
        if tokio::time::timeout(Duration::from_secs(5), exited)
            .await
            .is_err()
        {
            self.kill().await;
        }
    }
}

fn cancelled() -> RustlerError {
    RustlerError::new(ErrorCode::Cancelled, STAGE, "cancelled")
}

fn dispatch_line(sh: &Shared, line: &str) {
    if line.trim().is_empty() {
        return;
    }
    match protocol::parse_line(line) {
        Ok(Incoming::Response { id, outcome }) => {
            let Some(id) = id else {
                // e.g. a parse error reported with id null: nothing to route it to.
                tracing::warn!("engine response without id: {line}");
                return;
            };
            let pending = sh.state.lock().unwrap().pending.remove(&id);
            if let Some(p) = pending {
                let res = outcome.map_err(|e| protocol::map_error("engine", e));
                let _ = p.tx.send(res);
            } else {
                tracing::debug!("engine response for unknown/cancelled id {id}");
            }
        }
        Ok(Incoming::Progress(p)) => {
            let sink = p.id.and_then(|id| {
                sh.state
                    .lock()
                    .unwrap()
                    .pending
                    .get(&id)
                    .map(|x| x.progress.clone())
            });
            if let Some(sink) = sink {
                sink.emit(ProgressEvent {
                    stage: p.stage,
                    pct: p.pct.map(|v| (v / 100.0).clamp(0.0, 1.0) as f32),
                    message: p.message,
                });
            }
        }
        Ok(Incoming::Other(v)) => tracing::debug!("engine sent unrecognized message: {v}"),
        Err(e) => tracing::warn!("engine stdout line is not JSON ({e}): {line}"),
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Kills `pid` and all its descendants (the uv/venv launchers spawn the real interpreter).
fn kill_tree_blocking(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // The child leads its own process group (see `process_group(0)`).
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
