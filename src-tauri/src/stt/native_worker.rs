use crate::lifecycle::{TaskHandle, TaskScope, WorkLease};
use std::{
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tokio::sync::{oneshot, Mutex as AsyncMutex, OnceCell};
use tokio_util::sync::CancellationToken;

pub(super) const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const CHECK_STOP_INTERVAL: Duration = Duration::from_millis(50);

struct Run<C> {
    commands: mpsc::Sender<C>,
    join: AsyncMutex<TaskHandle<Result<(), String>>>,
    result: OnceCell<Result<(), String>>,
}

pub(super) struct NativeWorker<C> {
    name: &'static str,
    work: TaskScope,
    service: TaskScope,
    stop: CancellationToken,
    run: Mutex<Option<Arc<Run<C>>>>,
}

impl<C: Send + 'static> NativeWorker<C> {
    pub(super) fn new(name: &'static str, work: TaskScope, service: TaskScope) -> Self {
        Self {
            name,
            stop: work.token().child_token(),
            work,
            service,
            run: Mutex::new(None),
        }
    }

    pub(super) fn admit(&self, action: &str) -> Result<WorkLease, String> {
        let work = self.work.lease(action).map_err(|error| error.to_string())?;
        if self.stop.is_cancelled() {
            return Err(format!("{} is shutting down", self.name));
        }
        Ok(work)
    }

    pub(super) fn commands(
        &self,
        start: impl FnOnce(mpsc::Receiver<C>, CancellationToken) -> Result<(), String> + Send + 'static,
    ) -> Result<mpsc::Sender<C>, String> {
        let mut slot = self.run.lock().unwrap();
        if self.stop.is_cancelled() {
            return Err(format!("{} is shutting down", self.name));
        }
        if let Some(run) = slot.as_ref() {
            let live = run.result.get().is_none()
                && run.join.try_lock().map_or(true, |task| !task.is_finished());
            if live {
                return Ok(run.commands.clone());
            }
        }

        let (commands, incoming) = mpsc::channel();
        let stop = self.stop.clone();
        let task = self
            .service
            .spawn_blocking(self.name, move || start(incoming, stop))
            .map_err(|error| error.to_string())?;
        *slot = Some(Arc::new(Run {
            commands: commands.clone(),
            join: AsyncMutex::new(task),
            result: OnceCell::new(),
        }));
        Ok(commands)
    }

    pub(super) fn running_commands(&self) -> Option<mpsc::Sender<C>> {
        self.run
            .lock()
            .unwrap()
            .as_ref()
            .map(|run| run.commands.clone())
    }

    pub(super) async fn confirm_app_stop(&self) -> Result<(), String> {
        tokio::time::timeout(COMMAND_TIMEOUT, self.shutdown())
            .await
            .map_err(|_| format!("{} is still stopping", self.name))?
    }

    pub(super) async fn shutdown(&self) -> Result<(), String> {
        self.stop.cancel();
        let run = self.run.lock().unwrap().clone();
        let Some(run) = run else {
            return Ok(());
        };
        run.result
            .get_or_init(|| async {
                run.join
                    .lock()
                    .await
                    .join()
                    .await
                    .map_err(|error| format!("{} worker failed: {error}", self.name))?
            })
            .await
            .clone()
    }
}

pub(super) fn receive<C>(commands: &mpsc::Receiver<C>, stop: &CancellationToken) -> Option<C> {
    while !stop.is_cancelled() {
        match commands.recv_timeout(CHECK_STOP_INTERVAL) {
            Ok(command) if !stop.is_cancelled() => return Some(command),
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    None
}

pub(super) async fn response<T>(
    receiver: oneshot::Receiver<Result<T, String>>,
    name: &str,
) -> Result<T, String> {
    tokio::time::timeout(COMMAND_TIMEOUT, receiver)
        .await
        .map_err(|_| format!("{name} is still busy. Completion has not been confirmed."))?
        .map_err(|_| format!("{name} worker exited before confirming completion"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::{AppLifecycle, TaskStage};

    #[tokio::test]
    async fn shutdown_waits_for_non_interruptible_native_work() {
        let app = AppLifecycle::new();
        let worker = Arc::new(NativeWorker::<()>::new(
            "test native audio",
            app.scope(TaskStage::Work, "audio"),
            app.scope(TaskStage::Service, "audio"),
        ));
        let (started, running) = oneshot::channel();
        let (release, finish) = mpsc::sync_channel(1);
        worker
            .commands(move |_, stop| {
                let _ = started.send(());
                finish.recv().unwrap();
                assert!(stop.is_cancelled());
                Ok(())
            })
            .unwrap();
        running.await.unwrap();
        let stopping = worker.clone();
        let task = tokio::spawn(async move { stopping.shutdown().await });
        worker.stop.cancelled().await;
        assert!(!task.is_finished());
        assert!(worker.admit("new capture").is_err());
        release.send(()).unwrap();
        task.await.unwrap().unwrap();
    }
}
