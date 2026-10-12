//! In-memory preference values. Keep this module independent of GTK
//! widgets so loading, migration and UI behavior can be tested separately.

/// View mode used only when creating a new tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Icons,
    Compact,
    Details,
}

impl ViewMode {
    pub fn as_config_value(self) -> &'static str {
        match self {
            Self::Icons => "icons",
            Self::Compact => "compact",
            Self::Details => "details",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "icons" => Some(Self::Icons),
            "compact" => Some(Self::Compact),
            "details" => Some(Self::Details),
            _ => None,
        }
    }
}

/// How pointer activation opens an item. Keyboard activation is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenItems {
    #[default]
    DoubleClick,
    SingleClick,
}

pub const ICON_ZOOM_MIN: u8 = 60;
pub const ICON_ZOOM_MAX: u8 = 180;
pub const ICON_ZOOM_STEP: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetailColumnWidths {
    pub name: i32,
    pub size: i32,
    pub kind: i32,
    pub modified: i32,
}

impl DetailColumnWidths {
    pub const DEFAULT: Self = Self {
        name: 240,
        size: 110,
        kind: 200,
        modified: 170,
    };
    pub fn clamped(self) -> Self {
        Self {
            name: self.name.clamp(120, 1600),
            size: self.size.clamp(70, 1200),
            kind: self.kind.clamp(100, 1200),
            modified: self.modified.clamp(110, 1200),
        }
    }
}

impl OpenItems {
    pub fn as_config_value(self) -> &'static str {
        match self {
            Self::DoubleClick => "double-click",
            Self::SingleClick => "single-click",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "double-click" => Some(Self::DoubleClick),
            "single-click" => Some(Self::SingleClick),
            _ => None,
        }
    }
}

/// A manually chosen emulator is kept even if it later disappears from PATH.
/// The terminal layer detects that state and falls back to Automatic.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TerminalChoice {
    #[default]
    Automatic,
    Emulator(String),
}

impl TerminalChoice {
    pub fn as_config_value(&self) -> String {
        match self {
            Self::Automatic => "auto".to_string(),
            Self::Emulator(program) => format!("emulator:{program}"),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if matches!(value.to_ascii_lowercase().as_str(), "auto" | "automatic") {
            return Some(Self::Automatic);
        }
        let (kind, program) = value.split_once(':')?;
        if kind.trim().eq_ignore_ascii_case("emulator") && !program.trim().is_empty() {
            Some(Self::Emulator(program.trim().to_string()))
        } else {
            None
        }
    }
}

/// Window controls shown in the header bar. `follow_system` (default)
/// leaves the system decoration layout untouched; otherwise only the
/// selected buttons are shown on the right in minimize, maximize, close
/// order. All three may be hidden. Custom choices are preserved while
/// following the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowControls {
    pub follow_system: bool,
    pub show_minimize: bool,
    pub show_maximize: bool,
    pub show_close: bool,
}

impl Default for WindowControls {
    fn default() -> Self {
        Self {
            follow_system: true,
            show_minimize: true,
            show_maximize: true,
            show_close: true,
        }
    }
}

impl WindowControls {
    /// `None` means "no app override, follow the system live".
    /// `Some(layout)` is only for custom mode and always keeps the left
    /// side empty so controls appear exclusively on the right.
    pub fn decoration_layout(&self) -> Option<String> {
        if self.follow_system {
            None
        } else {
            Some(self.custom_layout())
        }
    }

    /// Custom `GtkWindowControls:decoration-layout` value, e.g.
    /// `":minimize,maximize,close"` or `":"` when everything is hidden.
    pub fn custom_layout(&self) -> String {
        let mut right = Vec::new();
        if self.show_minimize {
            right.push("minimize");
        }
        if self.show_maximize {
            right.push("maximize");
        }
        if self.show_close {
            right.push("close");
        }
        format!(":{}", right.join(","))
    }

    fn as_config_bool(value: bool) -> &'static str {
        if value {
            "true"
        } else {
            "false"
        }
    }

    fn parse_bool(value: &str) -> Option<bool> {
        match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    }
}

