use crate::lifecycle::{TaskHandle, TaskScope, WorkLease};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::{Mutex, OnceCell};
use tokio_util::sync::CancellationToken;

const STOP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(crate) struct BotIngress {
    stop: CancellationToken,
    work: TaskScope,
}

impl BotIngress {
    pub(crate) fn admit(&self, label: &'static str) -> Option<WorkLease> {
        let lease = self.work.lease(label).ok()?;
        (!self.stop.is_cancelled()).then_some(lease)
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        use crate::lifecycle::{AppLifecycle, TaskStage};
        let work = AppLifecycle::new().scope(TaskStage::Work, "test bot");
        Self {
            stop: work.token().child_token(),
            work,
        }
    }

    pub(crate) fn token(&self) -> &CancellationToken {
        &self.stop
    }
}

#[derive(Clone)]
pub(crate) struct BotWorker {
    name: &'static str,
    work: TaskScope,
    service: TaskScope,
    closed: CancellationToken,
    run: Arc<Mutex<Option<Arc<Run>>>>,
}

struct Run {
    stop: CancellationToken,
    task: Mutex<TaskHandle<Result<(), String>>>,
    result: OnceCell<Result<(), String>>,
}

impl Run {
    fn is_running(&self) -> bool {
        self.result.get().is_none()
            && self
                .task
                .try_lock()
                .map_or(true, |task| !task.is_finished())
    }

    async fn joined(&self) -> Result<(), String> {
        self.result
            .get_or_init(|| async {
                self.task
                    .lock()
                    .await
                    .join()
                    .await
                    .map_err(|error| format!("Bot worker failed: {error}"))?
            })
            .await
            .clone()
    }
}

impl BotWorker {
    pub(crate) fn new(name: &'static str, work: TaskScope, service: TaskScope) -> Self {
        Self {
            name,
            closed: work.token().child_token(),
            work,
            service,
            run: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn close_ingress(&self) {
        self.closed.cancel();
    }

    pub(crate) fn admit_control(&self, action: &str) -> Result<WorkLease, String> {
        let lease = self.work.lease(action).map_err(|error| error.to_string())?;
        if self.closed.is_cancelled() {
            return Err(format!("{} is shutting down", self.name));
        }
        Ok(lease)
    }

    pub(crate) async fn start<F, Fut>(&self, prepare: F) -> Result<(), String>
    where
        F: FnOnce(BotIngress) -> Result<Fut, String>,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let mut run = self.run.lock().await;
        if run.as_ref().is_some_and(|run| run.is_running()) {
            return Err(format!("{} is already running or stopping", self.name));
        }
        let _lease = self.admit_control(self.name)?;
        let stop = self.closed.child_token();
        let worker = prepare(BotIngress {
            stop: stop.clone(),
            work: self.work.clone(),
        })?;
        let name = self.name;
        let stopping = stop.clone();
        let task = self
            .service
            .spawn_durable(name, async move {
                let result = worker.await;
                if let Err(error) = &result {
                    tracing::error!(target: "bot", service = name, %error, "Bot worker failed");
                } else if !stopping.is_cancelled() {
                    tracing::warn!(target: "bot", service = name, "Bot worker exited without a stop request");
                }
                result
            })
            .map_err(|error| error.to_string())?;
        *run = Some(Arc::new(Run {
            stop,
            task: Mutex::new(task),
            result: OnceCell::new(),
        }));
        Ok(())
    }

    pub(crate) async fn is_running(&self) -> bool {
        self.run
            .lock()
            .await
            .as_ref()
            .is_some_and(|run| run.is_running())
    }

    pub(crate) async fn stop(&self) -> Result<(), String> {
        let Some(run) = self.run.lock().await.clone() else {
            return Ok(());
        };
        run.stop.cancel();
        let joined = tokio::time::timeout(STOP_TIMEOUT, run.joined())
            .await
            .map_err(|_| {
                format!(
                    "{} is still stopping. Delivery of any outstanding external reply is uncertain.",
                    self.name
                )
            })?;
        let mut slot = self.run.lock().await;
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &run))
        {
            slot.take();
        }
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::{AppLifecycle, TaskStage};
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn stop_waits_for_admitted_work_and_blocks_restart_until_completion() {
        let app = AppLifecycle::new();
        let worker = BotWorker::new(
            "bot test",
            app.scope(TaskStage::Work, "bot"),
            app.scope(TaskStage::Service, "bot"),
        );
        let (ready, admitted) = oneshot::channel();
        let (stopped, gate_closed) = oneshot::channel();
        let (release, finish) = oneshot::channel();
        worker
            .start(move |gate| {
                Ok(async move {
                    let _lease = gate.admit("request").unwrap();
                    ready.send(()).unwrap();
                    gate.token().cancelled().await;
                    assert!(gate.admit("new request").is_none());
                    stopped.send(()).unwrap();
                    finish.await.unwrap();
                    Ok(())
                })
            })
            .await
            .unwrap();

        admitted.await.unwrap();
        let stopping = worker.clone();
        let stop = tokio::spawn(async move { stopping.stop().await });
        gate_closed.await.unwrap();
        assert!(worker.is_running().await);
        assert!(worker.start(|_| Ok(async { Ok(()) })).await.is_err());
        assert!(!stop.is_finished());
        release.send(()).unwrap();
        stop.await.unwrap().unwrap();

        worker
            .start(|gate| {
                Ok(async move {
                    gate.token().cancelled().await;
                    Ok(())
                })
            })
            .await
            .unwrap();
        worker.close_ingress();
        worker.stop().await.unwrap();
        assert!(!worker.is_running().await);
        assert!(worker.start(|_| Ok(async { Ok(()) })).await.is_err());
    }

    #[tokio::test]
    async fn panicking_worker_is_joined_and_stop_reports_failure() {
        let app = AppLifecycle::new();
        let worker = BotWorker::new(
            "panic test",
            app.scope(TaskStage::Work, "bot"),
            app.scope(TaskStage::Service, "bot"),
        );
        worker
            .start(|_| Ok(async { panic!("sample bot panic") }))
            .await
            .unwrap();
        assert!(worker.stop().await.unwrap_err().contains("panic"));
        assert!(!worker.is_running().await);
    }
}
