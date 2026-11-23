use serde::{Serialize, Serializer};
use serde_json::Value;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// The recursive shape of a JSON value on the WebView transport. The Rust
/// application continues to use serde_json::Value for the actual payload.
#[allow(dead_code)]
#[derive(serde::Deserialize, serde::Serialize, specta::Type)]
#[serde(untagged)]
pub(crate) enum JsonValue {
    Boolean(bool),
    Number(Option<f64>),
    String(String),
    Array(Vec<JsonValue>),
    Object(std::collections::HashMap<String, JsonValue>),
}

#[derive(Clone, Debug, serde::Deserialize, Serialize, specta::Type)]
#[serde(transparent)]
pub struct JsonPayload(#[specta(type = JsonValue)] pub(crate) Value);

pub(crate) fn validate_json(value: &Value) -> Result<(), &'static str> {
    match value {
        Value::Number(number)
            if number.as_u64().is_some_and(|n| n > MAX_SAFE_INTEGER)
                || number
                    .as_i64()
                    .is_some_and(|n| n.unsigned_abs() > MAX_SAFE_INTEGER)
                || (number.is_f64()
                    && number.as_f64().is_some_and(|n| {
                        n.fract() == 0.0 && n.abs() > MAX_SAFE_INTEGER as f64
                    })) =>
        {
            Err("IPC integer exceeds the JavaScript safe integer range")
        }
        Value::Array(values) => values.iter().try_for_each(validate_json),
        Value::Object(fields) => fields.values().try_for_each(validate_json),
        _ => Ok(()),
    }
}

/// Export the inner type and reject responses that could be rounded by JSON
/// parsing in the WebView. This wrapper applies only to the IPC transport.
#[derive(Clone, Debug, PartialEq, specta::Type)]
#[specta(transparent)]
pub struct Response<T>(pub(crate) T);

impl<T: Serialize> Serialize for Response<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = serde_json::to_value(&self.0).map_err(serde::ser::Error::custom)?;
        validate_json(&value).map_err(serde::ser::Error::custom)?;
        value.serialize(serializer)
    }
}

impl<T> std::ops::Deref for Response<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

pub(crate) fn emit<R: tauri::Runtime, T: Clone + Serialize + tauri_specta::Event>(
    target: &impl tauri::Emitter<R>,
    payload: T,
) -> tauri::Result<()> {
    target.emit(T::NAME, Response(payload))
}

pub(crate) fn emit_to<R: tauri::Runtime, T: Clone + Serialize + tauri_specta::Event>(
    target: &impl tauri::Emitter<R>,
    destination: impl Into<tauri::EventTarget>,
    payload: T,
) -> tauri::Result<()> {
    target.emit_to(destination, T::NAME, Response(payload))
}
