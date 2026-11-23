use super::config::ProviderConfig;
use super::interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError, TtsParams, TtsProvider, VoiceProfile,
};
use crate::lifecycle::TaskScope;
use async_trait::async_trait;
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::{fs, io::AsyncReadExt, process::Command};
use tokio_util::sync::CancellationToken;

const DEFAULT_MODEL: &str = "k2-fsa/OmniVoice";
const DEFAULT_PYTHON: &str = "python";
const OMNIVOICE_PYTHON_SCRIPT: &str = r#"
import json
import sys

import soundfile as sf
import torch

from omnivoice.models.omnivoice import OmniVoice, OmniVoiceGenerationConfig


def best_device():
    if torch.cuda.is_available():
        return "cuda"
    mps = getattr(torch.backends, "mps", None)
    if mps is not None and mps.is_available():
        return "mps"
    return "cpu"


with open(sys.argv[1], "r", encoding="utf-8") as f:
    req = json.load(f)

device = req.get("device") or best_device()
model = OmniVoice.from_pretrained(
    req["model"],
    device_map=device,
    dtype=torch.float16,
)

config_keys = [
    "num_step",
    "guidance_scale",
    "t_shift",
    "denoise",
    "preprocess_prompt",
    "postprocess_output",
    "layer_penalty_factor",
    "position_temperature",
    "class_temperature",
]
config_kwargs = {key: req[key] for key in config_keys if req.get(key) is not None}
generation_config = OmniVoiceGenerationConfig(**config_kwargs)

kwargs = {
    "text": req["text"],
    "generation_config": generation_config,
    "ref_audio": req["ref_audio"],
}

if req.get("ref_text"):
    kwargs["ref_text"] = req["ref_text"]
if req.get("language"):
    kwargs["language"] = req["language"]
if req.get("instruct"):
    kwargs["instruct"] = req["instruct"]
if req.get("duration") is not None and float(req["duration"]) > 0:
    kwargs["duration"] = float(req["duration"])
elif req.get("speed") is not None:
    kwargs["speed"] = float(req["speed"])

audio = model.generate(**kwargs)
sf.write(req["output"], audio[0], model.sampling_rate)
"#;

/// OmniVoice provider — invokes the local OmniVoice Python API in a child process.
///
/// Expected config:
/// - `model`: OmniVoice checkpoint path or HuggingFace repo id, defaults to `k2-fsa/OmniVoice`
/// - `extra.ref_audio_path` or `extra.ref_audio`: reference audio path
/// - `extra.ref_text` or `extra.prompt_text`: optional reference transcript
/// - `extra.project_path`: optional local OmniVoice checkout path for `PYTHONPATH`
/// - `extra.python_executable`: optional Python executable, defaults to `python`
#[derive(Clone)]
pub struct OmniVoiceProvider {
    scope: Option<TaskScope>,
    provider_id: String,
    python_executable: String,
    project_path: Option<PathBuf>,
    model: String,
    default_ref_audio: Option<String>,
    default_ref_text: Option<String>,
    default_language: Option<String>,
    default_device: Option<String>,
    default_speed: Option<f32>,
    num_step: Option<u32>,
    guidance_scale: Option<f32>,
    duration: Option<f32>,
    t_shift: Option<f32>,
    denoise: Option<bool>,
    preprocess_prompt: Option<bool>,
    postprocess_output: Option<bool>,
    layer_penalty_factor: Option<f32>,
    position_temperature: Option<f32>,
    class_temperature: Option<f32>,
}

