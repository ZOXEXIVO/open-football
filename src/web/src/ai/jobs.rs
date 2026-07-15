use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use tokio::time::{Duration, timeout};

/// One tool the agent chose to call, surfaced live to the dialog so the
/// operator can watch the model work.
#[derive(Serialize, Clone)]
pub struct ToolTrace {
    pub name: String,
    pub arguments: String,
}

/// Mutable server-side state of a single agent run.
struct JobState {
    status: &'static str,
    tool_calls: Vec<ToolTrace>,
    text: String,
    detail: String,
}

/// Snapshot handed back to a long-poll request: the run status plus any tool
/// calls made since the client's `cursor`, and the final text once done.
#[derive(Serialize)]
pub struct JobSnapshot {
    pub status: &'static str,
    pub cursor: usize,
    pub new_tool_calls: Vec<ToolTrace>,
    pub text: String,
    pub detail: String,
}

/// Process-wide registry of in-flight AI agent runs. Cloneable, Arc-backed
/// (mirrors `AiConfig`); a run reports progress through an `AiJobHandle` and
/// the squad dialog long-polls `wait()` to render tool calls in real time.
#[derive(Clone, Default)]
pub struct AiJobs {
    inner: Arc<Mutex<HashMap<u64, JobState>>>,
    notify: Arc<Notify>,
    counter: Arc<AtomicU64>,
}

impl AiJobs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a fresh running job and return the handle the agent updates.
    pub fn create(&self) -> AiJobHandle {
        let id = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        {
            let mut map = self.inner.lock().unwrap();
            // Bound memory: drop finished jobs once the map grows.
            if map.len() > 32 {
                map.retain(|_, job| job.status == "running");
            }
            map.insert(
                id,
                JobState {
                    status: "running",
                    tool_calls: Vec::new(),
                    text: String::new(),
                    detail: String::new(),
                },
            );
        }
        AiJobHandle {
            jobs: self.clone(),
            id,
        }
    }

    fn mutate(&self, id: u64, f: impl FnOnce(&mut JobState)) {
        {
            let mut map = self.inner.lock().unwrap();
            if let Some(job) = map.get_mut(&id) {
                f(job);
            }
        }
        self.notify.notify_waiters();
    }

    fn snapshot(&self, id: u64, cursor: usize) -> Option<JobSnapshot> {
        let map = self.inner.lock().unwrap();
        let job = map.get(&id)?;
        let new_tool_calls = job.tool_calls.get(cursor..).unwrap_or(&[]).to_vec();
        Some(JobSnapshot {
            status: job.status,
            cursor: job.tool_calls.len(),
            new_tool_calls,
            text: job.text.clone(),
            detail: job.detail.clone(),
        })
    }

    /// Long-poll: resolve as soon as the job has progressed past `cursor`
    /// (more tool calls) or finished; otherwise hold for up to ~20s (checking
    /// every 500 ms so a lost notify can't stall the client) then return the
    /// current snapshot so the client re-polls. `None` if the job is unknown.
    pub async fn wait(&self, id: u64, cursor: usize) -> Option<JobSnapshot> {
        for _ in 0..40 {
            let snap = self.snapshot(id, cursor)?;
            if snap.status != "running" || snap.cursor > cursor {
                return Some(snap);
            }
            let _ = timeout(Duration::from_millis(500), self.notify.notified()).await;
        }
        self.snapshot(id, cursor)
    }
}

/// Writer handle for one job, held by the spawned agent task.
pub struct AiJobHandle {
    jobs: AiJobs,
    id: u64,
}

impl AiJobHandle {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Record a tool call the model just made (streamed to the dialog).
    pub fn push_tool(&self, name: String, arguments: String) {
        self.jobs
            .mutate(self.id, |job| job.tool_calls.push(ToolTrace { name, arguments }));
    }

    pub fn finish(&self, text: String) {
        self.jobs.mutate(self.id, |job| {
            job.status = "done";
            job.text = text;
        });
    }

    pub fn fail(&self, detail: String) {
        self.jobs.mutate(self.id, |job| {
            job.status = "error";
            job.detail = detail;
        });
    }
}
