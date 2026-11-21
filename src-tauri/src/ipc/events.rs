use super::wire::JsonPayload;
use crate::bot::QqAuthorizationPrompt;
use crate::characters::activation::CommittedCharacterRuntime;
use crate::live2d::Live2dModelProfile;
use crate::mods::theme::ModThemeJson;
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use tauri_specta::Event;

macro_rules! event {
    ($name:ident, $channel:literal, $payload:ty) => {
        #[derive(Clone, Serialize, Type, Event)]
        #[serde(transparent)]
        #[tauri_specta(event_name = $channel)]
        pub(crate) struct $name(pub(crate) $payload);
    };
}

macro_rules! signal {
    ($name:ident, $channel:literal) => {
        #[derive(Clone, Serialize, Type, Event)]
        #[tauri_specta(event_name = $channel)]
        pub(crate) struct $name;
    };
}

event!(BubbleTextUpdate, "bubble-text-update", String);
event!(CameraObservation, "camera-observation", String);
event!(
    CharacterRuntimeCommitted,
    "character-runtime-committed",
    CommittedCharacterRuntime
);
event!(ChatError, "chat-error", LegacyChatError);
event!(ImagegenError, "imagegen:error", String);
event!(
    Live2dProfileUpdated,
    "live2d-profile-updated",
    Live2dModelProfile
);
event!(ModComponentsRegister, "mod:components-register", HashMap<String, String>);
event!(ModLayoutOverride, "mod:layout-override", JsonPayload);
event!(ModThemeOverride, "mod:theme-override", ModThemeJson);
event!(
    QqAuthorizationExpired,
    "qq-authorization-expired",
    QqAuthorizationPrompt
);
event!(
    QqAuthorizationRequest,
    "qq-authorization-request",
    QqAuthorizationPrompt
);
event!(SttWakeWordDetected, "stt:wake-word-detected", String);
event!(SttWakeWordError, "stt:wake-word-error", String);
signal!(PetWindowClosed, "pet-window-closed");
signal!(ShowMainWindow, "show-main-window");
signal!(SttMicAutoStop, "stt:mic-auto-stop");
signal!(ToggleChatInput, "toggle-chat-input");
signal!(ModUnload, "mod:unload");

#[allow(dead_code)]
#[derive(Clone, Serialize, Type)]
#[serde(untagged)]
pub(crate) enum LegacyChatError {
    Message(String),
    Structured(crate::error::ChatErrorEvent),
}

#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "voice-interrupt-stt")]
pub(crate) struct VoiceInterruptStt {}

#[derive(Clone, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VisionStatus {
    Active,
    Inactive,
}
event!(VisionStatusChanged, "vision-status", VisionStatus);

#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "vision-observation")]
pub(crate) struct VisionObservationEvent {
    pub summary: String,
    pub captured_at: String,
    pub source: crate::vision::context::VisionObservationSource,
}

#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "proactive-trigger")]
pub(crate) struct ProactiveTrigger {
    pub trigger: String,
    pub character_id: String,
    pub conversation_id: Option<String>,
    pub client_request_id: String,
    pub instruction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_seconds: Option<u64>,
}

#[derive(Clone, Serialize, Type, Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "live2d-model-selection-updated")]
pub(crate) struct Live2dModelSelectionUpdated {
    model_path: String,
    custom_model_path: Option<String>,
    model_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_source: Option<Live2dSelectionSource>,
}

#[allow(dead_code)]
#[derive(Clone, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Live2dSelectionSource {
    User,
    Package,
    Builtin,
    None,
}

#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "tts:playback-error")]
pub(crate) struct TtsPlaybackErrorEvent {
    request_id: String,
    error: String,
}

#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "telegram:chat-sync")]
pub(crate) struct TelegramChatSync {
    role: String,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    translation: Option<String>,
}
