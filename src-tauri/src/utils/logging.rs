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

pub fn format_log_line(level: &str, target: &str, message: &str, with_color: bool) -> String {
    if with_color {
        format!("[{level}][{target}] {message}")
    } else {
        format!("[{level}][{target}] {message}")
    }
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
}
