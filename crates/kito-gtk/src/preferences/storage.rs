//! File persistence for the process-wide preference model. The shared
//! localization crate owns the config path and atomic key merge.

use super::model::Preferences;
use std::path::{Path, PathBuf};

#[cfg(test)]
use gtk::gio;

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

/// One-time migration requested for legacy Kito custom locations. Bookmark
/// writes are idempotent; the legacy list is cleared only after all writes
/// and the settings save succeed.
pub fn migrate_custom_locations_to_bookmarks(
    settings_path: &Path,
    bookmarks_path: &Path,
) -> Result<bool, String> {
    let mut preferences = load_from(settings_path);
    if preferences.custom_sidebar_uris.is_empty() {
        return Ok(false);
    }
    for uri in &preferences.custom_sidebar_uris {
        let name = kito_core::uri_file_name(uri)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| kito_core::uri_to_display(uri));
        kito_core::bookmarks::pin_to(bookmarks_path, &name, uri)
            .map_err(|error| error.to_string())?;
    }
    for uri in &preferences.custom_sidebar_uris {
        let old = format!("custom:{uri}");
        if let Some(index) = preferences
            .hidden_sidebar_ids
            .iter()
            .position(|item| item == &old)
        {
            preferences.hidden_sidebar_ids[index] =
                format!("bookmark:{}", kito_core::bookmarks::normalize_uri(uri));
        }
    }
    preferences.custom_sidebar_uris.clear();
    save_to(settings_path, &preferences).map_err(|error| error.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::model::{
        DetailColumnWidths, OpenItems, TerminalChoice, ViewMode, WindowControls,
    };
    use gio::prelude::*;

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
            window_controls: WindowControls {
                follow_system: false,
                show_minimize: true,
                show_maximize: false,
                show_close: true,
            },
            icon_zoom: 140,
            show_size_column: false,
            show_type_column: true,
            show_modified_column: true,
            detail_column_widths: DetailColumnWidths {
                name: 320,
                size: 95,
                kind: 280,
                modified: 210,
            },
            ..Preferences::default()
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
            "default-view=invalid\nopen-items=single\nterminal=???\nlanguage = klingon\nicon-zoom = invalid\nshow-size-column = maybe\nfuture-option = retained\n",
        )
        .unwrap();
        assert_eq!(load_from(&path), Preferences::default());

        save_to(&path, &Preferences::default()).unwrap();
        let saved = std::fs::read_to_string(path).unwrap();
        assert!(saved.contains("future-option = retained"));
        assert!(saved.contains("default-view = icons"));
        assert!(saved.contains("language = system"));
        assert!(saved.contains("window-controls-follow-system = true"));
    }

    #[test]
    fn window_controls_persist_and_invalid_values_use_automatic_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        let custom = Preferences {
            window_controls: WindowControls {
                follow_system: false,
                show_minimize: false,
                show_maximize: true,
                show_close: false,
            },
            ..Preferences::default()
        };
        save_to(&path, &custom).unwrap();
        assert_eq!(load_from(&path), custom);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("window-controls-follow-system = false"));
        assert!(saved.contains("window-controls-minimize = false"));
        assert!(saved.contains("window-controls-maximize = true"));

        std::fs::write(
            &path,
            "window-controls-follow-system = maybe\nwindow-controls-minimize = 2\n",
        )
        .unwrap();
        assert_eq!(load_from(&path), Preferences::default());
    }

    #[test]
    fn legacy_locations_migrate_idempotently_and_preserve_hidden_ids() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.conf");
        let bookmarks = dir.path().join("gtk-3.0/bookmarks");
        let folder = dir.path().join("folder");
        let file = dir.path().join("file.txt");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(&file, b"ok").unwrap();
        let folder_uri = gio::File::for_path(&folder).uri().to_string();
        let file_uri = gio::File::for_path(&file).uri().to_string();
        let prefs = Preferences {
            custom_sidebar_uris: vec![folder_uri.clone(), file_uri.clone()],
            hidden_sidebar_ids: vec![format!("custom:{folder_uri}")],
            ..Preferences::default()
        };
        save_to(&settings, &prefs).unwrap();
        std::fs::create_dir_all(bookmarks.parent().unwrap()).unwrap();
        std::fs::write(&bookmarks, "# unrelated\nsmb://host/share Existing label\n").unwrap();

        assert!(migrate_custom_locations_to_bookmarks(&settings, &bookmarks).unwrap());
        assert!(!migrate_custom_locations_to_bookmarks(&settings, &bookmarks).unwrap());
        let migrated = load_from(&settings);
        assert!(migrated.custom_sidebar_uris.is_empty());
        assert_eq!(
            migrated.hidden_sidebar_ids,
            [format!("bookmark:{folder_uri}")]
        );
        let text = std::fs::read_to_string(bookmarks).unwrap();
        assert!(text.starts_with("# unrelated\nsmb://host/share Existing label\n"));
        assert!(text.contains(&format!("{folder_uri} folder")));
        assert!(text.contains(&format!("{file_uri} file.txt")));
        assert_eq!(text.matches(&folder_uri).count(), 1);
    }

    #[test]
    fn failed_custom_location_migration_keeps_legacy_settings() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.conf");
        let bookmarks = dir.path().join("invalid-bookmarks");
        let uri = "file:///tmp/custom";
        let prefs = Preferences {
            custom_sidebar_uris: vec![uri.into()],
            ..Preferences::default()
        };
        save_to(&settings, &prefs).unwrap();
        std::fs::write(&bookmarks, [0xff, 0xfe]).unwrap();
        assert!(migrate_custom_locations_to_bookmarks(&settings, &bookmarks).is_err());
        assert_eq!(load_from(&settings).custom_sidebar_uris, [uri]);
        assert_eq!(std::fs::read(bookmarks).unwrap(), [0xff, 0xfe]);
    }

    #[test]
    fn migration_normalizes_trailing_slash_in_hidden_bookmark_id() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.conf");
        let bookmarks = dir.path().join("bookmarks");
        let uri = "file:///tmp/cartella/";
        let prefs = Preferences {
            custom_sidebar_uris: vec![uri.into()],
            hidden_sidebar_ids: vec![format!("custom:{uri}"), "custom:file:///tmp/other".into()],
            ..Preferences::default()
        };
        save_to(&settings, &prefs).unwrap();

        assert!(migrate_custom_locations_to_bookmarks(&settings, &bookmarks).unwrap());
        let migrated = load_from(&settings);
        assert!(migrated.custom_sidebar_uris.is_empty());
        assert_eq!(
            migrated.hidden_sidebar_ids,
            ["bookmark:file:///tmp/cartella", "custom:file:///tmp/other"]
        );
        assert_eq!(
            std::fs::read_to_string(bookmarks).unwrap(),
            "file:///tmp/cartella cartella\n"
        );
        assert_eq!(kito_core::bookmarks::normalize_uri("file:///"), "file:///");
    }
}
