use rquickjs::{Ctx, Function, Object, Result};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::Sender;

/// Events emitted by QuickJS scripts, forwarded to the Tauri event bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScriptEvent {
    /// Kokoro.emit(event, payload) → mod:script-event Tauri event
    Emit {
        event: String,
        payload: serde_json::Value,
    },
    /// Kokoro.ui.send(component, data) → mod:ui-message Tauri event
    UiSend {
        component: String,
        data: serde_json::Value,
    },
    /// Kokoro.character.setExpression(expr) → set-expression Tauri event
    SetExpression { expression: String },
}

/// Register the Kokoro API into the QuickJS context.
/// `event_tx` is a std::sync channel sender used by closures to forward events
/// out of the QuickJS thread to the Tauri event bus.
pub fn register_api(ctx: &Ctx<'_>, event_tx: Sender<ScriptEvent>) -> Result<()> {
    let globals = ctx.globals();
    let kokoro = Object::new(ctx.clone())?;

    // ── Kokoro.log(msg) ──
    kokoro.set(
        "log",
        Function::new(ctx.clone(), |msg: String| {
            println!("[Kokoro JS] {}", msg);
        })?,
    )?;

    // ── Kokoro.version ──
    kokoro.set("version", "0.3.0")?;

    // Mount Kokoro to globalThis BEFORE eval blocks that reference it.
    // JS objects are by-reference, so later kokoro.set() calls still apply.
    globals.set("Kokoro", kokoro.clone())?;

    // ── Kokoro.on(eventName, callback) ──
    // Stores callbacks in a JS-side registry object (__listeners).
    // The DispatchEvent ScriptCommand will invoke __dispatch(event, payload).
    // We build the listener registry in JS for simplicity.
    ctx.eval::<(), _>(
        r#"
        globalThis.__listeners = {};
        globalThis.__dispatch = function(event, payload) {
            var cbs = globalThis.__listeners[event];
            if (!cbs) return;
            for (var i = 0; i < cbs.length; i++) {
                try { cbs[i](payload); } catch(e) { Kokoro.log("Listener error: " + e); }
            }
        };
    "#,
    )?;

    // ── Kokoro.on(eventName, callback) ──
    // Implemented in JS to avoid Rust closure lifetime issues with rquickjs.
    // Pushes callbacks into __listeners[eventName], which __dispatch() iterates.
    ctx.eval::<(), _>(
        r#"
        Kokoro.on = function(eventName, callback) {
            if (!globalThis.__listeners[eventName]) {
                globalThis.__listeners[eventName] = [];
            }
            globalThis.__listeners[eventName].push(callback);
        };
    "#,
    )?;

    // ── Kokoro.emit(eventName, payload) ──
    let emit_tx = event_tx.clone();
    kokoro.set(
        "emit",
        Function::new(
            ctx.clone(),
            move |event: String, payload: rquickjs::Value<'_>| {
                // Convert JS value to serde_json::Value
                let json_payload = js_value_to_json(&payload);
                let _ = emit_tx.send(ScriptEvent::Emit {
                    event,
                    payload: json_payload,
                });
            },
        )?,
    )?;

    // ── Kokoro.ui ── (namespace for UI communication)
    let ui = Object::new(ctx.clone())?;

    let ui_tx = event_tx.clone();
    ui.set(
        "send",
        Function::new(
            ctx.clone(),
            move |component: String, data: rquickjs::Value<'_>| {
                let json_data = js_value_to_json(&data);
                let _ = ui_tx.send(ScriptEvent::UiSend {
                    component,
                    data: json_data,
                });
            },
        )?,
    )?;

    kokoro.set("ui", ui)?;

    // ── Kokoro.character ── (namespace for character control)
    let character = Object::new(ctx.clone())?;

    let char_tx = event_tx.clone();
    character.set(
        "setExpression",
        Function::new(ctx.clone(), move |expression: String| {
            let _ = char_tx.send(ScriptEvent::SetExpression { expression });
        })?,
    )?;

    kokoro.set("character", character)?;
    Ok(())
}

/// Convert a rquickjs::Value to serde_json::Value for transit.
/// Handles primitives, strings, arrays, and objects.
fn js_value_to_json(val: &rquickjs::Value<'_>) -> serde_json::Value {
    if val.is_null() || val.is_undefined() {
        return serde_json::Value::Null;
    }

    if let Some(b) = val.as_bool() {
        return serde_json::Value::Bool(b);
    }

    if let Some(n) = val.as_int() {
        return serde_json::json!(n);
    }

    if let Some(n) = val.as_float() {
        return serde_json::json!(n);
    }

    if let Some(s) = val.clone().into_string() {
        if let Ok(s) = s.to_string() {
            return serde_json::Value::String(s);
        }
    }

    // Try JSON.stringify for complex objects
    if let Ok(ctx) = val.ctx().globals().get::<_, rquickjs::Object>("JSON") {
        if let Ok(stringify) = ctx.get::<_, rquickjs::Function>("stringify") {
            if let Ok(result) = stringify.call::<_, String>((val.clone(),)) {
                if let Ok(parsed) = serde_json::from_str(&result) {
                    return parsed;
                }
            }
        }
    }

    serde_json::Value::Null
}
