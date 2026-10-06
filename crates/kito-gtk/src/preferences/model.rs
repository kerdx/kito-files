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

/// All preferences for one process. New options can be added here without
/// tying persistence to the dialog implementation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preferences {
    pub default_view: ViewMode,
    pub open_items: OpenItems,
    pub terminal: TerminalChoice,
    pub language: kito_i18n::AppLang,
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
                _ => {}
            }
        }
        result
    }

    pub fn config_entries(&self) -> [(&'static str, String); 4] {
        [
            (
                "default-view",
                self.default_view.as_config_value().to_string(),
            ),
            ("open-items", self.open_items.as_config_value().to_string()),
            ("terminal", self.terminal.as_config_value()),
            ("language", self.language.config_str().to_string()),
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
}
