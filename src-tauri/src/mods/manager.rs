// pattern: Imperative Shell
use crate::hooks::{HookEvent, HookPayload, HookRuntime, ModHookPayload};
use crate::mods::api::ScriptEvent;
use crate::mods::manifest::ModManifest;
use crate::mods::theme::ModThemeJson;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

const SCRIPT_EXECUTION_BUDGET: Duration = Duration::from_secs(5);
const SCRIPT_EVENT_BUDGET: Duration = Duration::from_millis(250);
const SCRIPT_EVENT_CAPACITY: usize = 32;

fn install_script_interrupt_handler(runtime: &rquickjs::Runtime) -> Arc<Mutex<Option<Instant>>> {
    let deadline = Arc::new(Mutex::new(None));
    let interrupt_deadline = deadline.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        interrupt_deadline
            .lock()
            .map(|deadline| deadline.is_some_and(|deadline| Instant::now() >= deadline))
            .unwrap_or(true)
    })));
    deadline
}

fn eval_script_with_budget(
    context: &rquickjs::Context,
    deadline: &Arc<Mutex<Option<Instant>>>,
    code: &str,
    budget: Duration,
) -> Result<(), String> {
    with_script_budget(deadline, budget, || {
        context.with(|ctx| ctx.eval::<(), _>(code))
    })
}

fn with_script_budget<T>(
    deadline: &Arc<Mutex<Option<Instant>>>,
    budget: Duration,
    execute: impl FnOnce() -> rquickjs::Result<T>,
) -> Result<T, String> {
    {
        let mut guard = deadline
            .lock()
            .map_err(|_| "failed to lock script execution budget".to_string())?;
        *guard = Some(Instant::now() + budget);
    }

    let result = execute();
    let timed_out = deadline
        .lock()
        .map(|guard| guard.is_some_and(|deadline| Instant::now() >= deadline))
        .unwrap_or(true);
    if let Ok(mut guard) = deadline.lock() {
        *guard = None;
    }

    match result {
        Ok(value) => Ok(value),
        Err(_error) if timed_out => Err(format!(
            "script execution timed out after {} ms",
            budget.as_millis()
        )),
        Err(error) => Err(error.to_string()),
    }
}

/// 验证 `file_path` 在规范化后仍位于 `base_dir` 内，防止路径遍历攻击。
/// 返回规范化后的绝对路径，若路径逃出 base_dir 则返回 Err。
fn safe_join(base_dir: &Path, file_path: &str) -> Result<PathBuf, String> {
    let joined = base_dir.join(file_path);
    // canonicalize 会解析 .. 和符号链接；文件必须已存在
    let canonical = joined
        .canonicalize()
        .map_err(|e| format!("Invalid path '{}': {}", file_path, e))?;
    let canonical_base = base_dir
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize mod dir: {}", e))?;
    if canonical.starts_with(&canonical_base) {
        Ok(canonical)
    } else {
        Err(format!(
            "Path '{}' escapes mod directory (resolved to '{}')",
            file_path,
            canonical.display()
        ))
    }
}

struct ScriptModule {
    id: String,
    scripts: Vec<(String, String)>,
}

enum ScriptCommand {
    DispatchEvent {
        event: String,
        payload: JsonValue,
        slot: Option<OwnedSemaphorePermit>,
    },
    ReplaceListeners {
        module: Option<ScriptModule>,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// All commands enter the same FIFO so a listener replacement cannot overtake
/// previously accepted events. Only completions and serialized lifecycle
/// changes bypass the limit; chat fragments and cues may be shed under load.
#[derive(Clone)]
pub(crate) struct ModScriptQueue {
    tx: mpsc::UnboundedSender<ScriptCommand>,
    event_slots: Arc<Semaphore>,
}

impl ModScriptQueue {
    fn channel() -> (Self, mpsc::UnboundedReceiver<ScriptCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Self {
                tx,
                event_slots: Arc::new(Semaphore::new(SCRIPT_EVENT_CAPACITY)),
            },
            rx,
        )
    }

    pub(crate) fn enqueue_chat_event(&self, name: &str, payload: JsonValue) {
        let event = match name {
            "chat-turn-delta" => "chat",
            "chat-turn-finish" => "chat-done",
            "chat-cue" => "cue",
            _ => return,
        };
        let slot = if event == "chat-done" {
            None
        } else {
            let Ok(slot) = self.event_slots.clone().try_acquire_owned() else {
                return;
            };
            Some(slot)
        };
        // The receiver can only disconnect if the runtime has stopped.
        let _ = self.send_event(event, payload, slot);
    }

    fn send_event(
        &self,
        event: &str,
        payload: JsonValue,
        slot: Option<OwnedSemaphorePermit>,
    ) -> Result<(), String> {
        self.tx
            .send(ScriptCommand::DispatchEvent {
                event: event.into(),
                payload,
                slot,
            })
            .map_err(|_| "Script runtime disconnected".to_string())
    }

    async fn dispatch_event(&self, event: &str, payload: JsonValue) -> Result<(), String> {
        let slot = self
            .event_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| err.to_string())?;
        self.send_event(event, payload, Some(slot))
    }

