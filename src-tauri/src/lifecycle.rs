//! Owns background work and coordinates application exit. Domain state and
//! cancellation policy remain with the services that use it.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::{oneshot, Notify};
use tokio::task::{AbortHandle, JoinError, JoinHandle};
use tokio::time::Instant;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone, Copy)]
pub(crate) enum TaskStage {
    Work,
    Service,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Stage {
    Work,
    Service,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Running,
    Stopping,
    Finished,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Stopping;

impl fmt::Display for Stopping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("application is shutting down")
    }
}

impl std::error::Error for Stopping {}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TaskError {
    #[error(transparent)]
    Join(#[from] JoinError),
    #[error("task completion observer is unavailable")]
    ObserverUnavailable,
    #[error("task completion was already received")]
    AlreadyJoined,
}

pub(crate) struct TaskHandle<T> {
    result: Option<oneshot::Receiver<Result<T, JoinError>>>,
    joined: Arc<AtomicBool>,
}

impl<T> TaskHandle<T> {
    // Unlike AbortHandle::is_finished, this confirms the owner has joined the task.
    pub(crate) fn is_finished(&self) -> bool {
        self.joined.load(Ordering::Acquire)
    }

    // Joining with a mutable borrow means a timeout does not consume the handle.
    pub(crate) async fn join(&mut self) -> Result<T, TaskError> {
        let result = self.result.as_mut().ok_or(TaskError::AlreadyJoined)?.await;
        self.result = None;
        Ok(result.map_err(|_| TaskError::ObserverUnavailable)??)
    }
}

struct Pending {
    stage: Stage,
    label: String,
    safe_abort: Option<AbortHandle>,
}

struct State {
    phase: Phase,
    can_finalize: bool,
    next_id: u64,
    pending: BTreeMap<u64, Pending>,
    scopes: BTreeMap<(Stage, &'static str), CancellationToken>,
    failures: Vec<String>,
}

struct Inner {
    state: Mutex<State>,
    changed: Notify,
    observers: TaskTracker,
    runtime: Option<Handle>,
}

#[derive(Clone)]
pub(crate) struct AppLifecycle(Arc<Inner>);

impl AppLifecycle {
    pub(crate) fn new() -> Self {
        Self::from_runtime(None)
    }

    pub(crate) fn with_runtime(runtime: Handle) -> Self {
        Self::from_runtime(Some(runtime))
    }

