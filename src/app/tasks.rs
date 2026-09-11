//! Ownership of application effects for one event-loop session.
//!
//! Child effects inherit the session. Dropping it cancels every outstanding
//! async effect; completed handles are removed rather than accumulating.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

static BLOCKING_WORK: AtomicUsize = AtomicUsize::new(0);
struct BlockingWork;
impl Drop for BlockingWork {
    fn drop(&mut self) {
        BLOCKING_WORK.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Wait for accepted filesystem/CPU work after async effects have been cancelled.
/// Returns false on deadline so shutdown can report possible unsaved changes.
pub async fn finish_blocking(timeout: std::time::Duration) -> bool {
    let wait = async {
        while BLOCKING_WORK.load(Ordering::Acquire) != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(timeout, wait).await.is_ok()
}
use tokio::task::{AbortHandle, JoinHandle};

tokio::task_local! {
    static OWNER: Arc<Mutex<Registry>>;
}

#[derive(Default)]
struct Registry {
    failures: Option<tokio::sync::mpsc::Sender<super::Event>>,
    next_id: u64,
    closed: bool,
    tasks: HashMap<u64, AbortHandle>,
}

pub struct TaskSession(Arc<Mutex<Registry>>);

/// Cancels an operation when its last owning state value is replaced/dropped.
#[derive(Debug, Clone)]
pub struct TaskLease(Arc<AbortOnDrop>);

#[derive(Debug)]
struct AbortOnDrop(AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl TaskLease {
    pub fn new<T>(task: &JoinHandle<T>) -> Self {
        Self(Arc::new(AbortOnDrop(task.abort_handle())))
    }
    pub fn cancel(&self) {
        self.0 .0.abort();
    }
}

impl Default for TaskSession {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Registry::default())))
    }
}

impl TaskSession {
    pub fn new(failures: tokio::sync::mpsc::Sender<super::Event>) -> Self {
        Self(Arc::new(Mutex::new(Registry {
            failures: Some(failures),
            ..Registry::default()
        })))
    }
    pub async fn run<F: Future>(&self, future: F) -> F::Output {
        OWNER.scope(self.0.clone(), future).await
    }
}

impl Drop for TaskSession {
    fn drop(&mut self) {
        let mut registry = self.0.lock().unwrap_or_else(|p| p.into_inner());
        registry.closed = true;
        for (_, task) in registry.tasks.drain() {
            task.abort();
        }
    }
}

struct Registration {
    owner: Arc<Mutex<Registry>>,
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.owner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tasks
            .remove(&self.id);
    }
}

/// Same return contract as Tokio spawn, with session ownership propagated to
/// descendants. Standalone callers (including unit tests) use their runtime.
#[track_caller]
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let Ok(owner) = OWNER.try_with(Arc::clone) else {
        return tokio::spawn(future);
    };
    let mut registry = owner.lock().unwrap_or_else(|p| p.into_inner());
    let id = registry.next_id;
    registry.next_id = registry.next_id.wrapping_add(1);
    let registration = Registration {
        owner: owner.clone(),
        id,
    };
    let failures = registry.failures.clone();
    let location = std::panic::Location::caller();
    let task = tokio::spawn(OWNER.scope(owner.clone(), async move {
        use futures::FutureExt;
        let _registration = registration;
        match std::panic::AssertUnwindSafe(future).catch_unwind().await {
            Ok(output) => output,
            Err(panic) => {
                let message = format!("Background task started at {location} panicked");
                tracing::error!("{message}");
                if let Some(failures) = failures {
                    let _ = failures.send(super::Event::WorkerFailed(message)).await;
                }
                std::panic::resume_unwind(panic)
            }
        }
    }));
    if registry.closed {
        task.abort();
    } else {
        registry.tasks.insert(id, task.abort_handle());
    }
    task
}

/// Run blocking work off the runtime while supervising its completion.
/// Started blocking calls cannot be aborted; process shutdown bounds its wait.
pub fn spawn_blocking<F, R>(work: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    // Schedule immediately: accepted persistence work must survive cancellation
    // of its async observer during quit.
    BLOCKING_WORK.fetch_add(1, Ordering::AcqRel);
    let registration = BlockingWork;
    let work = tokio::task::spawn_blocking(move || {
        let _registration = registration;
        work()
    });
    spawn(async move {
        match work.await {
            Ok(value) => value,
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Err(error) => panic!("Blocking worker cancelled: {error}"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_cancels_descendants_and_reaps_completed_tasks() {
        let session = TaskSession::default();
        let (tx, rx) = tokio::sync::oneshot::channel();
        session
            .run(async {
                spawn(async move {
                    let child = spawn(std::future::pending::<()>());
                    let _ = tx.send(child);
                })
                .await
                .unwrap();
            })
            .await;
        let child = rx.await.unwrap();
        assert_eq!(session.0.lock().unwrap().tasks.len(), 1);
        drop(session);
        assert!(child.await.unwrap_err().is_cancelled());
    }
}
