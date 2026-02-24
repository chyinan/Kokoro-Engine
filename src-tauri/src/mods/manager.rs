use crate::mods::api::ScriptEvent;
use crate::mods::manifest::ModManifest;
use crate::mods::theme::ModThemeJson;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};

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

/// Payload for character expression events
#[derive(serde::Serialize, Clone)]
struct ExpressionPayload {
    expression: String,
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
                            let payload_str = serde_json::to_string(&payload).unwrap_or_default();
                            // Escape for embedding in a JS backtick template literal
                            let escaped = payload_str
                                .replace('\\', "\\\\")
                                .replace('`', "\\`")
                                .replace("${", "\\${");
                            let dispatch_code = format!(
                                "globalThis.__dispatch(\"{}\", JSON.parse(`{}`));",
                                event.replace('"', "\\\""),
                                escaped
                            );
                            if let Err(_e) = ctx.eval::<(), _>(dispatch_code.as_str()) {
                                // Silently ignore — expected when no mods register listeners
                            }
                        });
                    }
                    ScriptCommand::Shutdown => break,
                }
            }
            println!("[ModManager] Script thread shut down.");
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
                        println!(
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
                        println!("[ModManager] UI message sent to component '{}'", component);
                    }
                    ScriptEvent::SetExpression { expression } => {
                        let _ = handle.emit(
                            "chat-expression",
                            ExpressionPayload {
                                expression: expression.clone(),
                            },
                        );
                        println!("[ModManager] Expression set to '{}'", expression);
                    }
                }
            }
            println!("[ModManager] Event relay shut down.");
        });
    }

    pub fn scan_mods(&mut self) -> Vec<ModManifest> {
        println!(
            "[ModManager] Current Working Directory: {:?}",
            std::env::current_dir()
        );
        println!("[ModManager] Scanning mods from: {:?}", self.mods_path);
        let mut mods = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.mods_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let manifest_path = path.join("mod.json");
                    if manifest_path.exists() {
                        println!("[ModManager] Found manifest at: {:?}", manifest_path);
                        if let Ok(content) = fs::read_to_string(&manifest_path) {
                            match serde_json::from_str::<ModManifest>(&content) {
                                Ok(manifest) => {
                                    println!(
                                        "[ModManager] Successfully loaded mod: {}",
                                        manifest.id
                                    );
                                    mods.push(manifest.clone());
                                    self.loaded_mods.insert(manifest.id.clone(), manifest);
                                }
                                Err(e) => {
                                    eprintln!("Failed to parse mod.json in {:?}: {}", path, e);
                                    println!(
                                        "[ModManager] ERROR parsing mod.json in {:?}: {}",
                                        path, e
                                    );
                                }
                            }
                        }
                    } else {
                        println!("[ModManager] No mod.json in check: {:?}", path);
                    }
                }
            }
        } else {
            println!(
                "[ModManager] Failed to read mods directory: {:?}",
                self.mods_path
            );
        }
        println!("[ModManager] Total mods loaded: {}", mods.len());
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
            let full_path = mod_dir.join(theme_path);
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
                        println!("[ModManager] Theme loaded for mod '{}'", mod_id);
                    }
                    Err(e) => eprintln!("[ModManager] Failed to parse theme.json: {}", e),
                },
                Err(e) => eprintln!("[ModManager] Failed to read theme.json: {}", e),
            }
        }

        // ── 2. Load layout.json ──
        if let Some(layout_path) = &manifest.layout {
            let full_path = mod_dir.join(layout_path);
            match fs::read_to_string(&full_path) {
                Ok(content) => match serde_json::from_str::<JsonValue>(&content) {
                    Ok(layout) => {
                        self.active_layout = Some(layout.clone());
                        let _ = app_handle.emit("mod:layout-override", &layout);
                        println!("[ModManager] Layout loaded for mod '{}'", mod_id);
                    }
                    Err(e) => eprintln!("[ModManager] Failed to parse layout.json: {}", e),
                },
                Err(e) => eprintln!("[ModManager] Failed to read layout.json: {}", e),
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
            println!(
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
            let full_path = mod_dir.join(script_path);
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
                            eprintln!("[ModManager] Failed to send script to runtime: {}", e);
                            continue;
                        }
                        match reply_rx.await {
                            Ok(Ok(())) => {
                                println!(
                                    "[ModManager] Script '{}' executed for mod '{}'",
                                    script_path, mod_id
                                );
                            }
                            Ok(Err(e)) => {
                                eprintln!("[ModManager] Script '{}' error: {}", script_path, e);
                            }
                            Err(e) => {
                                eprintln!("[ModManager] Script thread dropped: {}", e);
                            }
                        }
                    } else {
                        eprintln!("[ModManager] Script runtime not initialized");
                    }
                }
                Err(e) => {
                    eprintln!(
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
            println!(
                "[ModManager] Dispatched 'init' lifecycle event for mod '{}'",
                mod_id
            );
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
