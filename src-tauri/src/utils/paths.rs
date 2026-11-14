use std::path::PathBuf;

/// Keep production path semantics; permit an explicit isolated root in debug
/// acceptance runs, where APPDATA does not override Windows Known Folder APIs.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os("KOKORO_TEST_DATA_ROOT") {
        let root = PathBuf::from(root);
        assert!(root.is_absolute(), "KOKORO_TEST_DATA_ROOT must be absolute");
        return Some(root);
    }
    dirs_next::data_dir()
}

#[cfg(test)]
mod tests {
    #[test]
    fn production_fallback_keeps_known_folder_semantics() {
        if std::env::var_os("KOKORO_TEST_DATA_ROOT").is_none() {
            assert_eq!(super::data_dir(), dirs_next::data_dir());
        }
    }
}