    fn from_runtime(runtime: Option<Handle>) -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new(State {
                phase: Phase::Running,
                can_finalize: true,
                next_id: 0,
                pending: BTreeMap::new(),
                scopes: BTreeMap::new(),
                failures: Vec::new(),
            }),
            changed: Notify::new(),
            observers: TaskTracker::new(),
            runtime,
        }))
    }

    pub(crate) fn scope(&self, stage: TaskStage, domain: &'static str) -> TaskScope {
        let stage = match stage {
            TaskStage::Work => Stage::Work,
            TaskStage::Service => Stage::Service,
        };
        let mut state = self.0.state.lock().unwrap();
        let token = match state.phase {
            Phase::Running => state.scopes.entry((stage, domain)).or_default().clone(),
            // An existing service scope must remain live while Work drains.
            _ => state
                .scopes
                .get(&(stage, domain))
                .cloned()
                .unwrap_or_else(|| {
                    let token = CancellationToken::new();
                    token.cancel();
                    token
                }),
        };
        TaskScope {
            app: self.clone(),
            stage,
            domain,
            token,
        }
    }

    pub(crate) fn phase(&self) -> Phase {
        self.0.state.lock().unwrap().phase
    }

    pub(crate) fn try_begin_shutdown(&self, budget: Duration) -> Option<Shutdown> {
        let started = Instant::now();
        let real_deadline = std::time::Instant::now() + budget;
        let work_tokens = {
            let mut state = self.0.state.lock().unwrap();
            if state.phase != Phase::Running {
                return None;
            }
            state.phase = Phase::Stopping;
            Self::tokens(&state, Stage::Work)
        };
        for token in work_tokens {
            token.cancel();
        }
        Some(Shutdown {
            app: self.clone(),
            work_grace: started + budget.mul_f64(0.60),
            work_limit: started + budget.mul_f64(0.75),
            service_grace: started + budget.mul_f64(0.90),
            async_limit: started + budget.mul_f64(0.95),
            real_deadline,
        })
    }

    fn tokens(state: &State, stage: Stage) -> Vec<CancellationToken> {
        state
            .scopes
            .iter()
            .filter(|((task_stage, _), _)| *task_stage == stage)
            .map(|(_, token)| token.clone())
            .collect()
    }

    fn runtime(&self) -> Handle {
        self.0.runtime.clone().unwrap_or_else(Handle::current)
    }

    // The ledger entry outlives the worker future: only its observer removes
    // the entry, after Tokio confirms completion, cancellation or panic.
    fn observe<T: Send + 'static>(
        &self,
        state: &mut State,
        stage: Stage,
        label: String,
        worker: JoinHandle<T>,
        safe_abort: bool,
    ) -> TaskHandle<T> {
        let id = state.next_id;
        state.next_id += 1;
        state.pending.insert(
            id,
            Pending {
                stage,
                label: label.clone(),
                safe_abort: safe_abort.then(|| worker.abort_handle()),
            },
        );
        let app = self.clone();
        let (sender, result) = oneshot::channel();
        let joined = Arc::new(AtomicBool::new(false));
        let worker_joined = joined.clone();
        self.0.observers.spawn_on(
            async move {
                let outcome = worker.await;
                let failure = outcome
                    .as_ref()
                    .err()
                    .filter(|error| error.is_panic())
                    .map(|error| format!("{label}: {error}"));
                app.remove(id, failure.clone());
                worker_joined.store(true, Ordering::Release);
                let _ = sender.send(outcome);
                if let Some(failure) = failure {
                    tracing::error!(target: "lifecycle", %failure, "background task panicked");
                }
            },
            &self.runtime(),
        );
        TaskHandle {
            result: Some(result),
            joined,
        }
    }

    fn remove(&self, id: u64, failure: Option<String>) {
        let mut state = self.0.state.lock().unwrap();
        state.pending.remove(&id);
        if state.phase == Phase::Stopping {
            state.failures.extend(failure);
        }
        drop(state);
        self.0.changed.notify_waiters();
    }

    async fn wait_for(&self, deadline: Instant, stages: &[Stage]) -> bool {
        let wait = async {
            loop {
                let changed = self.0.changed.notified();
                if !self
                    .0
                    .state
                    .lock()
                    .unwrap()
                    .pending
                    .values()
                    .any(|item| stages.contains(&item.stage))
                {
                    return;
                }
                changed.await;
            }
        };
        tokio::time::timeout_at(deadline, wait).await.is_ok()
    }

    fn abort_safe(&self, stage: Stage) -> Vec<String> {
        let candidates: Vec<_> = self
            .0
            .state
            .lock()
            .unwrap()
            .pending
            .values()
            .filter(|item| item.stage == stage)
            .filter_map(|item| {
                let abort = item.safe_abort.as_ref()?;
                (!abort.is_finished()).then(|| (item.label.clone(), abort.clone()))
            })
            .collect();
        candidates
            .into_iter()
            .map(|(label, abort)| {
                abort.abort();
                label
            })
            .collect()
    }

    pub(crate) fn pending_labels(&self) -> Vec<String> {
        self.0
            .state
            .lock()
            .unwrap()
            .pending
            .values()
            .map(|item| item.label.clone())
            .collect()
    }
}

#[derive(Clone)]
pub(crate) struct TaskScope {
    app: AppLifecycle,
    stage: Stage,
    domain: &'static str,
    token: CancellationToken,
}

impl TaskScope {
    pub(crate) fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    pub(crate) fn is_running(&self) -> bool {
        self.app.phase() == Phase::Running
    }

    /// Keep an unconfirmed external resource release visible even if the
    /// task that owned the stop attempt has already returned.
    pub(crate) fn report_failure(&self, name: &str, error: &str) {
        let failure = format!("{}: {error}", self.label(name));
        tracing::error!(target: "lifecycle", %failure, "resource release unconfirmed");
        let mut state = self.app.0.state.lock().unwrap();
        if state.phase != Phase::Finished && !state.failures.contains(&failure) {
            state.failures.push(failure);
        }
    }

    pub(crate) fn lease(&self, name: &str) -> Result<WorkLease, Stopping> {
        let mut state = self.app.0.state.lock().unwrap();
        self.check_gate(&state)?;
        let id = state.next_id;
        state.next_id += 1;
        state.pending.insert(
            id,
            Pending {
                stage: self.stage,
                label: self.label(name),
                safe_abort: None,
            },
        );
        Ok(WorkLease {
            id,
            scope: self.clone(),
        })
    }