/// All preferences for one process. New options can be added here without
/// tying persistence to the dialog implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    /// Per-binding overrides: `Some(accelerator)` replaces the default and
    /// `None` explicitly disables the binding. Missing keys use defaults.
    pub keyboard_shortcuts: std::collections::BTreeMap<String, Option<String>>,
    pub default_view: ViewMode,
    pub open_items: OpenItems,
    pub terminal: TerminalChoice,
    pub language: kito_i18n::AppLang,
    pub window_controls: WindowControls,
    pub icon_zoom: u8,
    pub show_size_column: bool,
    pub show_type_column: bool,
    pub show_modified_column: bool,
    pub show_status_item_count: bool,
    pub show_status_zoom_controls: bool,
    pub detail_column_widths: DetailColumnWidths,
    /// Stable, non-localized sidebar row identifiers hidden in this app.
    pub hidden_sidebar_ids: Vec<String>,
    /// GIO URIs added to the sidebar; these never touch shared GTK bookmarks.
    pub custom_sidebar_uris: Vec<String>,
    /// Kito-only favorites. Kept separate from shared Freedesktop bookmarks.
    pub favorite_uris: Vec<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            keyboard_shortcuts: std::collections::BTreeMap::new(),
            default_view: ViewMode::default(),
            open_items: OpenItems::default(),
            terminal: TerminalChoice::default(),
            language: kito_i18n::AppLang::default(),
            window_controls: WindowControls::default(),
            icon_zoom: 100,
            show_size_column: true,
            show_type_column: true,
            show_modified_column: false,
            show_status_item_count: true,
            show_status_zoom_controls: true,
            detail_column_widths: DetailColumnWidths::DEFAULT,
            hidden_sidebar_ids: Vec::new(),
            custom_sidebar_uris: Vec::new(),
            favorite_uris: Vec::new(),
        }
    }
}

