use crate::actions::registry::ActionServices;
use crate::ai::context::AIOrchestrator;
use crate::bot::{BotChatServices, BotEvent};
use crate::chat::application::{ChatDependencies, ChatService};
use crate::chat::event_payloads::{ChatEventScope, ImageGenDone};
use crate::chat::events::{ChatDesktop, ChatEvents};
use crate::chat::runtime::ChatRuntimeState;
use crate::imagegen::{ImageGenService, WindowSizeState};
use crate::ipc::{events::{QqAuthorizationExpired, QqAuthorizationRequest, ShowMainWindow}, wire::{emit, validate_json}};
use crate::live2d::Live2dStore;
use crate::tts::{desktop::TauriTtsPlayback, TtsPlayback};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Weak};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// The app event loop owns the strong handle. Managed services hold this
/// weak adapter so keeping a service alive cannot keep Tauri's state alive.
#[derive(Clone)]
pub(crate) struct TauriChatDesktop(Weak<AppHandle>);

impl TauriChatDesktop {
    pub(crate) fn new(app: &Arc<AppHandle>) -> Self {
        Self(Arc::downgrade(app))
    }

    fn notify(&self, title: &str, message: &str) -> Result<(), String> {
        let app = self.0.upgrade().ok_or("The desktop is closed")?;
        app.notification()
            .builder()
            .title(title)
            .body(message)
            .show()
            .map_err(|error| error.to_string())
    }

    fn emit_bot(&self, event: BotEvent) -> Result<(), String> {
        let app = self.0.upgrade().ok_or("The desktop is closed")?;
        let result = match event {
            BotEvent::QqAuthorizationRequested(prompt) => {
                let result = emit(&*app, QqAuthorizationRequest(prompt));
                let _ = emit(&*app, ShowMainWindow);
                result
            }
            BotEvent::QqAuthorizationExpired(prompt) => {
                emit(&*app, QqAuthorizationExpired(prompt))
            }
            BotEvent::ImageGenerated {
                result,
                character_id,
                conversation_id,
                client_request_id,
            } => {
                emit(&*app, ImageGenDone {
                    result,
                    scope: ChatEventScope {
                        character_id: Some(character_id),
                        conversation_id: Some(Some(conversation_id)),
                        client_request_id: Some(Some(client_request_id)),
                        ..Default::default()
                    },
                    turn_id: None,
                })
            }
        };
        result.map_err(|error| error.to_string())
    }
}

impl ChatDesktop for TauriChatDesktop {
    fn emit(&self, name: &'static str, payload: Value) -> Result<(), String> {
        let Some(app) = self.0.upgrade() else {
            return Ok(());
        };
        validate_json(&payload)?;
        app.emit(name, payload)
            .map_err(|error| error.to_string())
    }

    fn window_is_open(&self, label: &str) -> bool {
        self.0
            .upgrade()
            .is_some_and(|app| app.get_webview_window(label).is_some())
    }

    fn playback(&self, runtime: Arc<ChatRuntimeState>) -> Option<Box<dyn TtsPlayback>> {
        let app = self.0.upgrade()?;
        Some(Box::new(TauriTtsPlayback::new(&app, Some(runtime))))
    }
}

pub(crate) fn install_services(
    app: &AppHandle,
    app_data: &Path,
    desktop: TauriChatDesktop,
) -> BotChatServices {
    let state = app
        .try_state::<AIOrchestrator>()
        .map(|state| Arc::new(state.inner().clone()));
    let imagegen = app.state::<ImageGenService>().inner().clone();
    let window_size = app.state::<WindowSizeState>().inner().clone();
    let vision = app
        .state::<crate::vision::watcher::VisionWatcher>()
        .inner()
        .clone();
    let live2d = Arc::new(Live2dStore::new(app_data.to_owned()));
    let events = ChatEvents {
        runtime: app.state::<Arc<ChatRuntimeState>>().inner().clone(),
        desktop: Some(Arc::new(desktop.clone())),
        mods: app
            .try_state::<crate::mods::script_queue::ModScriptQueue>()
            .map(|state| state.inner().clone()),
    };
    app.manage(events.clone());
    let notification_desktop = desktop.clone();
    let notification = move |title: &str, body: &str| notification_desktop.notify(title, body);
    let actions = Arc::new(ActionServices {
        state: state.clone(),
        vision: vision.clone(),
        capture_screen: crate::vision::capture::capture_screen_with_options,
        imagegen: Some(imagegen.clone()),
        window_size: Some(window_size.clone()),
        live2d: live2d.clone(),
        events: events.clone(),
        notification: Some(Arc::new(notification)),
    });
    app.manage(actions.clone());

    let tts = app
        .try_state::<crate::tts::TtsService>()
        .map(|state| state.inner().clone());
    let stt = app
        .try_state::<crate::stt::SttService>()
        .map(|state| state.inner().clone());
    let chat = state.map(|state| {
        let chat = ChatService::new(
            ChatDependencies {
                state: state.clone(),
                imagegen: imagegen.clone(),
                llm: app
                    .state::<crate::llm::service::LlmService>()
                    .inner()
                    .clone(),
                registry: app
                    .state::<Arc<tokio::sync::RwLock<crate::actions::ActionRegistry>>>()
                    .inner()
                    .clone(),
                tool_settings: app
                    .state::<Arc<tokio::sync::RwLock<crate::actions::tool_settings::ToolSettings>>>(
                    )
                    .inner()
                    .clone(),
                approval: app
                    .state::<Arc<crate::chat::application::PendingToolApprovalState>>()
                    .inner()
                    .clone(),
                cancellation: app
                    .state::<Arc<crate::chat::application::TurnCancellationState>>()
                    .inner()
                    .clone(),
                vision,
                window_size,
                vision_server: app
                    .state::<Arc<tokio::sync::Mutex<crate::vision::server::VisionServer>>>()
                    .inner()
                    .clone(),
                hooks: app
                    .try_state::<crate::hooks::HookRuntime>()
                    .map(|state| state.inner().clone()),
                activation: app
                    .state::<crate::characters::activation::ActivationCoordinator>()
                    .inner()
                    .clone(),
                package_root: app
                    .state::<crate::characters::catalog::CharacterCatalog>()
                    .root()
                    .to_owned(),
                live2d,
                tts: tts.clone(),
                stt: stt.clone(),
                actions,
            },
            events,
        );
        app.manage(chat.clone());
        (chat, state)
    });

    let services = BotChatServices::new(
        chat,
        stt,
        tts,
        Some(imagegen),
        Arc::new(move |event| desktop.emit_bot(event)),
    );
    app.manage(services.clone());
    services
}
