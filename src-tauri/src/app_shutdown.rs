//! Tauri exit integration; the lifecycle driver itself does not depend on Tauri.

use crate::lifecycle::{AppLifecycle, Phase, TaskScope};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, ExitRequestApi, Manager};

const SHUTDOWN_BUDGET: Duration = Duration::from_secs(10);
const FORCED_EXIT_CODE: i32 = 1;

#[derive(Clone, Default)]
pub(super) struct ExitCompletion(Arc<Mutex<Option<i32>>>);

impl ExitCompletion {
    pub(super) fn code(&self) -> Option<i32> {
        *self.0.lock().unwrap()
    }
}

pub(super) fn on_exit_requested(app: &AppHandle, code: Option<i32>, api: ExitRequestApi) {
    let lifecycle = app.state::<AppLifecycle>();
    if lifecycle.phase() == Phase::Finished
        && code.is_some()
        && code == app.state::<ExitCompletion>().code()
    {
        return;
    }
    if code == Some(tauri::RESTART_EXIT_CODE) {
        // Tauri ignores prevent_exit for its restart sentinel. The application
        // currently has no user-facing path that calls the process restart API.
        tracing::warn!(target: "lifecycle", "Tauri restart bypassed asynchronous shutdown");
        return;
    }
    api.prevent_exit();
    let Some(shutdown) = lifecycle.try_begin_shutdown(SHUTDOWN_BUDGET) else {
        return;
    };
    start_watchdog(lifecycle.inner().clone(), shutdown.real_deadline());
    if let Some(bot) = app.try_state::<crate::bot::BotRuntimeService>() {
        bot.close_ingress();
    }
    if let Some(telegram) = app.try_state::<crate::telegram::TelegramService>() {
        telegram.close_ingress();
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let report = shutdown
            .finish_with_finalizer(
                |scope| release_services(app.clone(), scope),
                |scope| close_database(app.clone(), scope),
            )
            .await;
        let code = code.unwrap_or_default();
        let final_code = if report.is_clean() {
            tracing::info!(target: "lifecycle", "background shutdown complete; requesting final exit");
            code
        } else {
            tracing::error!(
                target: "lifecycle",
                aborted = ?report.aborted,
                pending = ?report.pending,
                failures = ?report.failures,
                "application shutdown forced",
            );
            if code == 0 {
                FORCED_EXIT_CODE
            } else {
                code
            }
        };
        *app.state::<ExitCompletion>().0.lock().unwrap() = Some(final_code);
        app.exit(final_code);
    });
}

async fn close_database(app: AppHandle, scope: TaskScope) -> Result<(), String> {
    run_release(&scope, "sqlite-pool", async move {
        if let Some(orchestrator) = app.try_state::<crate::ai::context::AIOrchestrator>() {
            let _database_guard =
                crate::ai::context::acquire_database_operation_write_guard().await;
            orchestrator.db.close().await;
        }
        Ok(())
    })
    .await
}