    async fn replace_listeners(&self, module: Option<ScriptModule>) -> Result<(), String> {
        let (reply, result) = oneshot::channel();
        self.tx
            .send(ScriptCommand::ReplaceListeners { module, reply })
            .map_err(|_| "Script runtime disconnected".to_string())?;
        result
            .await
            .map_err(|_| "Script runtime disconnected".to_string())?
    }
}

fn dispatch_script_event(
    context: &rquickjs::Context,
    deadline: &Arc<Mutex<Option<Instant>>>,
    event: &str,
    payload: &JsonValue,
    budget: Duration,
) {
    context.with(|ctx| {
        let invocation = with_script_budget(deadline, budget, || {
            let registry: rquickjs::Object = ctx.globals().get("__listeners")?;
            let Some(listeners) = registry.get::<_, Option<rquickjs::Array>>(event)? else {
                return Ok(None);
            };
            let callbacks = listeners
                .iter::<rquickjs::Value>()
                .collect::<rquickjs::Result<Vec<_>>>()?;
            let argument = ctx.json_parse(payload.to_string())?;
            Ok(Some((listeners, callbacks, argument)))
        });
        let (listeners, callbacks, argument) = match invocation {
            Ok(Some(invocation)) => invocation,
            Ok(None) => return,
            Err(error) => {
                let _ = ctx.catch();
                tracing::warn!(target: "mods", "QuickJS event '{event}' failed: {error}");
                return;
            }
        };
        for callback in callbacks
            .into_iter()
            .filter_map(rquickjs::Value::into_function)
        {
            // A timed-out listener must not suppress later listeners for the
            // same event, especially a chat completion.
            if let Err(error) = with_script_budget(deadline, budget, || {
                callback.call::<_, ()>((
                    rquickjs::function::This(listeners.clone()),
                    argument.clone(),
                ))
            }) {
                let _ = ctx.catch();
                tracing::warn!(target: "mods", "QuickJS listener for '{event}' failed: {error}");
            }
        }
    });
}

fn run_script_command(
    context: &rquickjs::Context,
    deadline: &Arc<Mutex<Option<Instant>>>,
    command: ScriptCommand,
    event_budget: Duration,
) {
    match command {
        ScriptCommand::DispatchEvent {
            event,
            payload,
            slot: _slot,
        } => {
            dispatch_script_event(context, deadline, &event, &payload, event_budget);
        }
        ScriptCommand::ReplaceListeners { module, reply } => {
            let result = eval_script_with_budget(context, deadline, "globalThis.__listeners = { __proto__: null };", SCRIPT_EXECUTION_BUDGET)
                .map(|()| {
                    if let Some(module) = module {
                        // Install and initialize the replacement in one command so
                        // no event can run in the gap between reset and registration.
                        for (path, code) in module.scripts {
                            match eval_script_with_budget(context, deadline, &code, SCRIPT_EXECUTION_BUDGET) {
                                Ok(()) => tracing::info!(target: "mods", "[ModManager] Script '{}' executed for mod '{}'", path, module.id),
                                Err(error) => tracing::error!(target: "mods", "[ModManager] Script '{}' error: {}", path, error),
                            }
                        }
                        dispatch_script_event(context, deadline, "init", &serde_json::json!({ "modId": module.id }), event_budget);
                    }
                });
            let _ = reply.send(result);
        }
    }
}

/// Payload for the mod:script-event Tauri event
#[derive(serde::Serialize, Clone)]
struct ScriptEventPayload {
    event: String,
    payload: serde_json::Value,
}

/// Payload for the mod:ui-message Tauri event
#[derive(serde::Serialize, Clone)]
struct UiMessagePayload {
    component: String,
    payload: serde_json::Value,
}

/// Payload for character cue events
#[derive(serde::Serialize, Clone)]
struct CuePayload {
    cue: String,
}

fn validate_manifest_capabilities(manifest: &ModManifest) -> Result<(), String> {
    for capability in &manifest.capabilities {
        if capability.name.trim().is_empty() {
            return Err(format!(
                "Invalid capability in mod '{}': capability name cannot be empty",
                manifest.id
            ));
        }
    }
    Ok(())
}

fn build_mod_hook_payload(manifest: &ModManifest, stage: &str) -> HookPayload {
    let script_count = if !manifest.scripts.is_empty() {
        manifest.scripts.len()
    } else if manifest.entry.is_some() {
        1
    } else {
        0
    };

    HookPayload::Mod(ModHookPayload {
        mod_id: manifest.id.clone(),
        stage: stage.to_string(),
        has_theme: manifest.theme.is_some(),
        has_layout: manifest.layout.is_some(),
        component_count: manifest.components.len(),
        script_count,
    })
}

/// ModManager handles mod discovery, metadata, theme/layout loading, and script execution.
/// The QuickJS runtime lives on a separate thread and is communicated with via channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModRuntimeState {
    Uninitialized,
    Initializing,
    Running,
    Failed,
    Disconnected,
}