impl Preferences {
    pub fn from_config_text(text: &str) -> Self {
        let mut result = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "keyboard-shortcuts" => result.keyboard_shortcuts = decode_shortcuts(value),
                "default-view" => {
                    result.default_view = ViewMode::parse(value).unwrap_or_default();
                }
                "open-items" => {
                    result.open_items = OpenItems::parse(value).unwrap_or_default();
                }
                "terminal" => {
                    result.terminal = TerminalChoice::parse(value).unwrap_or_default();
                }
                "language" => {
                    result.language =
                        kito_i18n::AppLang::parse(value).unwrap_or(kito_i18n::AppLang::System);
                }
                "window-controls-follow-system" => {
                    // Missing or invalid: automatic mode.
                    result.window_controls.follow_system =
                        WindowControls::parse_bool(value).unwrap_or(true);
                }
                "window-controls-minimize" => {
                    result.window_controls.show_minimize =
                        WindowControls::parse_bool(value).unwrap_or(true);
                }
                "window-controls-maximize" => {
                    result.window_controls.show_maximize =
                        WindowControls::parse_bool(value).unwrap_or(true);
                }
                "window-controls-close" => {
                    result.window_controls.show_close =
                        WindowControls::parse_bool(value).unwrap_or(true);
                }
                "icon-zoom" => {
                    result.icon_zoom = value
                        .trim()
                        .parse::<u8>()
                        .ok()
                        .filter(|zoom| (ICON_ZOOM_MIN..=ICON_ZOOM_MAX).contains(zoom))
                        .unwrap_or(100);
                }
                "show-status-item-count" => {
                    result.show_status_item_count =
                        WindowControls::parse_bool(value).unwrap_or(true);
                }
                "show-status-zoom-controls" => {
                    result.show_status_zoom_controls =
                        WindowControls::parse_bool(value).unwrap_or(true);
                }
                "show-size-column" => {
                    result.show_size_column = WindowControls::parse_bool(value).unwrap_or(true);
                }
                "show-type-column" => {
                    result.show_type_column = WindowControls::parse_bool(value).unwrap_or(true);
                }
                "show-modified-column" => {
                    result.show_modified_column =
                        WindowControls::parse_bool(value).unwrap_or(false);
                }
                "details-name-width" => {
                    result.detail_column_widths.name = parse_width(value, 120, 1600, 240)
                }
                "details-size-width" => {
                    result.detail_column_widths.size = parse_width(value, 70, 1200, 110)
                }
                "details-type-width" => {
                    result.detail_column_widths.kind = parse_width(value, 100, 1200, 200)
                }
                "details-modified-width" => {
                    result.detail_column_widths.modified = parse_width(value, 110, 1200, 170)
                }
                "sidebar-hidden" => result.hidden_sidebar_ids = decode_list(value),
                "sidebar-custom" => result.custom_sidebar_uris = decode_list(value),
                "favorites" => result.favorite_uris = decode_list(value),
                _ => {}
            }
        }
        result
    }

    pub fn config_entries(&self) -> [(&'static str, String); 22] {
        [
            (
                "keyboard-shortcuts",
                encode_shortcuts(&self.keyboard_shortcuts),
            ),
            (
                "default-view",
                self.default_view.as_config_value().to_string(),
            ),
            ("open-items", self.open_items.as_config_value().to_string()),
            ("terminal", self.terminal.as_config_value()),
            ("language", self.language.config_str().to_string()),
            (
                "window-controls-follow-system",
                WindowControls::as_config_bool(self.window_controls.follow_system).to_string(),
            ),
            (
                "window-controls-minimize",
                WindowControls::as_config_bool(self.window_controls.show_minimize).to_string(),
            ),
            (
                "window-controls-maximize",
                WindowControls::as_config_bool(self.window_controls.show_maximize).to_string(),
            ),
            (
                "window-controls-close",
                WindowControls::as_config_bool(self.window_controls.show_close).to_string(),
            ),
            ("icon-zoom", self.icon_zoom.to_string()),
            (
                "show-size-column",
                WindowControls::as_config_bool(self.show_size_column).to_string(),
            ),
            (
                "show-type-column",
                WindowControls::as_config_bool(self.show_type_column).to_string(),
            ),
            (
                "show-status-item-count",
                WindowControls::as_config_bool(self.show_status_item_count).to_string(),
            ),
            (
                "show-status-zoom-controls",
                WindowControls::as_config_bool(self.show_status_zoom_controls).to_string(),
            ),
            (
                "show-modified-column",
                WindowControls::as_config_bool(self.show_modified_column).to_string(),
            ),
            ("sidebar-hidden", encode_list(&self.hidden_sidebar_ids)),
            ("sidebar-custom", encode_list(&self.custom_sidebar_uris)),
            ("favorites", encode_list(&self.favorite_uris)),
            (
                "details-name-width",
                self.detail_column_widths.name.to_string(),
            ),
            (
                "details-size-width",
                self.detail_column_widths.size.to_string(),
            ),
            (
                "details-type-width",
                self.detail_column_widths.kind.to_string(),
            ),
            (
                "details-modified-width",
                self.detail_column_widths.modified.to_string(),
            ),
        ]
    }
}

