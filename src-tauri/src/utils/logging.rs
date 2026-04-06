use std::fmt;

use tracing::field::{Field, Visit};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModulePalette {
    Ai,
    Tts,
    Stt,
    Mcp,
    Vision,
    ImageGen,
    Tools,
    Pet,
    Default,
}

pub fn module_palette(target: &str) -> ModulePalette {
    match target {
        "ai" => ModulePalette::Ai,
        "tts" => ModulePalette::Tts,
        "stt" => ModulePalette::Stt,
        "mcp" => ModulePalette::Mcp,
        "vision" => ModulePalette::Vision,
        "imagegen" => ModulePalette::ImageGen,
        "tools" => ModulePalette::Tools,
        "pet" => ModulePalette::Pet,
        _ => ModulePalette::Default,
    }
}

fn level_ansi(level: &str) -> &'static str {
    match level {
        "ERROR" => "\u{1b}[31m",
        "WARN" => "\u{1b}[33m",
        "INFO" => "\u{1b}[32m",
        "DEBUG" => "\u{1b}[34m",
        "TRACE" => "\u{1b}[90m",
        _ => "\u{1b}[37m",
    }
}

fn target_ansi(target: &str) -> &'static str {
    match module_palette(target) {
        ModulePalette::Ai => "\u{1b}[95m",
        ModulePalette::Tts => "\u{1b}[96m",
        ModulePalette::Stt => "\u{1b}[36m",
        ModulePalette::Mcp => "\u{1b}[94m",
        ModulePalette::Vision => "\u{1b}[35m",
        ModulePalette::ImageGen => "\u{1b}[92m",
        ModulePalette::Tools => "\u{1b}[93m",
        ModulePalette::Pet => "\u{1b}[91m",
        ModulePalette::Default => "\u{1b}[37m",
    }
}

pub fn color_enabled() -> bool {
    use std::io::IsTerminal;

    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

pub fn format_log_line(level: &str, target: &str, message: &str, with_color: bool) -> String {
    if with_color {
        let level = format!("{}{}\u{1b}[0m", level_ansi(level), level);
        let target = format!("{}{}\u{1b}[0m", target_ansi(target), target);
        format!("[{level}][{target}] {message}")
    } else {
        format!("[{level}][{target}] {message}")
    }
}

struct LogLineFormatter {
    with_color: bool,
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    extra: Vec<(String, String)>,
}

impl MessageVisitor {
    fn normalize_debug_value(value: String) -> String {
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            value[1..value.len() - 1].to_string()
        } else {
            value
        }
    }

    fn into_message(self) -> String {
        if let Some(msg) = self.message {
            if self.extra.is_empty() {
                msg
            } else {
                let extras = self
                    .extra
                    .into_iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{} {}", msg, extras)
            }
        } else if self.extra.is_empty() {
            String::new()
        } else {
            self.extra
                .into_iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let raw = format!("{:?}", value);
        let normalized = Self::normalize_debug_value(raw);
        if field.name() == "message" {
            self.message = Some(normalized);
        } else {
            self.extra.push((field.name().to_string(), normalized));
        }
    }
}

impl<S, N> FormatEvent<S, N> for LogLineFormatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        let level = meta.level().as_str();
        let target = meta.target();

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.into_message();

        let line = format_log_line(level, target, &message, self.with_color);
        writeln!(writer, "{}", line)
    }
}

pub fn init_logging() {
    let with_color = color_enabled();

    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .with_level(false)
        .event_format(LogLineFormatter { with_color })
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_line_keeps_structure_without_color() {
        let line = format_log_line("INFO", "ai", "Restored memory_enabled=true", false);
        assert!(line.starts_with("[INFO][ai] "));
        assert!(line.contains("Restored memory_enabled=true"));
        assert!(!line.contains("\u{1b}["));
    }

    #[test]
    fn module_palette_returns_default_for_unknown_target() {
        assert_eq!(module_palette("unknown-target"), ModulePalette::Default);
    }

    #[test]
    fn module_palette_maps_known_targets() {
        assert_eq!(module_palette("ai"), ModulePalette::Ai);
        assert_eq!(module_palette("mcp"), ModulePalette::Mcp);
    }

    #[test]
    fn module_palette_maps_context_related_ai_target() {
        assert_eq!(module_palette("ai"), ModulePalette::Ai);
    }

    #[test]
    fn module_palette_maps_mcp_tts_stt_targets() {
        assert_eq!(module_palette("mcp"), ModulePalette::Mcp);
        assert_eq!(module_palette("tts"), ModulePalette::Tts);
        assert_eq!(module_palette("stt"), ModulePalette::Stt);
    }

    #[test]
    fn module_palette_maps_vision_imagegen_pet_tools_targets() {
        assert_eq!(module_palette("vision"), ModulePalette::Vision);
        assert_eq!(module_palette("imagegen"), ModulePalette::ImageGen);
        assert_eq!(module_palette("pet"), ModulePalette::Pet);
        assert_eq!(module_palette("tools"), ModulePalette::Tools);
    }

    #[test]
    fn format_line_contains_ansi_when_color_enabled() {
        let line = format_log_line("ERROR", "mcp", "connection failed", true);
        assert!(line.contains("\u{1b}["));
    }

    #[test]
    fn format_line_no_ansi_when_color_disabled() {
        let line = format_log_line("ERROR", "mcp", "connection failed", false);
        assert!(!line.contains("\u{1b}["));
    }
}