impl ModRuntimeState {
    fn as_u8(self) -> u8 {
        match self {
            ModRuntimeState::Uninitialized => 0,
            ModRuntimeState::Initializing => 1,
            ModRuntimeState::Running => 2,
            ModRuntimeState::Failed => 3,
            ModRuntimeState::Disconnected => 4,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => ModRuntimeState::Initializing,
            2 => ModRuntimeState::Running,
            3 => ModRuntimeState::Failed,
            4 => ModRuntimeState::Disconnected,
            _ => ModRuntimeState::Uninitialized,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ModRuntimeState::Uninitialized => "uninitialized",
            ModRuntimeState::Initializing => "initializing",
            ModRuntimeState::Running => "running",
            ModRuntimeState::Failed => "failed",
            ModRuntimeState::Disconnected => "disconnected",
        }
    }
}

pub struct ModManager {
    pub mods_path: PathBuf,
    pub loaded_mods: HashMap<String, ModManifest>,
    scripts: Option<ModScriptQueue>,
    runtime_state: Arc<AtomicU8>,
    /// Currently active theme loaded from a mod's theme.json
    pub active_theme: Option<ModThemeJson>,
    /// Currently active layout loaded from a mod's layout.json
    pub active_layout: Option<JsonValue>,
    /// Identity of the MOD whose scripts currently own the QuickJS listeners.
    active_mod_id: Option<String>,
}

impl ModManager {
    pub fn runtime_state(&self) -> ModRuntimeState {
        ModRuntimeState::from_u8(self.runtime_state.load(Ordering::SeqCst))
    }

    fn set_runtime_state(&self, state: ModRuntimeState) {
        self.runtime_state.store(state.as_u8(), Ordering::SeqCst);
    }