    pub(crate) fn spawn<F: Future + Send + 'static>(
        &self,
        name: &str,
        task: F,
    ) -> Result<TaskHandle<F::Output>, Stopping>
    where
        F::Output: Send + 'static,
    {
        self.spawn_inner(name, task, true, None)
    }

    /// Use for tasks with a persistence or native completion boundary that
    /// cannot be interrupted safely. Their owner must stop them cooperatively.
    pub(crate) fn spawn_durable<F: Future + Send + 'static>(
        &self,
        name: &str,
        task: F,
    ) -> Result<TaskHandle<F::Output>, Stopping>
    where
        F::Output: Send + 'static,
    {
        self.spawn_inner(name, task, false, None)
    }

    pub(crate) fn spawn_blocking<F, T>(
        &self,
        name: &str,
        task: F,
    ) -> Result<TaskHandle<T>, Stopping>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let mut state = self.app.0.state.lock().unwrap();
        self.check_gate(&state)?;
        let worker = self.app.runtime().spawn_blocking(task);
        Ok(self
            .app
            .observe(&mut state, self.stage, self.label(name), worker, false))
    }

    fn label(&self, name: &str) -> String {
        format!("{}/{name}", self.domain)
    }

    fn check_gate(&self, state: &State) -> Result<(), Stopping> {
        if (state.phase == Phase::Running && self.stage != Stage::Release)
            || (state.phase == Phase::Stopping && self.stage == Stage::Release)
        {
            Ok(())
        } else {
            Err(Stopping)
        }
    }

    fn spawn_inner<F: Future + Send + 'static>(
        &self,
        name: &str,
        task: F,
        safe_abort: bool,
        parent: Option<u64>,
    ) -> Result<TaskHandle<F::Output>, Stopping>
    where
        F::Output: Send + 'static,
    {
        let mut state = self.app.0.state.lock().unwrap();
        let finalizing = self.stage == Stage::Work
            && state.phase == Phase::Stopping
            && state.can_finalize
            && parent.is_some_and(|id| state.pending.contains_key(&id));
        if !finalizing {
            self.check_gate(&state)?;
        }
        let worker = self.app.runtime().spawn(task);
        Ok(self
            .app
            .observe(&mut state, self.stage, self.label(name), worker, safe_abort))
    }
}

/// Registration for an operation whose execution is owned by its caller,
/// such as an accepted request or a native callback.
pub(crate) struct WorkLease {
    id: u64,
    scope: TaskScope,
}

impl WorkLease {
    /// Only for a child needed to finish this admitted operation. The child is
    /// registered before the lease can drop; independent work uses the normal gate.
    pub(crate) fn spawn_finalizer<F: Future + Send + 'static>(
        &self,
        name: &str,
        task: F,
    ) -> Result<TaskHandle<F::Output>, Stopping>
    where
        F::Output: Send + 'static,
    {
        self.scope.spawn_inner(name, task, false, Some(self.id))
    }
}

impl Drop for WorkLease {
    fn drop(&mut self) {
        self.scope.app.remove(self.id, None);
    }
}

#[derive(Debug)]
pub(crate) struct ShutdownReport {
    pub(crate) aborted: Vec<String>,
    pub(crate) pending: Vec<String>,
    pub(crate) failures: Vec<String>,
}

impl ShutdownReport {
    pub(crate) fn is_clean(&self) -> bool {
        self.aborted.is_empty() && self.pending.is_empty() && self.failures.is_empty()
    }
}

pub(crate) struct Shutdown {
    app: AppLifecycle,
    work_grace: Instant,
    work_limit: Instant,
    service_grace: Instant,
    async_limit: Instant,
    real_deadline: std::time::Instant,
}

impl Shutdown {
    pub(crate) fn real_deadline(&self) -> std::time::Instant {
        self.real_deadline
    }

    #[cfg(test)]
    async fn finish<F, Fut>(self, release: F) -> ShutdownReport
    where
        F: FnOnce(TaskScope) -> Fut,
        Fut: Future<Output = Vec<String>>,
    {
        self.finish_with_finalizer(release, |_| async { Ok(()) })
            .await
    }