impl OmniVoiceProvider {
    pub fn from_config(config: &ProviderConfig) -> Option<Self> {
        let extra = &config.extra;
        let python_executable = extra_string(extra, &["python_executable", "python_path"])
            .unwrap_or_else(|| DEFAULT_PYTHON.to_string());

        Some(Self {
            scope: None,
            provider_id: config.id.clone(),
            python_executable,
            project_path: extra_string(extra, &["project_path", "install_path"]).map(PathBuf::from),
            model: config
                .model
                .clone()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            default_ref_audio: extra_string(extra, &["ref_audio_path", "ref_audio"]),
            default_ref_text: extra_string(extra, &["ref_text", "prompt_text"]),
            default_language: extra_string(extra, &["language"]),
            default_device: extra_string(extra, &["device"]),
            default_speed: extra_f32(extra, &["speed"]),
            num_step: extra_u32(extra, &["num_step"]),
            guidance_scale: extra_f32(extra, &["guidance_scale"]),
            duration: extra_f32(extra, &["duration"]),
            t_shift: extra_f32(extra, &["t_shift"]),
            denoise: extra_bool(extra, &["denoise"]),
            preprocess_prompt: extra_bool(extra, &["preprocess_prompt"]),
            postprocess_output: extra_bool(extra, &["postprocess_output"]),
            layer_penalty_factor: extra_f32(extra, &["layer_penalty_factor"]),
            position_temperature: extra_f32(extra, &["position_temperature"]),
            class_temperature: extra_f32(extra, &["class_temperature"]),
        })
    }

    fn python_executable_for_params(&self, params: &TtsParams) -> String {
        params
            .extra_params
            .as_ref()
            .and_then(|extra| extra_string(extra, &["python_executable", "python_path"]))
            .unwrap_or_else(|| self.python_executable.clone())
    }

    fn project_path_for_params(&self, params: &TtsParams) -> Option<PathBuf> {
        params
            .extra_params
            .as_ref()
            .and_then(|extra| extra_string(extra, &["project_path", "install_path"]))
            .map(PathBuf::from)
            .or_else(|| self.project_path.clone())
    }

    fn build_request(
        &self,
        text: &str,
        params: &TtsParams,
        output_path: &Path,
    ) -> Result<OmniVoiceGenerateRequest, TtsError> {
        let request_extra = params.extra_params.as_ref();

        let model = request_extra
            .and_then(|extra| extra_string(extra, &["model"]))
            .unwrap_or_else(|| self.model.clone());

        let ref_audio = request_extra
            .and_then(|extra| extra_string(extra, &["ref_audio_path", "ref_audio"]))
            .or_else(|| self.default_ref_audio.clone())
            .ok_or_else(|| {
                TtsError::ConfigError(
                    "OmniVoice requires extra.ref_audio_path or extra.ref_audio".to_string(),
                )
            })?;

        let ref_text = request_extra
            .and_then(|extra| extra_string(extra, &["ref_text", "prompt_text"]))
            .or_else(|| self.default_ref_text.clone());

        let language = request_extra
            .and_then(|extra| extra_string(extra, &["language"]))
            .or_else(|| self.default_language.clone());
        let device = request_extra
            .and_then(|extra| extra_string(extra, &["device"]))
            .or_else(|| self.default_device.clone());
        let speed = request_extra
            .and_then(|extra| extra_f32(extra, &["speed"]))
            .or(self.default_speed)
            .or(params.speed);

        let num_step = request_extra
            .and_then(|extra| extra_u32(extra, &["num_step"]))
            .or(self.num_step);
        let guidance_scale = request_extra
            .and_then(|extra| extra_f32(extra, &["guidance_scale"]))
            .or(self.guidance_scale);
        let duration = request_extra
            .and_then(|extra| extra_f32(extra, &["duration"]))
            .or(self.duration);
        let t_shift = request_extra
            .and_then(|extra| extra_f32(extra, &["t_shift"]))
            .or(self.t_shift);
        let denoise = request_extra
            .and_then(|extra| extra_bool(extra, &["denoise"]))
            .or(self.denoise);
        let preprocess_prompt = request_extra
            .and_then(|extra| extra_bool(extra, &["preprocess_prompt"]))
            .or(self.preprocess_prompt);
        let postprocess_output = request_extra
            .and_then(|extra| extra_bool(extra, &["postprocess_output"]))
            .or(self.postprocess_output);
        let layer_penalty_factor = request_extra
            .and_then(|extra| extra_f32(extra, &["layer_penalty_factor"]))
            .or(self.layer_penalty_factor);
        let position_temperature = request_extra
            .and_then(|extra| extra_f32(extra, &["position_temperature"]))
            .or(self.position_temperature);
        let class_temperature = request_extra
            .and_then(|extra| extra_f32(extra, &["class_temperature"]))
            .or(self.class_temperature);

        Ok(OmniVoiceGenerateRequest {
            model,
            text: text.to_string(),
            output: output_path.to_string_lossy().to_string(),
            ref_audio,
            ref_text: ref_text.filter(|s| !s.trim().is_empty()),
            language,
            device,
            speed,
            duration,
            instruct: request_extra.and_then(|extra| extra_string(extra, &["instruct"])),
            num_step,
            guidance_scale,
            t_shift,
            denoise,
            preprocess_prompt,
            postprocess_output,
            layer_penalty_factor,
            position_temperature,
            class_temperature,
        })
    }
}