    fn runtime_ready(&self) -> bool {
        self.runtime_state() == ModRuntimeState::Running && self.scripts.is_some()
    }

    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            mods_path: path.as_ref().to_path_buf(),
            loaded_mods: HashMap::new(),
            scripts: None,
            runtime_state: Arc::new(AtomicU8::new(ModRuntimeState::Uninitialized.as_u8())),
            active_theme: None,
            active_layout: None,
            active_mod_id: None,
        }
    }

    /// Spawn the QuickJS runtime thread and the event relay task.
    /// The event relay forwards ScriptEvents from QuickJS → Tauri event bus.
    pub(crate) fn init<R: tauri::Runtime>(
        &mut self,
        app_handle: tauri::AppHandle<R>,
    ) -> ModScriptQueue {
        self.set_runtime_state(ModRuntimeState::Initializing);

        let (scripts, mut rx) = ModScriptQueue::channel();
        self.scripts = Some(scripts.clone());

        // Outgoing event channel: QuickJS closures → event relay → Tauri
        // Using std::sync::mpsc because QuickJS closures are !Send and run on a std thread
        let (event_tx, event_rx) = std::sync::mpsc::channel::<ScriptEvent>();
        let state_for_script = self.runtime_state.clone();
        let state_for_health = self.runtime_state.clone();

        // ── QuickJS runtime thread ──
        std::thread::spawn(move || {
            let rt = match rquickjs::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(target: "mods", "[ModManager] Failed to create QuickJS runtime: {}", e);
                    state_for_script.store(ModRuntimeState::Failed.as_u8(), Ordering::SeqCst);
                    return;
                }
            };
            let ctx = match rquickjs::Context::full(&rt) {
                Ok(ctx) => ctx,
                Err(e) => {
                    tracing::error!(target: "mods", "[ModManager] Failed to create QuickJS context: {}", e);
                    state_for_script.store(ModRuntimeState::Failed.as_u8(), Ordering::SeqCst);
                    return;
                }
            };
            let script_deadline = install_script_interrupt_handler(&rt);

            // Register the Kokoro API with the event sender
            ctx.with(|ctx| {
                if let Err(e) = crate::mods::api::register_api(&ctx, event_tx) {
                    eprintln!("Failed to register Kokoro API: {}", e);
                }
            });

            state_for_script.store(ModRuntimeState::Running.as_u8(), Ordering::SeqCst);

            // Event loop: process incoming script commands
            while let Some(cmd) = rx.blocking_recv() {
                run_script_command(&ctx, &script_deadline, cmd, SCRIPT_EVENT_BUDGET);
            }

            state_for_script.store(ModRuntimeState::Disconnected.as_u8(), Ordering::SeqCst);
            tracing::info!(target: "mods", "[ModManager] Script thread shut down.");
        });

        // ── Event relay task: forward ScriptEvents to Tauri event bus ──
        let handle = app_handle.clone();
        std::thread::spawn(move || {
            while let Ok(event) = event_rx.recv() {
                match event {
                    ScriptEvent::Emit { event, payload } => {
                        let _ = handle.emit(
                            "mod:script-event",
                            ScriptEventPayload {
                                event: event.clone(),
                                payload: payload.clone(),
                            },
                        );
                        tracing::info!(
                            target: "mods",
                            "[ModManager] Script event '{}' forwarded to frontend",
                            event
                        );
                    }
                    ScriptEvent::UiSend { component, data } => {
                        let _ = handle.emit(
                            "mod:ui-message",
                            UiMessagePayload {
                                component: component.clone(),
                                payload: data.clone(),
                            },
                        );
                        tracing::info!(target: "mods", "[ModManager] UI message sent to component '{}'", component);
                    }
                    ScriptEvent::PlayCue { cue } => {
                        let _ = handle.emit("chat-cue", CuePayload { cue: cue.clone() });
                        tracing::info!(target: "mods", "[ModManager] Cue triggered '{}'", cue);
                    }
                }
            }
            tracing::info!(target: "mods", "[ModManager] Event relay shut down.");
        });

        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            if ModRuntimeState::from_u8(state_for_health.load(Ordering::SeqCst))
                == ModRuntimeState::Initializing
            {
                state_for_health.store(ModRuntimeState::Failed.as_u8(), Ordering::SeqCst);
                tracing::error!(
                    target: "mods",
                    "[ModManager] Runtime health check timed out during initialization"
                );
            }
        });
        scripts
    }

    pub fn scan_mods(&mut self) -> Vec<ModManifest> {
        tracing::info!(
            target: "mods",
            "[ModManager] Current Working Directory: {:?}",
            std::env::current_dir()
        );
        tracing::info!(target: "mods", "[ModManager] Scanning mods from: {:?}", self.mods_path);
        let mut mods = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.mods_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let manifest_path = path.join("mod.json");
                    if manifest_path.exists() {
                        tracing::info!(target: "mods", "[ModManager] Found manifest at: {:?}", manifest_path);
                        if let Ok(content) = fs::read_to_string(&manifest_path) {
                            match serde_json::from_str::<ModManifest>(&content) {
                                Ok(manifest) => {
                                    if let Err(error) = validate_manifest_capabilities(&manifest) {
                                        eprintln!(
                                            "Failed to validate mod.json in {:?}: {}",
                                            path, error
                                        );
                                        tracing::error!(
                                            target: "mods",
                                            "[ModManager] ERROR validating mod.json in {:?}: {}",
                                            path,
                                            error
                                        );
                                        continue;
                                    }
                                    tracing::info!(
                                        target: "mods",
                                        "[ModManager] Successfully loaded mod: {}",
                                        manifest.id
                                    );
                                    mods.push(manifest.clone());
                                    self.loaded_mods.insert(manifest.id.clone(), manifest);
                                }
                                Err(e) => {
                                    eprintln!("Failed to parse mod.json in {:?}: {}", path, e);
                                    tracing::error!(
                                        target: "mods",
                                        "[ModManager] ERROR parsing mod.json in {:?}: {}",
                                        path, e
                                    );
                                }
                            }
                        }
                    } else {
                        tracing::info!(target: "mods", "[ModManager] No mod.json in check: {:?}", path);
                    }
                }
            }
        } else {
            tracing::error!(
                target: "mods",
                "[ModManager] Failed to read mods directory: {:?}",
                self.mods_path
            );
        }
        tracing::info!(target: "mods", "[ModManager] Total mods loaded: {}", mods.len());
        mods
    }

    /// Load a mod: parse theme.json, layout.json, register components, run scripts.
    pub async fn load_mod<R: tauri::Runtime>(
        &mut self,
        mod_id: &str,
        app_handle: &tauri::AppHandle<R>,
    ) -> Result<(), String> {
        let manifest = self
            .loaded_mods
            .get(mod_id)
            .cloned()
            .ok_or_else(|| format!("Mod '{}' not found", mod_id))?;

        let mod_dir = self.mods_path.join(mod_id);

        // ── 1. Load theme.json ──
        if let Some(theme_path) = &manifest.theme {
            let full_path = match safe_join(&mod_dir, theme_path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(target: "mods", "[ModManager] Rejected theme path '{}': {}", theme_path, e);
                    return Err(e);
                }
            };
            match fs::read_to_string(&full_path) {
                Ok(content) => match serde_json::from_str::<ModThemeJson>(&content) {
                    Ok(mut theme) => {
                        // Prefix asset paths with mod:// protocol
                        if let Some(ref mut assets) = theme.assets {
                            if let Some(ref mut bg) = assets.background {
                                if !bg.starts_with("http") && !bg.starts_with("mod://") {
                                    *bg = format!("mod://{}/{}", mod_id, bg);
                                }
                            }
                            if let Some(ref mut fonts) = assets.fonts {
                                for font in fonts.iter_mut() {
                                    if !font.starts_with("http") && !font.starts_with("mod://") {
                                        *font = format!("mod://{}/{}", mod_id, font);
                                    }
                                }
                            }
                        }

                        self.active_theme = Some(theme.clone());
                        let _ = app_handle.emit("mod:theme-override", &theme);
                        tracing::info!(target: "mods", "[ModManager] Theme loaded for mod '{}'", mod_id);
                    }
                    Err(e) => {
                        tracing::error!(target: "mods", "[ModManager] Failed to parse theme.json: {}", e)
                    }
                },
                Err(e) => {
                    tracing::error!(target: "mods", "[ModManager] Failed to read theme.json: {}", e)
                }
            }
        }

        // ── 2. Load layout.json ──
        if let Some(layout_path) = &manifest.layout {
            let full_path = match safe_join(&mod_dir, layout_path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(target: "mods", "[ModManager] Rejected layout path '{}': {}", layout_path, e);
                    return Err(e);
                }
            };
            match fs::read_to_string(&full_path) {
                Ok(content) => match serde_json::from_str::<JsonValue>(&content) {
                    Ok(layout) => {
                        self.active_layout = Some(layout.clone());
                        let _ = app_handle.emit("mod:layout-override", &layout);
                        tracing::info!(target: "mods", "[ModManager] Layout loaded for mod '{}'", mod_id);
                    }
                    Err(e) => {
                        tracing::error!(target: "mods", "[ModManager] Failed to parse layout.json: {}", e)
                    }
                },
                Err(e) => {
                    tracing::error!(target: "mods", "[ModManager] Failed to read layout.json: {}", e)
                }
            }
        }

        // ── 3. Register components ──
        if !manifest.components.is_empty() {
            let component_map: HashMap<String, String> = manifest
                .components
                .iter()
                .map(|(slot, path)| (slot.clone(), format!("mod://{}/{}", mod_id, path)))
                .collect();
            let _ = app_handle.emit("mod:components-register", &component_map);
            tracing::info!(
                target: "mods",
                "[ModManager] Registered {} components for mod '{}'",
                component_map.len(),
                mod_id
            );
        }

        // Collect scripts before replacing the listeners so chat events cannot
        // observe a partially installed MOD.
        if let Some(queue) = self.scripts.as_ref().filter(|_| self.runtime_ready()) {
            let paths = if manifest.scripts.is_empty() {
                manifest.entry.as_slice()
            } else {
                &manifest.scripts
            };
            let mut scripts = Vec::new();
            for path in paths {
                let code = safe_join(&mod_dir, path)
                    .and_then(|file| fs::read_to_string(file).map_err(|err| err.to_string()));
                match code {
                    Ok(code) => scripts.push((path.clone(), code)),
                    Err(error) => {
                        tracing::error!(target: "mods", "[ModManager] Could not load script '{}': {}", path, error)
                    }
                }
            }
            queue
                .replace_listeners(Some(ScriptModule {
                    id: mod_id.into(),
                    scripts,
                }))
                .await?;
        } else {
            tracing::warn!(
                target: "mods",
                "[ModManager] Skip script installation for mod '{}' because runtime state is '{}'",
                mod_id,
                self.runtime_state().as_str()
            );
        }

        if let Some(hooks) = app_handle.try_state::<HookRuntime>() {
            self.active_mod_id = Some(mod_id.to_string());
            hooks
                .emit_best_effort(
                    &HookEvent::OnModLoaded,
                    &build_mod_hook_payload(&manifest, "loaded"),
                )
                .await;
        } else {
            self.active_mod_id = Some(mod_id.to_string());
        }

        Ok(())
    }

    /// Dispatch an engine event to QuickJS listeners (registered via Kokoro.on).
    pub async fn dispatch_event(
        &self,
        event: &str,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        if !self.runtime_ready() {
            return Err(format!(
                "Script runtime not ready (state={})",
                self.runtime_state().as_str()
            ));
        }

        let Some(queue) = &self.scripts else {
            return Err("Script runtime sender missing".to_string());
        };

        queue.dispatch_event(event, payload).await.inspect_err(|_| {
            self.set_runtime_state(ModRuntimeState::Disconnected);
        })
    }

    pub fn runtime_status_module_tag(&self) -> Option<String> {
        if self.runtime_ready() {
            Some("mods".to_string())
        } else {
            Some(format!("mods:{}", self.runtime_state().as_str()))
        }
    }

    pub fn runtime_state_text(&self) -> &'static str {
        self.runtime_state().as_str()
    }

    pub fn runtime_status_summary(&self) -> (String, bool, bool) {
        let state = self.runtime_state();
        let ready = self.runtime_ready();
        let degraded = matches!(
            state,
            ModRuntimeState::Failed | ModRuntimeState::Disconnected
        );
        (state.as_str().to_string(), ready, degraded)
    }

    pub fn runtime_is_running(&self) -> bool {
        self.runtime_state() == ModRuntimeState::Running
    }

    pub fn runtime_is_initialized(&self) -> bool {
        self.runtime_state() != ModRuntimeState::Uninitialized
    }

    pub fn get_active_theme(&self) -> Option<&ModThemeJson> {
        self.active_theme.as_ref()
    }

    pub fn get_active_layout(&self) -> Option<&JsonValue> {
        self.active_layout.as_ref()
    }

    /// 卸载当前活跃的 Mod（清除主题、布局、组件），恢复原生模式
    pub async fn unload_mod<R: tauri::Runtime>(&mut self, app_handle: &tauri::AppHandle<R>) {
        let manifest = self
            .active_mod_id
            .as_deref()
            .and_then(|id| self.loaded_mods.get(id))
            .cloned();
        self.active_mod_id = None;
        self.active_theme = None;
        self.active_layout = None;
        if let Some(queue) = &self.scripts {
            if let Err(error) = queue.replace_listeners(None).await {
                tracing::warn!(target: "mods", "Failed to reset MOD listeners during unload: {}", error);
            }
        }
        let _ = app_handle.emit("mod:unload", ());
        if let (Some(hooks), Some(manifest)) = (app_handle.try_state::<HookRuntime>(), manifest) {
            hooks
                .emit_best_effort(
                    &HookEvent::OnModUnloaded,
                    &build_mod_hook_payload(&manifest, "unloaded"),
                )
                .await;
        }
        tracing::info!(target: "mods", "[ModManager] Active mod unloaded, native mode restored");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HookPayload;
    use std::fs;
    use tempfile::TempDir;

    fn script_context(code: &str) -> (rquickjs::Context, Arc<Mutex<Option<Instant>>>) {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        let deadline = install_script_interrupt_handler(&runtime);
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        context.with(|ctx| {
            crate::mods::api::register_api(&ctx, event_tx).unwrap();
            ctx.eval::<(), _>(code).unwrap();
        });
        (context, deadline)
    }

    fn received(context: &rquickjs::Context) -> JsonValue {
        let received =
            context.with(|ctx| ctx.eval::<String, _>("JSON.stringify(received)").unwrap());
        serde_json::from_str(&received).unwrap()
    }

    #[tokio::test]
    async fn queued_completions_reach_the_original_mod_before_unload_or_switch() {
        for switch in [false, true] {
            let (queue, mut rx) = ModScriptQueue::channel();
            let (context, deadline) = script_context(
                r#"
                globalThis.received = [];
                Kokoro.on('chat', event => received.push(['old-chat', event.delta]));
                Kokoro.on('chat-done', event => received.push(['old-done', event.turn_id]));
            "#,
            );
            queue.enqueue_chat_event("chat-turn-delta", serde_json::json!({"delta": "before"}));
            queue.enqueue_chat_event(
                "chat-turn-finish",
                serde_json::json!({"turn_id": "original"}),
            );
            let replacement = switch.then(|| ScriptModule {
                id: "replacement".into(),
                scripts: vec![(
                    "test.js".into(),
                    r#"
                    Kokoro.on('init', event => received.push(['init', event.modId]));
                    Kokoro.on('chat-done', event => received.push(['new-done', event.turn_id]));
                "#
                    .into(),
                )],
            });
            let mut replace = tokio_test::task::spawn(queue.replace_listeners(replacement));
            assert!(
                replace.poll().is_pending(),
                "wait for QuickJS to install the new listeners"
            );
            queue.enqueue_chat_event("chat-turn-finish", serde_json::json!({"turn_id": "later"}));

            // The consumer has not run yet; reset or replacement must not overtake
            // the original notifications already waiting in the same queue.
            while let Ok(command) = rx.try_recv() {
                run_script_command(&context, &deadline, command, SCRIPT_EVENT_BUDGET);
            }
            replace.await.unwrap();
            let mut expected = vec![["old-chat", "before"], ["old-done", "original"]];
            if switch {
                expected.extend([["init", "replacement"], ["new-done", "later"]]);
            }
            assert_eq!(received(&context), serde_json::json!(expected));
        }
    }

    #[tokio::test]
    async fn full_queue_and_slow_listeners_do_not_block_chat_or_lose_final_text() {
        use crate::chat::runtime::{relay_chat_event, ChatRuntimeState};

        let (queue, mut rx) = ModScriptQueue::channel();
        let journal = ChatRuntimeState::default();
        for id in ["first-done", "second-done"] {
            let request = serde_json::from_value(
                serde_json::json!({"client_request_id": id, "message": "question"}),
            )
            .unwrap();
            journal.begin(&request).unwrap();
        }
        let relay = |name, payload| relay_chat_event(Some(&journal), Some(&queue), name, payload);
        relay(
            "chat-turn-start",
            serde_json::json!({"client_request_id": "first-done", "turn_id": "first-done"}),
        );
        let mut manager = ModManager::new("unused");
        manager.scripts = Some(queue.clone());
        let manager = tokio::sync::Mutex::new(manager);
        let _guard = manager.lock().await;
        let (context, deadline) = script_context(
            r#"
            globalThis.received = [];
            function slow() {
                const start = Date.now();
                while (Date.now() - start < 1000) {}
                received.push('ran beyond budget');
            }
            Kokoro.on('chat', event => {
                if (event.delta === 'slow') slow();
                if (event.delta !== 'filler') received.push(event.delta);
            });
            Kokoro.on('cue', event => received.push(event.cue));
            Kokoro.on('chat-done', event => {
                if (event.turn_id === 'first-done') slow();
            });
            Kokoro.on('chat-done', event => received.push([event.turn_id, event.text]));
        "#,
        );

        queue.enqueue_chat_event("chat-turn-delta", serde_json::json!({"delta": "slow"}));
        queue.enqueue_chat_event("chat-cue", serde_json::json!({"cue": "smile"}));
        relay(
            "chat-turn-delta",
            serde_json::json!({"turn_id": "first-done", "delta": "second"}),
        );
        for _ in 3..SCRIPT_EVENT_CAPACITY {
            queue.enqueue_chat_event("chat-turn-delta", serde_json::json!({"delta": "filler"}));
        }
        for _ in 0..(SCRIPT_EVENT_CAPACITY * 4) {
            relay(
                "chat-turn-delta",
                serde_json::json!({"turn_id": "first-done", "delta": " overflow"}),
            );
        }
        relay(
            "chat-turn-text-complete",
            serde_json::json!({"turn_id": "first-done", "text": "second fragment with the final suffix\n完整的答复"}),
        );
        for id in ["first-done", "second-done"] {
            relay(
                "chat-turn-finish",
                serde_json::json!({"client_request_id": id, "turn_id": id, "status": "completed"}),
            );
        }

        // Production finishes enqueueing while the manager is locked and the
        // consumer has not freed a single slot. Only the two completions bypass the limit.
        assert_eq!(rx.len(), SCRIPT_EVENT_CAPACITY + 2);
        while let Ok(command) = rx.try_recv() {
            run_script_command(&context, &deadline, command, Duration::from_millis(75));
        }
        assert_eq!(
            received(&context),
            serde_json::json!([
                "smile",
                "second",
                [
                    "first-done",
                    "second fragment with the final suffix\n完整的答复"
                ],
                ["second-done", ""]
            ]),
        );
        assert_eq!(queue.event_slots.available_permits(), SCRIPT_EVENT_CAPACITY);
    }

    /// Helper: create a temp dir with a fake mod inside
    fn setup_temp_mods() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let mods_path = tmp.path().to_path_buf();

        // Create a fake mod directory with mod.json
        let mod_dir = mods_path.join("test-mod");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(
            mod_dir.join("mod.json"),
            r#"{
                "id": "test-mod",
                "name": "Test Mod",
                "version": "0.1.0",
                "description": "A test mod",
                "components": { "TestPanel": "components/TestPanel.html" },
                "scripts": ["scripts/main.js"]
            }"#,
        )
        .unwrap();

        (tmp, mods_path)
    }

    #[test]
    fn build_mod_hook_payload_reflects_manifest_shape() {
        let manifest = ModManifest {
            id: "test-mod".to_string(),
            name: "Test Mod".to_string(),
            version: "0.1.0".to_string(),
            description: "A test mod".to_string(),
            engine_version: None,
            layout: Some("layout.json".to_string()),
            theme: Some("theme.json".to_string()),
            components: HashMap::from([(
                "TestPanel".to_string(),
                "components/TestPanel.html".to_string(),
            )]),
            scripts: vec!["scripts/main.js".to_string()],
            permissions: vec![],
            capabilities: vec![],
            entry: None,
            ui_entry: None,
        };

        let HookPayload::Mod(payload) = build_mod_hook_payload(&manifest, "loaded") else {
            panic!("expected mod payload");
        };

        assert_eq!(payload.mod_id, "test-mod");
        assert_eq!(payload.stage, "loaded");
        assert!(payload.has_theme);
        assert!(payload.has_layout);
        assert_eq!(payload.component_count, 1);
        assert_eq!(payload.script_count, 1);
    }

    #[test]
    fn scan_mods_discovers_valid_mod() {
        let (_tmp, mods_path) = setup_temp_mods();
        let mut manager = ModManager::new(&mods_path);

        let mods = manager.scan_mods();

        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].id, "test-mod");
        assert_eq!(mods[0].name, "Test Mod");
        assert!(mods[0].components.contains_key("TestPanel"));
    }

    #[test]
    fn scan_mods_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let mut manager = ModManager::new(tmp.path());

        let mods = manager.scan_mods();
        assert!(mods.is_empty());
    }

    #[test]
    fn scan_mods_nonexistent_dir() {
        let mut manager = ModManager::new("/nonexistent/path/to/mods");
        let mods = manager.scan_mods();
        assert!(mods.is_empty());
    }

    #[test]
    fn scan_mods_updates_loaded_mods() {
        let (_tmp, mods_path) = setup_temp_mods();
        let mut manager = ModManager::new(&mods_path);

        assert!(manager.loaded_mods.is_empty());
        manager.scan_mods();
        assert!(manager.loaded_mods.contains_key("test-mod"));
    }

    #[test]
    fn scan_mods_ignores_dirs_without_manifest() {
        let tmp = TempDir::new().unwrap();
        let mods_path = tmp.path().to_path_buf();

        // Create a directory without mod.json
        fs::create_dir_all(mods_path.join("no-manifest")).unwrap();
        // Create a directory with invalid mod.json
        let invalid_dir = mods_path.join("invalid-mod");
        fs::create_dir_all(&invalid_dir).unwrap();
        fs::write(invalid_dir.join("mod.json"), "{ not valid json }").unwrap();

        let mut manager = ModManager::new(&mods_path);
        let mods = manager.scan_mods();
        assert!(mods.is_empty());
    }

    #[test]
    fn scan_mods_multiple() {
        let tmp = TempDir::new().unwrap();
        let mods_path = tmp.path().to_path_buf();

        for name in ["mod-a", "mod-b", "mod-c"] {
            let mod_dir = mods_path.join(name);
            fs::create_dir_all(&mod_dir).unwrap();
            fs::write(
                mod_dir.join("mod.json"),
                format!(
                    r#"{{
                        "id": "{}",
                        "name": "Mod {}",
                        "version": "1.0.0",
                        "description": "desc"
                    }}"#,
                    name,
                    name.to_uppercase()
                ),
            )
            .unwrap();
        }

        let mut manager = ModManager::new(&mods_path);
        let mods = manager.scan_mods();
        assert_eq!(mods.len(), 3);
    }

    #[test]
    fn new_manager_has_no_state() {
        let manager = ModManager::new("/any/path");
        assert!(manager.loaded_mods.is_empty());
        assert!(manager.scripts.is_none());
        assert!(manager.active_theme.is_none());
        assert!(manager.active_layout.is_none());
    }

    #[test]
    fn production_init_path_avoids_runtime_expect_panics() {
        let source = include_str!("manager.rs");
        let production_part = source.split("#[cfg(test)]").next().unwrap_or(source);

        assert!(
            !production_part.contains("rquickjs::Runtime::new().expect("),
            "production init path should not panic on runtime creation"
        );
        assert!(
            !production_part.contains("rquickjs::Context::full(&rt).expect("),
            "production init path should not panic on context creation"
        );
    }

    #[test]
    fn script_execution_budget_interrupts_an_infinite_loop() {
        let runtime = rquickjs::Runtime::new().expect("runtime should initialize");
        let context = rquickjs::Context::full(&runtime).expect("context should initialize");
        let deadline = install_script_interrupt_handler(&runtime);

        let result = eval_script_with_budget(
            &context,
            &deadline,
            "while (true) {}",
            Duration::from_millis(50),
        );

        assert!(result.is_err(), "an infinite script must be interrupted");
        assert!(
            result
                .expect_err("infinite script must return an error")
                .contains("timed out"),
            "the error should identify the execution budget"
        );
    }

    #[test]
    fn script_execution_budget_allows_a_short_script() {
        let runtime = rquickjs::Runtime::new().expect("runtime should initialize");
        let context = rquickjs::Context::full(&runtime).expect("context should initialize");
        let deadline = install_script_interrupt_handler(&runtime);

        let result = eval_script_with_budget(
            &context,
            &deadline,
            "let result = 1 + 1;",
            Duration::from_millis(50),
        );

        assert!(
            result.is_ok(),
            "a short script should complete successfully"
        );
    }

    #[test]
    fn safe_join_normal_relative_path() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create a file inside the base directory
        let file_path = base.join("test.txt");
        fs::write(&file_path, "test content").unwrap();

        // safe_join with a normal relative path should succeed
        let result = safe_join(base, "test.txt");
        assert!(
            result.is_ok(),
            "safe_join should accept normal relative paths"
        );
        let resolved = result.unwrap();
        // Compare canonical forms since macOS adds /private prefix
        let expected_canonical = file_path.canonicalize().unwrap();
        assert_eq!(
            resolved, expected_canonical,
            "safe_join should resolve to the correct absolute path"
        );
    }

    #[test]
    fn safe_join_single_level_traversal_rejected() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create a subdirectory
        let subdir = base.join("subdir");
        fs::create_dir_all(&subdir).unwrap();

        // Create a file outside the base directory (in parent)
        let parent_file = base.parent().unwrap().join("outside.txt");
        fs::write(&parent_file, "outside").unwrap();

        // safe_join with ../ should fail
        let result = safe_join(&subdir, "../outside.txt");
        assert!(
            result.is_err(),
            "safe_join should reject paths that escape the base directory"
        );
    }

    #[test]
    fn safe_join_deep_traversal_rejected() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create nested subdirectories
        let nested = base.join("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();

        // safe_join with deep traversal should fail
        let result = safe_join(&nested, "../../../../etc/passwd");
        assert!(
            result.is_err(),
            "safe_join should reject deep path traversal attempts"
        );
    }

    #[test]
    fn safe_join_nonexistent_file_rejected() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // safe_join with a nonexistent file should fail (canonicalize fails)
        let result = safe_join(base, "nonexistent.txt");
        assert!(
            result.is_err(),
            "safe_join should reject nonexistent files (canonicalize fails)"
        );
    }

    #[test]
    fn safe_join_nested_valid_path() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create nested directories and a file
        let nested_dir = base.join("a").join("b");
        fs::create_dir_all(&nested_dir).unwrap();
        let nested_file = nested_dir.join("file.txt");
        fs::write(&nested_file, "nested").unwrap();

        // safe_join with a nested relative path should succeed
        let result = safe_join(base, "a/b/file.txt");
        assert!(
            result.is_ok(),
            "safe_join should accept nested relative paths"
        );
        let resolved = result.unwrap();
        // Compare canonical forms since macOS adds /private prefix
        let expected_canonical = nested_file.canonicalize().unwrap();
        assert_eq!(
            resolved, expected_canonical,
            "safe_join should resolve nested paths correctly"
        );
    }

    #[test]
    fn safe_join_traversal_with_dot_dot_in_middle() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create structure: base/a/b and base/c
        let a_dir = base.join("a");
        let b_dir = a_dir.join("b");
        fs::create_dir_all(&b_dir).unwrap();
        let c_dir = base.join("c");
        fs::create_dir_all(&c_dir).unwrap();

        // Create a file in c
        let c_file = c_dir.join("file.txt");
        fs::write(&c_file, "content").unwrap();

        // Try to access ../c/file.txt from b (should escape base)
        let result = safe_join(&b_dir, "../../c/file.txt");
        assert!(
            result.is_err(),
            "safe_join should reject paths that escape base even with valid target"
        );
    }
}