// Hex-encoded `id<TAB>value` records; `-` is an explicit disabled value.
// Encoding the complete map in one setting makes reset deletion-safe with
// the shared merge writer and preserves unknown future IDs.
fn encode_shortcuts(values: &std::collections::BTreeMap<String, Option<String>>) -> String {
    values
        .iter()
        .map(|(id, value)| {
            let record = format!("{}\t{}", id, value.as_deref().unwrap_or("-"));
            record
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn decode_shortcuts(value: &str) -> std::collections::BTreeMap<String, Option<String>> {
    value
        .trim()
        .split(',')
        .filter_map(|encoded| {
            if encoded.len() % 2 != 0 {
                return None;
            }
            let bytes = (0..encoded.len())
                .step_by(2)
                .map(|i| {
                    Some(
                        hex_digit(encoded.as_bytes()[i])? * 16
                            + hex_digit(encoded.as_bytes()[i + 1])?,
                    )
                })
                .collect::<Option<Vec<_>>>()?;
            let text = String::from_utf8(bytes).ok()?;
            let (id, shortcut) = text.split_once('\t')?;
            if id.is_empty() {
                return None;
            }
            Some((
                id.to_string(),
                (shortcut != "-").then(|| shortcut.to_string()),
            ))
        })
        .collect()
}

fn parse_width(value: &str, min: i32, max: i32, default: i32) -> i32 {
    value
        .trim()
        .parse::<i32>()
        .ok()
        .filter(|width| (min..=max).contains(width))
        .unwrap_or(default)
}

// Hex encoding makes commas, newlines and arbitrary UTF-8 safe in the
// line-oriented settings file. Empty lists encode as an empty value.
fn encode_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| {
            value
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn decode_list(value: &str) -> Vec<String> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    value
        .trim()
        .split(',')
        .filter_map(|item| {
            let encoded = item.as_bytes();
            if encoded.len() % 2 != 0 {
                return None;
            }
            let bytes = (0..encoded.len())
                .step_by(2)
                .map(|i| Some(hex_digit(encoded[i])? * 16 + hex_digit(encoded[i + 1])?))
                .collect::<Option<Vec<_>>>()?;
            String::from_utf8(bytes).ok()
        })
        .collect()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Returns whether a saved terminal is absent from the current emulator list.
pub fn saved_terminal_unavailable(choice: &TerminalChoice, available: &[String]) -> bool {
    match choice {
        TerminalChoice::Automatic => false,
        TerminalChoice::Emulator(program) => !available.iter().any(|name| name == program),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_the_existing_view_and_double_click_behavior() {
        let preferences = Preferences::default();
        assert_eq!(preferences.default_view, ViewMode::Icons);
        assert_eq!(preferences.open_items, OpenItems::DoubleClick);
        assert_eq!(preferences.terminal, TerminalChoice::Automatic);
        assert_eq!(preferences.language, kito_i18n::AppLang::System);
        assert_eq!(preferences.window_controls, WindowControls::default());
        assert_eq!(preferences.icon_zoom, 100);
        assert!(preferences.show_size_column && preferences.show_type_column);
        assert!(!preferences.show_modified_column);
        assert_eq!(
            preferences.detail_column_widths,
            DetailColumnWidths::DEFAULT
        );
        assert!(preferences.hidden_sidebar_ids.is_empty());
        assert!(preferences.custom_sidebar_uris.is_empty());
        assert!(preferences.keyboard_shortcuts.is_empty());
    }

    #[test]
    fn shortcut_overrides_round_trip_disabled_unknown_and_escaped_values() {
        let mut shortcuts = std::collections::BTreeMap::new();
        shortcuts.insert("undo".into(), Some("<Control><Shift>z".into()));
        shortcuts.insert("paste".into(), None);
        shortcuts.insert("future-binding".into(), Some("custom,\nvalue".into()));
        let preferences = Preferences {
            keyboard_shortcuts: shortcuts,
            ..Preferences::default()
        };
        let text = preferences
            .config_entries()
            .iter()
            .map(|(key, value)| format!("{key}={value}\n"))
            .collect::<String>();
        assert_eq!(Preferences::from_config_text(&text), preferences);
        let invalid = Preferences::from_config_text("keyboard-shortcuts=xx,ff\n");
        assert!(invalid.keyboard_shortcuts.is_empty());
    }

    #[test]
    fn detail_column_widths_parse_bounds_and_round_trip() {
        let parsed = Preferences::from_config_text(
            "details-name-width=320\ndetails-size-width=1\ndetails-type-width=1601\ndetails-modified-width=bad\nfuture=value\n",
        );
        assert_eq!(
            parsed.detail_column_widths,
            DetailColumnWidths {
                name: 320,
                size: 110,
                kind: 200,
                modified: 170,
            }
        );

        let widths = DetailColumnWidths {
            name: 1600,
            size: 70,
            kind: 1200,
            modified: 1199,
        };
        let preferences = Preferences {
            detail_column_widths: widths,
            ..Preferences::default()
        };
        let text = preferences
            .config_entries()
            .into_iter()
            .map(|(key, value)| format!("{key}={value}\n"))
            .collect::<String>();
        assert_eq!(Preferences::from_config_text(&text), preferences);
        assert_eq!(
            DetailColumnWidths {
                name: -1,
                size: 5000,
                kind: 50,
                modified: 2000,
            }
            .clamped(),
            DetailColumnWidths {
                name: 120,
                size: 1200,
                kind: 100,
                modified: 1200,
            }
        );
    }

    #[test]
    fn sidebar_lists_round_trip_delimiters_newlines_and_unicode_and_ignore_bad_items() {
        let preferences = Preferences {
            hidden_sidebar_ids: vec!["builtin:home".into(), "comma,\n🚀".into()],
            custom_sidebar_uris: vec!["smb://host/share/a,b".into()],
            ..Preferences::default()
        };
        let text = preferences
            .config_entries()
            .iter()
            .map(|(k, v)| format!("{k} = {v}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(Preferences::from_config_text(&text), preferences);
        assert_eq!(
            Preferences::from_config_text("future=value\n").hidden_sidebar_ids,
            Vec::<String>::new()
        );
        assert_eq!(decode_list("ff,61,ø"), vec!["a"]);
    }

    #[test]
    fn favorites_have_an_independent_settings_key_and_round_trip() {
        let favorites = vec![
            "file:///tmp/a".to_string(),
            "smb://host/share/é,one\nline".to_string(),
        ];
        let preferences = Preferences {
            favorite_uris: favorites.clone(),
            custom_sidebar_uris: vec!["file:///tmp/custom".into()],
            ..Preferences::default()
        };
        let entries = preferences.config_entries();
        let favorites_entry = entries.iter().find(|(key, _)| *key == "favorites").unwrap();
        assert_eq!(favorites_entry.1, encode_list(&favorites));
        let restored =
            Preferences::from_config_text(&format!("favorites = {}\n", favorites_entry.1));
        assert_eq!(restored.favorite_uris, favorites);
        assert!(restored.custom_sidebar_uris.is_empty());
    }

    #[test]
    fn icon_zoom_and_detail_columns_parse_with_safe_fallbacks() {
        let preferences = Preferences::from_config_text(
            "icon-zoom = 160\nshow-size-column = false\nshow-type-column = true\nshow-modified-column = true\n",
        );
        assert_eq!(preferences.icon_zoom, 160);
        assert!(!preferences.show_size_column);
        assert!(preferences.show_type_column && preferences.show_modified_column);

        let invalid = Preferences::from_config_text(
            "icon-zoom = 250\nshow-size-column = maybe\nshow-type-column = false\nshow-modified-column = no\n",
        );
        assert_eq!(invalid.icon_zoom, 100);
        assert!(invalid.show_size_column);
        assert!(!invalid.show_type_column && !invalid.show_modified_column);
    }

    #[test]
    fn status_bar_items_default_on_and_round_trip_independently() {
        let defaults = Preferences::default();
        assert!(defaults.show_status_item_count);
        assert!(defaults.show_status_zoom_controls);

        let parsed = Preferences::from_config_text(
            "show-status-item-count = false\nshow-status-zoom-controls = true\n",
        );
        assert!(!parsed.show_status_item_count);
        assert!(parsed.show_status_zoom_controls);

        let entries = parsed.config_entries();
        assert_eq!(
            entries
                .iter()
                .find(|(key, _)| *key == "show-status-item-count")
                .unwrap()
                .1,
            "false"
        );
        assert_eq!(
            entries
                .iter()
                .find(|(key, _)| *key == "show-status-zoom-controls")
                .unwrap()
                .1,
            "true"
        );

        let invalid = Preferences::from_config_text(
            "show-status-item-count = maybe\nshow-status-zoom-controls = invalid\n",
        );
        assert!(invalid.show_status_item_count);
        assert!(invalid.show_status_zoom_controls);
    }

    #[test]
    fn window_controls_default_to_follow_system_with_all_visible() {
        let controls = WindowControls::default();
        assert!(controls.follow_system);
        assert!(controls.show_minimize);
        assert!(controls.show_maximize);
        assert!(controls.show_close);
        assert_eq!(controls.decoration_layout(), None);
        assert_eq!(controls.custom_layout(), ":minimize,maximize,close");
    }

    #[test]
    fn invalid_values_fall_back_to_reliable_defaults() {
        let preferences = Preferences::from_config_text(
            "default-view=columns\nopen-items=triple-click\nterminal=emulator:\n",
        );
        assert_eq!(preferences, Preferences::default());
    }

    #[test]
    fn saved_terminal_is_reported_missing_without_changing_the_choice() {
        let choice = TerminalChoice::Emulator("old-term".to_string());
        assert!(saved_terminal_unavailable(&choice, &["xterm".to_string()]));
        assert_eq!(choice, TerminalChoice::Emulator("old-term".to_string()));
    }

    #[test]
    fn language_choice_is_parsed_and_invalid_values_follow_system() {
        let manual = Preferences::from_config_text("language = it\n");
        assert_eq!(manual.language, kito_i18n::AppLang::Italian);
        let english = Preferences::from_config_text("language=en\n");
        assert_eq!(english.language, kito_i18n::AppLang::English);
        let invalid = Preferences::from_config_text("language=klingon\n");
        assert_eq!(invalid.language, kito_i18n::AppLang::System);
    }

    #[test]
    fn window_controls_layout_covers_all_button_combinations() {
        // (minimize, maximize, close) -> right side of the layout, always in
        // minimize, maximize, close order, left side always empty.
        let cases = [
            ((true, true, true), ":minimize,maximize,close"),
            ((true, true, false), ":minimize,maximize"),
            ((true, false, true), ":minimize,close"),
            ((true, false, false), ":minimize"),
            ((false, true, true), ":maximize,close"),
            ((false, true, false), ":maximize"),
            ((false, false, true), ":close"),
            ((false, false, false), ":"),
        ];
        for ((minimize, maximize, close), expected) in cases {
            let controls = WindowControls {
                follow_system: false,
                show_minimize: minimize,
                show_maximize: maximize,
                show_close: close,
            };
            assert_eq!(controls.custom_layout(), expected);
            assert_eq!(controls.decoration_layout(), Some(expected.to_string()));
        }
        // Automatic mode never overrides: no static copy of the system layout.
        let automatic = WindowControls {
            follow_system: true,
            show_minimize: false,
            show_maximize: false,
            show_close: false,
        };
        assert_eq!(automatic.decoration_layout(), None);
    }

    #[test]
    fn window_controls_missing_or_invalid_values_use_automatic_mode() {
        assert_eq!(
            Preferences::from_config_text("").window_controls,
            WindowControls::default()
        );
        let invalid = Preferences::from_config_text(
            "window-controls-follow-system=maybe\nwindow-controls-minimize=2\nwindow-controls-maximize=\nwindow-controls-close=perhaps\n",
        );
        assert_eq!(invalid.window_controls, WindowControls::default());
        assert_eq!(invalid.window_controls.decoration_layout(), None);
    }

    #[test]
    fn window_controls_custom_choices_parse_and_round_trip() {
        let parsed = Preferences::from_config_text(
            "window-controls-follow-system=false\nwindow-controls-minimize=false\nwindow-controls-maximize=true\nwindow-controls-close=0\n",
        );
        assert_eq!(
            parsed.window_controls,
            WindowControls {
                follow_system: false,
                show_minimize: false,
                show_maximize: true,
                show_close: false,
            }
        );
        assert_eq!(
            parsed.window_controls.decoration_layout(),
            Some(":maximize".to_string())
        );
        // Entries round-trip through text without losing the custom choice.
        let entries = parsed.config_entries();
        let text = entries
            .iter()
            .map(|(key, value)| format!("{key} = {value}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            Preferences::from_config_text(&text).window_controls,
            parsed.window_controls
        );
    }

    #[test]
    fn window_controls_custom_choices_survive_automatic_mode() {
        let custom = WindowControls {
            follow_system: false,
            show_minimize: true,
            show_maximize: false,
            show_close: true,
        };
        assert_eq!(
            custom.decoration_layout(),
            Some(":minimize,close".to_string())
        );
        // Switching to automatic keeps the custom flags untouched.
        let automatic = WindowControls {
            follow_system: true,
            ..custom
        };
        assert_eq!(automatic.decoration_layout(), None);
        // Back to custom restores the same layout.
        let back = WindowControls {
            follow_system: false,
            ..automatic
        };
        assert_eq!(back, custom);
        assert_eq!(
            back.decoration_layout(),
            Some(":minimize,close".to_string())
        );
    }
}
