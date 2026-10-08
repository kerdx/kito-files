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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preferences {
    pub default_view: ViewMode,
    pub open_items: OpenItems,
    pub terminal: TerminalChoice,
    pub language: kito_i18n::AppLang,
    pub window_controls: WindowControls,
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
                _ => {}
            }
        }
        result
    }

    pub fn config_entries(&self) -> [(&'static str, String); 8] {
        [
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
        ]
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
