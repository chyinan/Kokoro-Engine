use crate::hooks::{HookEvent, HookPayload, HookRuntime, ModHookPayload};
use crate::mods::api::ScriptEvent;
use crate::mods::manifest::ModManifest;
use crate::mods::theme::ModThemeJson;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager};
use tokio::sync::{mpsc, oneshot};

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

// Messages sent to the dedicated QuickJS script thread
pub enum ScriptCommand {
    Eval {
        code: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Dispatch an engine event to QuickJS listeners registered via Kokoro.on()
    DispatchEvent {
        event: String,
        payload: serde_json::Value,
    },
    Shutdown,
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
pub struct ModManager {
    pub mods_path: PathBuf,
    pub loaded_mods: HashMap<String, ModManifest>,
    pub script_tx: Option<mpsc::Sender<ScriptCommand>>,
    /// Currently active theme loaded from a mod's theme.json
    pub active_theme: Option<ModThemeJson>,
    /// Currently active layout loaded from a mod's layout.json
    pub active_layout: Option<JsonValue>,
}

impl ModManager {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            mods_path: path.as_ref().to_path_buf(),
            loaded_mods: HashMap::new(),
            script_tx: None,
            active_theme: None,
            active_layout: None,
        }
    }

    /// Spawn the QuickJS runtime thread and the event relay task.
    /// The event relay forwards ScriptEvents from QuickJS → Tauri event bus.
    pub fn init<R: tauri::Runtime>(&mut self, app_handle: tauri::AppHandle<R>) {
        let (tx, mut rx) = mpsc::channel::<ScriptCommand>(32);
        self.script_tx = Some(tx);

        // Outgoing event channel: QuickJS closures → event relay → Tauri
        // Using std::sync::mpsc because QuickJS closures are !Send and run on a std thread
        let (event_tx, event_rx) = std::sync::mpsc::channel::<ScriptEvent>();

        // ── QuickJS runtime thread ──
        std::thread::spawn(move || {
            let rt = rquickjs::Runtime::new().expect("Failed to create QuickJS runtime");
            let ctx = rquickjs::Context::full(&rt).expect("Failed to create QuickJS context");

            // Register the Kokoro API with the event sender
            ctx.with(|ctx| {
                if let Err(e) = crate::mods::api::register_api(&ctx, event_tx) {
                    eprintln!("Failed to register Kokoro API: {}", e);
                }
            });

            // Event loop: process incoming script commands
            while let Some(cmd) = rx.blocking_recv() {
                match cmd {
                    ScriptCommand::Eval { code, reply } => {
                        let result = ctx.with(|ctx| {
                            ctx.eval::<(), _>(code.as_str())
                                .map_err(|e| format!("{}", e))
                        });
                        let _ = reply.send(result);
                    }
                    ScriptCommand::DispatchEvent { event, payload } => {
                        // Call globalThis.__dispatch(event, payload) in QuickJS
                        ctx.with(|ctx| {
                            // 使用 serde_json 双重编码确保安全，避免字符串拼接注入
                            let event_json = serde_json::to_string(&event).unwrap_or_default();
                            let payload_str = serde_json::to_string(&payload).unwrap_or_default();
                            // 二次编码：将 JSON 字符串本身编码为 JS 字符串字面量
                            let payload_escaped =
                                serde_json::to_string(&payload_str).unwrap_or_default();
                            let dispatch_code = format!(
                                "globalThis.__dispatch({}, JSON.parse({}));",
                                event_json, payload_escaped
                            );
                            if let Err(_e) = ctx.eval::<(), _>(dispatch_code.as_str()) {
                                // Silently ignore — expected when no mods register listeners
                            }
                        });
                    }
                    ScriptCommand::Shutdown => break,
                }
            }
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
                                        eprintln!("Failed to validate mod.json in {:?}: {}", path, error);
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

        // ── 4. Execute scripts ──
        let scripts_to_run: Vec<String> = if !manifest.scripts.is_empty() {
            manifest.scripts.clone()
        } else if let Some(ref entry) = manifest.entry {
            vec![entry.clone()]
        } else {
            vec![]
        };

        for script_path in &scripts_to_run {
            let full_path = match safe_join(&mod_dir, script_path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(target: "mods", "[ModManager] Rejected script path '{}': {}", script_path, e);
                    continue;
                }
            };
            match fs::read_to_string(&full_path) {
                Ok(code) => {
                    if let Some(tx) = &self.script_tx {
                        let (reply_tx, reply_rx) = oneshot::channel();
                        if let Err(e) = tx
                            .send(ScriptCommand::Eval {
                                code,
                                reply: reply_tx,
                            })
                            .await
                        {
                            tracing::error!(target: "mods", "[ModManager] Failed to send script to runtime: {}", e);
                            continue;
                        }
                        match reply_rx.await {
                            Ok(Ok(())) => {
                                tracing::info!(
                                    target: "mods",
                                    "[ModManager] Script '{}' executed for mod '{}'",
                                    script_path, mod_id
                                );
                            }
                            Ok(Err(e)) => {
                                tracing::error!(target: "mods", "[ModManager] Script '{}' error: {}", script_path, e);
                            }
                            Err(e) => {
                                tracing::error!(target: "mods", "[ModManager] Script thread dropped: {}", e);
                            }
                        }
                    } else {
                        tracing::error!(target: "mods", "[ModManager] Script runtime not initialized");
                    }
                }
                Err(e) => {
                    tracing::error!(
                        target: "mods",
                        "[ModManager] Failed to read script '{}': {}",
                        script_path, e
                    );
                }
            }
        }

        // ── 5. Dispatch lifecycle init event ──
        if let Some(tx) = &self.script_tx {
            let _ = tx
                .send(ScriptCommand::DispatchEvent {
                    event: "init".to_string(),
                    payload: serde_json::json!({ "modId": mod_id }),
                })
                .await;
            tracing::info!(
                target: "mods",
                "[ModManager] Dispatched 'init' lifecycle event for mod '{}'",
                mod_id
            );
        }

        if let Some(hooks) = app_handle.try_state::<HookRuntime>() {
            hooks
                .emit_best_effort(
                    &HookEvent::OnModLoaded,
                    &build_mod_hook_payload(&manifest, "loaded"),
                )
                .await;
        }

        Ok(())
    }

    /// Dispatch an engine event to QuickJS listeners (registered via Kokoro.on).
    pub async fn dispatch_event(
        &self,
        event: &str,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        if let Some(tx) = &self.script_tx {
            tx.send(ScriptCommand::DispatchEvent {
                event: event.to_string(),
                payload,
            })
            .await
            .map_err(|e| format!("Failed to dispatch event: {}", e))
        } else {
            Err("Script runtime not initialized".into())
        }
    }

    pub fn get_active_theme(&self) -> Option<&ModThemeJson> {
        self.active_theme.as_ref()
    }

    pub fn get_active_layout(&self) -> Option<&JsonValue> {
        self.active_layout.as_ref()
    }

    /// 卸载当前活跃的 Mod（清除主题、布局、组件），恢复原生模式
    pub async fn unload_mod<R: tauri::Runtime>(&mut self, app_handle: &tauri::AppHandle<R>) {
        let manifest = self.loaded_mods.values().next().cloned();
        self.active_theme = None;
        self.active_layout = None;
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
        assert!(manager.script_tx.is_none());
        assert!(manager.active_theme.is_none());
        assert!(manager.active_layout.is_none());
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