#[derive(Debug, Serialize)]
struct OmniVoiceGenerateRequest {
    model: String,
    text: String,
    output: String,
    ref_audio: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ref_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instruct: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_step: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    guidance_scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    t_shift: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    denoise: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preprocess_prompt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postprocess_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    layer_penalty_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position_temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    class_temperature: Option<f32>,
}

#[async_trait]
impl TtsProvider for OmniVoiceProvider {
    fn id(&self) -> String {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_streaming: false,
            supports_emotions: false,
            supports_speed: true,
            supports_pitch: false,
            supports_cloning: true,
            supports_ssml: false,
        }
    }

    fn voices(&self) -> Vec<VoiceProfile> {
        vec![VoiceProfile {
            voice_id: format!("{}_clone", self.provider_id),
            name: "OmniVoice Clone".to_string(),
            gender: Gender::Neutral,
            language: "auto".to_string(),
            engine: TtsEngine::Vits,
            provider_id: self.provider_id.clone(),
            extra_params: Default::default(),
        }]
    }

    fn cache_key_salt(&self) -> Option<String> {
        Some(
            serde_json::json!({
                "model": &self.model,
                "ref_audio": self.default_ref_audio.as_deref(),
                "ref_text": self.default_ref_text.as_deref(),
                "project_path": self.project_path.as_ref().map(|p| p.display().to_string()),
                "python_executable": &self.python_executable,
                "language": self.default_language.as_deref(),
                "device": self.default_device.as_deref(),
                "speed": self.default_speed,
                "num_step": self.num_step,
                "guidance_scale": self.guidance_scale,
                "duration": self.duration,
                "t_shift": self.t_shift,
                "denoise": self.denoise,
                "preprocess_prompt": self.preprocess_prompt,
                "postprocess_output": self.postprocess_output,
                "layer_penalty_factor": self.layer_penalty_factor,
                "position_temperature": self.position_temperature,
                "class_temperature": self.class_temperature,
            })
            .to_string(),
        )
    }

    async fn is_available(&self) -> bool {
        !self.python_executable.trim().is_empty()
            && self
                .default_ref_audio
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
    }

    async fn synthesize(&self, text: &str, params: TtsParams) -> Result<Vec<u8>, TtsError> {
        let provider = self.clone();
        let text = text.to_owned();
        let stop = self
            .scope
            .as_ref()
            .map_or_else(CancellationToken::new, |scope| scope.token().child_token());
        let _cancel_if_abandoned = stop.clone().drop_guard();
        let run = async move { provider.run_synthesis(&text, params, stop).await };
        if let Some(scope) = &self.scope {
            let mut task = scope
                .spawn_durable("OmniVoice Python inference", run)
                .map_err(|_| TtsError::Cancelled)?;
            task.join()
                .await
                .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?
        } else {
            run.await
        }
    }
}

impl OmniVoiceProvider {
    pub(crate) fn with_scope(mut self, scope: Option<TaskScope>) -> Self {
        self.scope = scope;
        self
    }

