//! File persistence for the process-wide preference model. The shared
//! localization crate owns the config path and atomic key merge.

use super::model::Preferences;
use std::path::{Path, PathBuf};

/// Shared settings file, honoring XDG_CONFIG_HOME.
pub fn config_path() -> Option<PathBuf> {
    kito_i18n::config_path()
}

pub fn load_from(path: &Path) -> Preferences {
    std::fs::read_to_string(path)
        .map(|text| Preferences::from_config_text(&text))
        .unwrap_or_default()
}

/// Merge preference keys into the shared file and atomically replace it.
/// Unrecognized settings keys are retained for other app components.
pub fn save_to(path: &Path, preferences: &Preferences) -> std::io::Result<()> {
    kito_i18n::save_settings_to(path, &preferences.config_entries())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::model::{OpenItems, TerminalChoice, ViewMode};

    #[test]
    fn missing_file_loads_defaults_and_save_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kito-files").join("settings.conf");
        assert_eq!(load_from(&path), Preferences::default());

        let preferences = Preferences {
            default_view: ViewMode::Details,
            open_items: OpenItems::SingleClick,
            terminal: TerminalChoice::Emulator("xterm".to_string()),
            language: kito_i18n::AppLang::Italian,
        };
        save_to(&path, &preferences).unwrap();
        assert_eq!(load_from(&path), preferences);
    }

    #[test]
    fn invalid_values_use_defaults_and_unknown_settings_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        std::fs::write(
            &path,
            "default-view=invalid\nopen-items=single\nterminal=???\nlanguage = klingon\nfuture-option = retained\n",
        )
        .unwrap();
        assert_eq!(load_from(&path), Preferences::default());

        save_to(&path, &Preferences::default()).unwrap();
        let saved = std::fs::read_to_string(path).unwrap();
        assert!(saved.contains("future-option = retained"));
        assert!(saved.contains("default-view = icons"));
        assert!(saved.contains("language = system"));
    }
}