    // Only the winner of try_begin_shutdown owns this driver. It is outside
    // the task scopes, so waiting for them cannot wait on the driver itself.
    pub(crate) async fn finish_with_finalizer<F, Fut, G, Final>(
        self,
        release: F,
        finalize: G,
    ) -> ShutdownReport
    where
        F: FnOnce(TaskScope) -> Fut,
        Fut: Future<Output = Vec<String>>,
        G: FnOnce(TaskScope) -> Final,
        Final: Future<Output = Result<(), String>>,
    {
        let mut aborted = Vec::new();
        let mut work_drained = self.app.wait_for(self.work_grace, &[Stage::Work]).await;
        if !work_drained {
            aborted.extend(self.app.abort_safe(Stage::Work));
            work_drained = self.app.wait_for(self.work_limit, &[Stage::Work]).await;
        }
        self.app.0.state.lock().unwrap().can_finalize = false;

        let mut failures = Vec::new();
        if work_drained {
            let service_tokens =
                AppLifecycle::tokens(&self.app.0.state.lock().unwrap(), Stage::Service);
            for token in service_tokens {
                token.cancel();
            }
            let release_scope = TaskScope {
                app: self.app.clone(),
                stage: Stage::Release,
                domain: "release",
                token: CancellationToken::new(),
            };
            let orderly_release = async {
                failures = release(release_scope.clone()).await;
                self.app
                    .wait_for(self.service_grace, &[Stage::Service, Stage::Release])
                    .await
            };
            let services_drained = if tokio::time::timeout_at(self.service_grace, orderly_release)
                .await
                .unwrap_or(false)
            {
                true
            } else {
                failures.push("service release did not finish within its grace period".into());
                aborted.extend(self.app.abort_safe(Stage::Service));
                aborted.extend(self.app.abort_safe(Stage::Release));
                self.app
                    .wait_for(self.async_limit, &[Stage::Service, Stage::Release])
                    .await
            };
            let no_recorded_failure = self.app.0.state.lock().unwrap().failures.is_empty();
            if services_drained && failures.is_empty() && no_recorded_failure {
                match tokio::time::timeout_at(self.async_limit, finalize(release_scope)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => failures.push(error),
                    Err(_) => failures.push("final resource cleanup timed out".into()),
                }
            } else {
                failures.push(
                    "final resource cleanup skipped because an earlier shutdown step failed".into(),
                );
            }
        } else {
            failures.push("service release skipped because admitted work did not stop".into());
        }

        self.app.0.observers.close();
        if work_drained {
            let _ = tokio::time::timeout_at(self.async_limit, self.app.0.observers.wait()).await;
        }
        let mut state = self.app.0.state.lock().unwrap();
        state.phase = Phase::Finished;
        failures.append(&mut state.failures);
        ShutdownReport {
            aborted,
            pending: state
                .pending
                .values()
                .map(|item| item.label.clone())
                .collect(),
            failures,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::pending;

    const BUDGET: Duration = Duration::from_secs(10);

    #[tokio::test(start_paused = true)]
    async fn closes_admission_and_keeps_services_live_for_an_admitted_finalizer() {
        let app = AppLifecycle::new();
        let work = app.scope(TaskStage::Work, "chat");
        let service = app.scope(TaskStage::Service, "mcp");
        let turn = work.lease("turn").unwrap();
        let stopped = work.token();
        let service_token = service.token();
        let shutdown = app.try_begin_shutdown(BUDGET).unwrap();

        assert_eq!(app.phase(), Phase::Stopping);
        assert!(app.try_begin_shutdown(BUDGET).is_none());
        assert!(stopped.is_cancelled());
        assert!(!service_token.is_cancelled());
        assert!(work.lease("late-turn").is_err());
        assert!(service.spawn("late-connect", async {}).is_err());
        assert!(work.spawn("independent-post-work", async {}).is_err());

        let (release_tx, release_rx) = oneshot::channel();
        let (committed_tx, committed_rx) = oneshot::channel();
        turn.spawn_finalizer("commit", async move {
            release_rx.await.unwrap();
            assert!(!service_token.is_cancelled());
            committed_tx.send(()).unwrap();
        })
        .unwrap();
        drop(turn);
        let join = tokio::spawn(async move {
            shutdown
                .finish(|_| async move {
                    assert!(service.token().is_cancelled());
                    committed_rx.await.unwrap();
                    Vec::new()
                })
                .await
        });
        tokio::task::yield_now().await;
        assert!(!join.is_finished());
        release_tx.send(()).unwrap();
        assert!(join.await.unwrap().is_clean());
        assert!(app.pending_labels().is_empty());
        assert_eq!(app.phase(), Phase::Finished);
        assert!(app.try_begin_shutdown(BUDGET).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn aborts_and_confirms_cancellable_work_before_releasing_services() {
        struct DropFlag(Arc<AtomicBool>);

        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let app = AppLifecycle::new();
        let scope = app.scope(TaskStage::Work, "worker");
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = DropFlag(dropped.clone());
        let mut task = scope
            .spawn("stubborn", async move {
                let _flag = flag;
                pending::<()>().await;
            })
            .unwrap();
        let shutdown = app.try_begin_shutdown(BUDGET).unwrap();
        let report = shutdown
            .finish(|_| async move {
                assert!(dropped.load(Ordering::SeqCst));
                Vec::new()
            })
            .await;

        assert_eq!(report.aborted, ["worker/stubborn"]);
        assert!(!report.is_clean());
        assert!(report.pending.is_empty());
        assert!(task.is_finished());
        assert!(matches!(task.join().await, Err(TaskError::Join(error)) if error.is_cancelled()));
    }

    #[tokio::test(start_paused = true)]
    async fn names_live_native_work_and_does_not_release_its_dependencies() {
        let app = AppLifecycle::new();
        let work = app.scope(TaskStage::Work, "native");
        let service = app.scope(TaskStage::Service, "dependency");
        let (started_tx, started_rx) = oneshot::channel();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let mut native = work
            .spawn_blocking("device", move || {
                started_tx.send(()).unwrap();
                stop_rx.recv().unwrap();
            })
            .unwrap();
        started_rx.await.unwrap();
        let shutdown = app.try_begin_shutdown(BUDGET).unwrap();
        tokio::time::advance(BUDGET).await;
        let report = shutdown
            .finish(|_| async { panic!("released a live dependency") })
            .await;

        // Unblock the worker to let the test runtime shut down.
        stop_tx.send(()).unwrap();
        native.join().await.unwrap();
        assert_eq!(report.pending, ["native/device"]);
        assert!(report.aborted.is_empty());
        assert!(!report.is_clean());
        assert!(!service.token().is_cancelled());
    }

    #[tokio::test(start_paused = true)]
    async fn observes_panics_even_when_the_caller_drops_the_handle() {
        let app = AppLifecycle::new();
        let scope = app.scope(TaskStage::Work, "tasks");
        let stop = scope.token();
        let task = scope
            .spawn("panic", async move {
                stop.cancelled().await;
                panic!("test panic");
            })
            .unwrap();
        drop(task);
        let report = app
            .try_begin_shutdown(BUDGET)
            .unwrap()
            .finish(|_| async { Vec::new() })
            .await;
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("tasks/panic")));
        assert!(report.pending.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn stuck_release_shares_the_original_budget_and_stays_named() {
        let app = AppLifecycle::new();
        let started = Instant::now();
        let report = app
            .try_begin_shutdown(BUDGET)
            .unwrap()
            .finish(|scope| async move {
                let mut handle = scope.spawn_durable("opaque-sdk", pending::<()>()).unwrap();
                handle.join().await.unwrap();
                Vec::new()
            })
            .await;
        assert!(Instant::now() - started < BUDGET);
        assert_eq!(report.pending, ["release/opaque-sdk"]);
        assert!(!report.is_clean());
    }

    #[tokio::test(start_paused = true)]
    async fn final_resource_waits_for_both_service_worker_and_release_hook() {
        let app = AppLifecycle::new();
        let service = app.scope(TaskStage::Service, "dependency");
        let stopped = service.token();
        let worker_done = Arc::new(AtomicBool::new(false));
        let worker_flag = worker_done.clone();
        service
            .spawn_durable("resource", async move {
                stopped.cancelled().await;
                // The explicit release hook is allowed to finish before this worker.
                tokio::time::sleep(Duration::from_secs(1)).await;
                worker_flag.store(true, Ordering::Release);
            })
            .unwrap();

        let hook_done = Arc::new(AtomicBool::new(false));
        let hook_flag = hook_done.clone();
        let report = app
            .try_begin_shutdown(BUDGET)
            .unwrap()
            .finish_with_finalizer(
                |_| async move {
                    hook_flag.store(true, Ordering::Release);
                    Vec::new()
                },
                |_| async move {
                    assert!(worker_done.load(Ordering::Acquire));
                    assert!(hook_done.load(Ordering::Acquire));
                    Ok(())
                },
            )
            .await;
        assert!(report.is_clean());
    }

    #[tokio::test(start_paused = true)]
    async fn unconfirmed_resource_release_is_retained_without_a_caller() {
        let app = AppLifecycle::new();
        let service = app.scope(TaskStage::Service, "process");
        service
            .spawn_durable("stop", {
                let service = service.clone();
                async move {
                    service.token().cancelled().await;
                    service.report_failure("stop", "child exit was not confirmed");
                }
            })
            .unwrap();

        let report = app
            .try_begin_shutdown(BUDGET)
            .unwrap()
            .finish_with_finalizer(
                |_| async { Vec::new() },
                |_| async { panic!("final resource released after failed child cleanup") },
            )
            .await;
        assert!(report.pending.is_empty());
        assert!(report
            .failures
            .contains(&"process/stop: child exit was not confirmed".into()));
        assert!(!report.is_clean());
    }
}