    async fn run_synthesis(
        &self,
        text: &str,
        params: TtsParams,
        stop: CancellationToken,
    ) -> Result<Vec<u8>, TtsError> {
        let workspace = tempfile::tempdir()
            .map_err(|error| TtsError::SynthesisFailed(format!("OmniVoice staging: {error}")))?;
        let output_path = workspace.path().join("audio.wav");
        let request_path = workspace.path().join("request.json");
        let request = self.build_request(text, &params, &output_path)?;
        let request_json = serde_json::to_vec(&request).map_err(|error| {
            TtsError::SynthesisFailed(format!("OmniVoice request serialization failed: {error}"))
        })?;
        fs::write(&request_path, request_json)
            .await
            .map_err(|error| {
                TtsError::SynthesisFailed(format!("OmniVoice request file: {error}"))
            })?;

        let python = self.python_executable_for_params(&params);
        let mut command = Command::new(&python);
        command
            .args(["-c", OMNIVOICE_PYTHON_SCRIPT])
            .arg(&request_path);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(project_path) = self.project_path_for_params(&params) {
            command.current_dir(&project_path);
            command.env("PYTHONPATH", build_pythonpath(&project_path));
        }
        let output = run_python(command, &stop, self.scope.as_ref()).await?;
        if !output.status.success() {
            return Err(TtsError::SynthesisFailed(format!(
                "OmniVoice exited with {}. stdout: {} stderr: {}",
                output.status,
                process_output_text(&output.stdout),
                process_output_text(&output.stderr)
            )));
        }
        let bytes = fs::read(&output_path).await.map_err(|error| {
            TtsError::SynthesisFailed(format!("OmniVoice did not produce readable audio: {error}"))
        })?;
        if bytes.is_empty() {
            return Err(TtsError::SynthesisFailed(
                "OmniVoice produced an empty audio file".into(),
            ));
        }
        Ok(bytes)
    }
}

async fn run_python(
    command: Command,
    stop: &CancellationToken,
    scope: Option<&TaskScope>,
) -> Result<std::process::Output, TtsError> {
    if stop.is_cancelled() {
        return Err(TtsError::Cancelled);
    }
    let mut command = CommandWrap::from(command);
    command.wrap(KillOnDrop);
    #[cfg(windows)]
    {
        let mut flags = process_wrap::tokio::CreationFlags(Default::default());
        flags.0 .0 = 0x08000000; // CREATE_NO_WINDOW
        command.wrap(flags).wrap(process_wrap::tokio::JobObject);
    }
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    let mut child = command.spawn().map_err(|error| {
        TtsError::SynthesisFailed(format!("OmniVoice process launch failed: {error}"))
    })?;
    let mut stdout = child.stdout().take().expect("piped OmniVoice stdout");
    let mut stderr = child.stderr().take().expect("piped OmniVoice stderr");
    let mut out = Vec::new();
    let mut err = Vec::new();
    let outcome = tokio::select! {
        biased;
        output = async { tokio::try_join!(child.wait(), stdout.read_to_end(&mut out), stderr.read_to_end(&mut err)) } => Some(output),
        _ = stop.cancelled() => None,
    };
    if let Some(Ok((status, _, _))) = outcome {
        return Ok(std::process::Output {
            status,
            stdout: out,
            stderr: err,
        });
    }
    drop((stdout, stderr));
    if let Err(error) = Box::into_pin(child.kill()).await {
        let message = format!("OmniVoice process exit could not be confirmed: {error}");
        if let Some(scope) = scope {
            scope.report_failure("OmniVoice Python inference", &message);
        }
        return Err(TtsError::SynthesisFailed(message));
    }
    match outcome {
        Some(Err(error)) => Err(TtsError::SynthesisFailed(format!(
            "OmniVoice process output: {error}"
        ))),
        _ => Err(TtsError::Cancelled),
    }
}

fn extra_string(extra: &HashMap<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = extra.get(*key).and_then(value_to_string) {
            return Some(value);
        }
    }
    None
}

fn extra_u32(extra: &HashMap<String, Value>, keys: &[&str]) -> Option<u32> {
    for key in keys {
        if let Some(value) = extra.get(*key).and_then(value_to_u32) {
            return Some(value);
        }
    }
    None
}

fn extra_f32(extra: &HashMap<String, Value>, keys: &[&str]) -> Option<f32> {
    for key in keys {
        if let Some(value) = extra.get(*key).and_then(value_to_f32) {
            return Some(value);
        }
    }
    None
}

fn extra_bool(extra: &HashMap<String, Value>, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(value) = extra.get(*key).and_then(value_to_bool) {
            return Some(value);
        }
    }
    None
}

