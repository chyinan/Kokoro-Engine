use super::events::{BubbleTextUpdate, ModLayoutOverride, ShowMainWindow};
use super::wire::{validate_json, JsonPayload, Response};
use crate::commands::characters::{CreateCharacterRequest, UpdateCharacterRequest};
use crate::error::KokoroError;
use crate::mcp::manager::McpServerConfig;
use serde_json::{json, Value};
use tauri_specta::Event;

#[test]
fn integer_precision_is_validated_in_both_directions() {
    let max_safe = 9_007_199_254_740_991_i64;
    let exact = json!({"ids": [max_safe, -max_safe], "page": 4.5, "bytes": [0, 255]});
    assert_eq!(validate_json(&exact), Ok(()));
    assert_eq!(
        serde_json::to_value(Response(exact.clone())).unwrap(),
        exact
    );

    for invalid in [
        json!(max_safe + 1),
        json!(-max_safe - 1),
        json!(u64::MAX),
        json!(1e21),
        serde_json::from_str("18446744073709551616").unwrap(),
    ] {
        let nested = json!({"items": [{"value": invalid}]});
        assert!(validate_json(&nested).unwrap_err().contains("safe integer"));
        assert!(serde_json::to_value(Response(nested)).is_err());
    }
    assert!(serde_json::to_value(Response(usize::MAX)).is_err() || usize::BITS == 32);
    assert_eq!(
        serde_json::from_value::<Vec<u8>>(json!([0, 255])).unwrap(),
        [0, 255]
    );
    assert!(serde_json::from_value::<Vec<u8>>(json!([0, 256])).is_err());
}

#[cfg(target_os = "windows")]
#[test]
fn native_dispatch_routes_results_and_rejections() {
    use crate::commands::pet::PetConfig;
    use std::sync::mpsc;
    use std::time::Duration;
    use tauri::ipc::{CallbackFn, InvokeError, InvokeResponse};
    use tauri::webview::InvokeRequest;

    let profile = tempfile::tempdir().unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let origin = context.config().build.dev_url.clone().unwrap();
    let builder = super::builder();
    let app = tauri::Builder::<tauri::Wry>::default()
        .any_thread()
        .invoke_handler(super::invoke_handler(&builder))
        .build(context)
        .unwrap();
    builder.mount_events(&app);
    let window = tauri::WebviewWindowBuilder::new(
        &app,
        "main",
        tauri::WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .data_directory(profile.path().to_owned())
    .visible(false)
    .build()
    .unwrap();

    let invoke = |command: &str, body: Value| {
        let (tx, rx) = mpsc::sync_channel(1);
        window.clone().on_message(
            InvokeRequest {
                cmd: command.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: origin.clone(),
                body: body.into(),
                headers: Default::default(),
                invoke_key: app.invoke_key().to_owned(),
            },
            Box::new(move |_, _, response, _, _| tx.send(response).unwrap()),
        );
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            InvokeResponse::Ok(response) => Ok(response.deserialize::<Value>().unwrap()),
            InvokeResponse::Err(InvokeError(error)) => Err(error),
        }
    };

    assert_eq!(
        invoke("get_engine_info", json!({})).unwrap()["name"],
        "Kokoro Engine"
    );
    let conf = PetConfig {
        shortcut: String::new(),
        ..Default::default()
    };
    assert_eq!(
        invoke("save_pet_config", json!({"config": conf})).unwrap_err(),
        json!({"code": "Validation", "message": "Pet shortcut cannot be empty"}),
    );
    let malformed = invoke("save_pet_config", json!({"config": 10})).unwrap_err();
    assert!(malformed
        .as_str()
        .is_some_and(|message| message.contains("invalid args")));
    let missing = invoke("missing_ipc_test_command", json!({})).unwrap_err();
    assert!(missing
        .as_str()
        .is_some_and(|message| message.contains("not found")));
    let unsafe_number = invoke(
        "set_window_size",
        json!({"width": 9_007_199_254_740_992u64, "height": 100}),
    )
    .unwrap_err();
    assert_eq!(
        unsafe_number,
        json!({"code": "Validation", "message": "IPC integer exceeds the JavaScript safe integer range"}),
    );
}

#[test]
fn serde_preserves_default_fields_renames_and_null_avatar_updates() {
    let request = json!({
        "id": "avatar-example", "name": "Example", "persona": "", "user_nickname": "",
        "source_format": "custom", "created_at": 123, "updated_at": 456,
    });
    let created: CreateCharacterRequest = serde_json::from_value(request.clone()).unwrap();
    assert_eq!(created.avatar_path, None);
    assert_eq!(created.description, "");
    assert_eq!(created.runtime_profile_json, "{}");

    let omitted: UpdateCharacterRequest = serde_json::from_value(request.clone()).unwrap();
    assert_eq!(omitted.avatar_path, None);
    let mut request = request;
    request["avatar_path"] = Value::Null;
    let clear: UpdateCharacterRequest = serde_json::from_value(request.clone()).unwrap();
    assert_eq!(clear.avatar_path, Some(None));
    request["avatar_path"] = json!("avatar.webp");
    let replace: UpdateCharacterRequest = serde_json::from_value(request).unwrap();
    assert_eq!(replace.avatar_path, Some(Some("avatar.webp".into())));

    let server: McpServerConfig = serde_json::from_value(json!({"name": "Example"})).unwrap();
    let wire = serde_json::to_value(server).unwrap();
    assert_eq!(wire["type"], "stdio");
    assert_eq!(wire["enabled"], true);
    assert_eq!(wire["url"], Value::Null);
    assert!(wire.get("transport_type").is_none());
}

#[test]
fn event_and_error_wire_shapes_remain_compatible() {
    assert_eq!(BubbleTextUpdate::NAME, "bubble-text-update");
    assert_eq!(ShowMainWindow::NAME, "show-main-window");
    assert_eq!(
        serde_json::to_value(BubbleTextUpdate("hello".into())).unwrap(),
        json!("hello")
    );
    assert_eq!(serde_json::to_value(ShowMainWindow).unwrap(), Value::Null);

    let layout = json!({"panels": [null, true, 1.5, "chat", {"x": 42}]});
    let dynamic: JsonPayload = serde_json::from_value(layout.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(Response(ModLayoutOverride(dynamic))).unwrap(),
        layout
    );
    assert_eq!(
        serde_json::to_value(KokoroError::Validation("invalid".into())).unwrap(),
        json!({"code": "Validation", "message": "invalid"}),
    );
}

#[test]
fn exporter_keeps_rejected_errors_and_native_event_names() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("ipc.ts");
    super::export(&output).unwrap();
    let binding = std::fs::read_to_string(&output).unwrap();
    assert!(binding.lines().any(|line| {
        line.contains("getEngineInfo:")
            && line.contains("__TAURI_INVOKE<EngineInfo>(\"get_engine_info\"")
    }));
    assert!(
        binding.contains("__TAURI_INVOKE<Response<StreamChatResponse_Serialize>>(\"stream_chat\"")
    );
    assert!(binding.lines().any(|line| {
        line.contains("ttsAudio: makeEvent<TtsAudioEvent") && line.contains("\"tts:audio\"")
    }));
    assert!(binding.lines().any(|line| {
        line.contains("showMainWindow: makeEvent<ShowMainWindow")
            && line.contains("\"show-main-window\"")
    }));
    assert!(
        binding.contains("JsonValue_Deserialize[] | { [key in string]: JsonValue_Deserialize }")
    );
    assert!(!binding.contains("typedError"));
}
