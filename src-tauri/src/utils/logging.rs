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

pub fn init_logging() {
    let with_color = color_enabled();

    let subscriber = tracing_subscriber::fmt()
        .with_ansi(with_color)
        .with_target(true)
        .with_level(true)
        .event_format(tracing_subscriber::fmt::format().compact())
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