fn value_to_string(value: &Value) -> Option<String> {
    let raw = value.as_str()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn value_to_u32(value: &Value) -> Option<u32> {
    if let Some(value) = value.as_u64() {
        return u32::try_from(value).ok();
    }
    value.as_str()?.trim().parse().ok()
}

fn value_to_f32(value: &Value) -> Option<f32> {
    if let Some(value) = value.as_f64() {
        return Some(value as f32);
    }
    value.as_str()?.trim().parse().ok()
}

fn value_to_bool(value: &Value) -> Option<bool> {
    if let Some(value) = value.as_bool() {
        return Some(value);
    }
    match value.as_str()?.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn build_pythonpath(project_path: &Path) -> OsString {
    let mut paths = vec![project_path.to_path_buf()];
    if let Some(existing) = std::env::var_os("PYTHONPATH") {
        paths.extend(std::env::split_paths(&existing));
    }

    std::env::join_paths(paths).unwrap_or_else(|_| project_path.as_os_str().to_os_string())
}

fn process_output_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    if text.len() > 2000 {
        format!("{}...", text.chars().take(2000).collect::<String>())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn cancelling_native_inference_waits_for_process_exit() {
        let temporary = tempfile::tempdir().unwrap();
        let marker = temporary.path().join("started.pid");
        let mut command = Command::new("node");
        command
            .args(["-e", "require('node:fs').writeFileSync(process.argv[1], String(process.pid)); setInterval(() => {}, 1000)"])
            .arg(&marker)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let stop = CancellationToken::new();
        let child_stop = stop.clone();
        let worker = tokio::spawn(async move { run_python(command, &child_stop, None).await });
        let pid = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(value) = fs::read_to_string(&marker).await {
                    if value.parse::<u32>().is_ok() {
                        break value;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        stop.cancel();
        assert!(matches!(worker.await.unwrap(), Err(TtsError::Cancelled)));
        let exited = Command::new("node")
            .args(["-e", "try { process.kill(Number(process.argv[1]), 0); process.exitCode = 1 } catch(e) { process.exitCode = e.code === 'ESRCH' ? 0 : 2 }", &pid])
            .status()
            .await
            .unwrap()
            .success();
        assert!(exited);
    }

    fn provider_config(extra: HashMap<String, Value>) -> ProviderConfig {
        ProviderConfig {
            id: "omnivoice".to_string(),
            provider_type: "omnivoice".to_string(),
            enabled: true,
            api_key: None,
            api_key_env: None,
            base_url: None,
            endpoint: None,
            model: Some("custom/OmniVoice".to_string()),
            default_voice: None,
            model_path: None,
            extra,
        }
    }

    #[test]
    fn builds_request_for_voice_cloning() {
        let mut extra = HashMap::new();
        extra.insert("ref_audio_path".to_string(), json!("D:/voice/ref.wav"));
        extra.insert("prompt_text".to_string(), json!("reference transcript"));
        extra.insert("num_step".to_string(), json!(16));
        extra.insert("guidance_scale".to_string(), json!(2.5));
        extra.insert("speed".to_string(), json!(0.9));
        extra.insert("preprocess_prompt".to_string(), json!(false));

        let provider = OmniVoiceProvider::from_config(&provider_config(extra)).unwrap();
        let request = provider
            .build_request("hello", &TtsParams::default(), Path::new("D:/tmp/out.wav"))
            .unwrap();

        assert_eq!(request.model, "custom/OmniVoice");
        assert_eq!(request.text, "hello");
        assert_eq!(request.output, "D:/tmp/out.wav");
        assert_eq!(request.ref_audio, "D:/voice/ref.wav");
        assert_eq!(request.ref_text.as_deref(), Some("reference transcript"));
        assert_eq!(request.num_step, Some(16));
        assert_eq!(request.guidance_scale, Some(2.5));
        assert_eq!(request.speed, Some(0.9));
        assert_eq!(request.preprocess_prompt, Some(false));
    }

    #[test]
    fn ref_audio_is_required() {
        let provider = OmniVoiceProvider::from_config(&provider_config(HashMap::new())).unwrap();
        let err = provider
            .build_request("hello", &TtsParams::default(), Path::new("out.wav"))
            .unwrap_err();

        assert!(err.to_string().contains("ref_audio"));
    }
}