async fn release_services(app: AppHandle, scope: TaskScope) -> Vec<String> {
    let bot_app = app.clone();
    let telegram_app = app.clone();
    let mic_app = app.clone();
    let wake_app = app.clone();
    let pet_app = app.clone();
    let (bot, telegram, mcp, mic, wake_word, pet) = tokio::join!(
        run_release(&scope, "bot", async move {
            if let Some(bot) = bot_app.try_state::<crate::bot::BotRuntimeService>() {
                bot.shutdown().await?;
            }
            Ok(())
        }),
        run_release(&scope, "telegram", async move {
            if let Some(telegram) = telegram_app.try_state::<crate::telegram::TelegramService>() {
                telegram.shutdown().await?;
            }
            Ok(())
        }),
        run_release(&scope, "mcp", async move {
            let manager = app.try_state::<Arc<tokio::sync::Mutex<crate::mcp::McpManager>>>();
            let registry =
                app.try_state::<Arc<tokio::sync::RwLock<crate::actions::ActionRegistry>>>();
            if let (Some(manager), Some(registry)) = (manager, registry) {
                crate::mcp::bridge::shutdown(manager.inner(), registry.inner())
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        }),
        run_release(&scope, "native-mic", async move {
            if let Some(mic) = mic_app.try_state::<crate::stt::NativeMicState>() {
                mic.shutdown().await?;
            }
            Ok(())
        }),
        run_release(&scope, "wake-word", async move {
            if let Some(wake_word) = wake_app.try_state::<crate::stt::NativeWakeWordState>() {
                wake_word.shutdown().await?;
            }
            Ok(())
        }),
        run_release(&scope, "pet-shortcut", async move {
            crate::commands::pet::release_pet_shortcut(&pet_app)
        }),
    );
    [bot, telegram, mcp, mic, wake_word, pet]
        .into_iter()
        .filter_map(Result::err)
        .collect()
}

async fn run_release<F>(scope: &TaskScope, name: &str, release: F) -> Result<(), String>
where
    F: Future<Output = Result<(), String>> + Send + 'static,
{
    let mut handle = scope
        .spawn_durable(name, release)
        .map_err(|error| format!("{name}: {error}"))?;
    handle
        .join()
        .await
        .map_err(|error| format!("{name}: {error}"))?
        .map_err(|error| format!("{name}: {error}"))
}

fn start_watchdog(lifecycle: AppLifecycle, deadline: Instant) {
    // Do not rely on Tokio or the Tauri event loop getting a chance to run.
    // Keep diagnostic output on another thread so a blocked writer cannot
    // defeat the hard deadline either.
    let watchdog = std::thread::Builder::new()
        .name("shutdown-deadline".into())
        .spawn(move || {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            hard_exit();
        });
    if let Err(error) = watchdog {
        tracing::error!(target: "lifecycle", %error, "could not start shutdown watchdog");
        hard_exit();
    }
    let _ = std::thread::Builder::new()
        .name("shutdown-diagnostics".into())
        .spawn(move || {
            let report_at = deadline - SHUTDOWN_BUDGET.mul_f64(0.025);
            std::thread::sleep(report_at.saturating_duration_since(Instant::now()));
            let mut pending = lifecycle.pending_labels();
            if pending.is_empty() {
                pending.push(match lifecycle.phase() {
                    Phase::Finished => "tauri/final-exit".into(),
                    _ => "lifecycle/exit-coordinator".into(),
                });
            }
            tracing::error!(
                target: "lifecycle",
                ?pending,
                "application has not exited before the watchdog deadline",
            );
        });
}

#[cfg(windows)]
fn hard_exit() -> ! {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};

    // SAFETY: The pseudo-handle denotes our own process, remains valid for
    // this call and does not need closing. Termination bypasses DLL cleanup,
    // which could be blocked by the very native work this watchdog bounds.
    unsafe {
        TerminateProcess(GetCurrentProcess(), FORCED_EXIT_CODE as u32);
    }
    std::process::exit(FORCED_EXIT_CODE)
}

#[cfg(unix)]
fn hard_exit() -> ! {
    // SAFETY: The watchdog owns the forced exit decision. _exit avoids
    // running process cleanup callbacks that could depend on a blocked worker.
    unsafe { libc::_exit(FORCED_EXIT_CODE) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::process::Stdio;

    #[test]
    fn watchdog_stops_process_even_when_main_is_blocked() {
        const CHILD: &str = "KOKORO_SHUTDOWN_WATCHDOG_PROCESS_TEST";
        if std::env::var_os(CHILD).is_some() {
            crate::utils::logging::init_logging();
            let lifecycle = AppLifecycle::new();
            let _blocked = lifecycle
                .scope(crate::lifecycle::TaskStage::Work, "watchdog-fixture")
                .lease("blocked native work")
                .unwrap();
            let shutdown = lifecycle
                .try_begin_shutdown(Duration::from_millis(600))
                .unwrap();
            start_watchdog(lifecycle, shutdown.real_deadline());
            loop {
                std::thread::park();
            }
        }

        // Exercise the actual native exit path without stopping the test runner.
        // The child has no Tokio runtime and its main test thread never returns.
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app_shutdown::tests::watchdog_stops_process_even_when_main_is_blocked",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("RUST_LOG", "lifecycle=error")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(FORCED_EXIT_CODE));
                let mut output = String::new();
                child
                    .stdout
                    .take()
                    .unwrap()
                    .read_to_string(&mut output)
                    .unwrap();
                assert!(output.contains("watchdog deadline"), "{output}");
                assert!(
                    output.contains("watchdog-fixture/blocked native work"),
                    "{output}"
                );
                return;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("native shutdown watchdog did not exit the child process");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
