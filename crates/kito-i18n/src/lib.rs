//! Application localization: system language detection, user language
//! preference, and Fluent message catalogs (English + Italian).
//!
//! Catalogs are embedded with `include_str!`, so they work both from
//! `cargo run` and from an installed binary, with no locale directory
//! lookup. gettext was evaluated and discarded: it needs compiled `.mo`
//! files plus a locale directory resolvable at runtime and a `msgfmt`
//! build dependency, while Fluent is pure Rust with first-class plurals
//! and parameterized messages.

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentResource, FluentValue};
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use unic_langid::LanguageIdentifier;

/// Re-exported so UI code builds arguments without depending on Fluent.
pub use fluent_bundle::FluentArgs;

const EN_SOURCE: &str = include_str!("../locales/en.ftl");
const IT_SOURCE: &str = include_str!("../locales/it.ftl");

/// Language choice stored in the preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppLang {
    /// Follow the system language (default).
    #[default]
    System,
    English,
    Italian,
}

impl AppLang {
    /// Parses a config value. Liberal on purpose; unknown values are
    /// rejected by the caller, which falls back to [`AppLang::System`].
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "system" | "auto" => Some(Self::System),
            "en" | "english" => Some(Self::English),
            "it" | "italian" | "italiano" => Some(Self::Italian),
            _ => None,
        }
    }

    /// Stable value written to the config file.
    pub fn config_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::English => "en",
            Self::Italian => "it",
        }
    }
}

/// Effective runtime language. Anything unsupported falls back here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    English,
    Italian,
}

/// Matches a locale tag to a supported language by primary subtag:
/// `it_IT` and `it_CH` use Italian, `en_US` and `en_GB` use English.
/// Encoding suffixes (`.UTF-8`) and modifiers (`@euro`) are stripped,
/// both BCP47 (`-`) and POSIX (`_`) separators accepted. `C`, `POSIX`,
/// empty and unsupported tags match nothing.
fn match_tag(tag: &str) -> Option<AppLang> {
    let base = tag.split(['.', '@']).next().unwrap_or(tag);
    let normalized = base.replace('-', "_");
    let primary = normalized.split('_').next().unwrap_or("");
    match primary.to_lowercase().as_str() {
        "it" => Some(AppLang::Italian),
        "en" => Some(AppLang::English),
        _ => None,
    }
}

fn is_c_locale(tag: &str) -> bool {
    matches!(
        tag.split(['.', '@'])
            .next()
            .unwrap_or(tag)
            .to_ascii_uppercase()
            .as_str(),
        "C" | "POSIX"
    )
}

/// Detects the language from explicit inputs (testable): the gettext
/// `LANGUAGE` priority list first (first supported entry wins), when a
/// message locale is active, then a consolidated system tag. GNU `C` and
/// `POSIX` locales disable `LANGUAGE`, matching gettext behavior.
pub fn detect_from(language_var: Option<&str>, system_tag: Option<&str>) -> Option<AppLang> {
    let active_locale = system_tag.filter(|tag| !is_c_locale(tag));
    if active_locale.is_some() {
        if let Some(list) = language_var {
            for part in list.split(':') {
                if let Some(lang) = match_tag(part) {
                    return Some(lang);
                }
            }
        }
    }
    system_tag.and_then(match_tag)
}

/// Detects the language from the live environment: the `LANGUAGE`
/// message override plus the `sys-locale` consolidated signal, which
/// follows platform conventions (`LC_ALL`, `LC_MESSAGES`, `LANG`).
/// Returns `None` when undetectable or unsupported.
pub fn detect_system() -> Option<AppLang> {
    // sys-locale provides ordered candidates from LANGUAGE, LC_ALL,
    // LC_MESSAGES, and LANG. gettext ignores LANGUAGE when the effective
    // message locale is C/POSIX, so preserve that specific Linux rule.
    let active_locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty());
    let active_locale = active_locale?;
    if is_c_locale(&active_locale) {
        return None;
    }
    sys_locale::get_locales().find_map(|tag| match_tag(&tag))
}

/// Resolves the effective language: a manual choice always wins, System
/// follows detection, anything unknown falls back to English.
pub fn resolve(choice: AppLang, detected: Option<AppLang>) -> Lang {
    match choice {
        AppLang::English => Lang::English,
        AppLang::Italian => Lang::Italian,
        AppLang::System => match detected {
            Some(AppLang::Italian) => Lang::Italian,
            _ => Lang::English,
        },
    }
}

const CONFIG_DIR_NAME: &str = "kito-files";
const CONFIG_FILE_NAME: &str = "settings.conf";
const CONFIG_KEY: &str = "language";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Config file path (`~/.config/kito-files/settings.conf`, honoring
/// `XDG_CONFIG_HOME`). `None` when no home can be determined.
pub fn config_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })?;
    Some(base.join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME))
}

/// Reads the language choice from `path`. Missing files and invalid
/// values fall back to [`AppLang::System`]; unknown keys are ignored.
pub fn load_choice_from(path: &std::path::Path) -> AppLang {
    let Ok(text) = std::fs::read_to_string(path) else {
        return AppLang::System;
    };
    let mut choice = AppLang::System;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == CONFIG_KEY {
                choice = AppLang::parse(value).unwrap_or_default();
            }
        }
    }
    choice
}

/// Saves the language choice to `path`, creating folders as needed.
pub fn save_choice_to(path: &std::path::Path, choice: AppLang) -> std::io::Result<()> {
    save_settings_to(path, &[(CONFIG_KEY, choice.config_str().to_string())])
}

/// Merges known settings and atomically replaces the shared config file.
/// Keys not listed by this caller are preserved for other app components.
pub fn save_settings_to(path: &std::path::Path, entries: &[(&str, String)]) -> io::Result<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut output = String::from("# Kito Files settings\n");
    for line in existing.lines() {
        let is_known = line.split_once('=').is_some_and(|(key, _)| {
            entries
                .iter()
                .any(|(candidate, _)| key.trim() == *candidate)
        });
        if !is_known && !line.trim().is_empty() && !line.trim().starts_with('#') {
            output.push_str(line);
            output.push('\n');
        }
    }
    for (key, value) in entries {
        output.push_str(key);
        output.push_str(" = ");
        output.push_str(value);
        output.push('\n');
    }

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "settings path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "settings path has no file name",
        )
    })?;
    let mut temp_path = None;
    let mut temp_file = None;
    for _ in 0..32 {
        let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = std::ffi::OsString::from(".");
        temp_name.push(file_name);
        temp_name.push(format!(".{}.{}.tmp", std::process::id(), serial));
        let candidate = parent.join(temp_name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temp_path = Some(candidate);
                temp_file = Some(file);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let temp_path = temp_path.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a temporary settings file",
        )
    })?;
    let Some(mut temp_file) = temp_file else {
        let _ = std::fs::remove_file(&temp_path);
        return Err(io::Error::other(
            "temporary settings file could not be opened",
        ));
    };
    let result = (|| {
        temp_file.write_all(output.as_bytes())?;
        temp_file.sync_all()?;
        drop(temp_file);
        std::fs::rename(&temp_path, path)?;
        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

/// Reads the choice from the default config path.
pub fn load_choice() -> AppLang {
    config_path()
        .map(|p| load_choice_from(&p))
        .unwrap_or(AppLang::System)
}

/// Saves the choice to the default config path.
pub fn save_choice(choice: AppLang) -> std::io::Result<()> {
    let Some(path) = config_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory available",
        ));
    };
    save_choice_to(&path, choice)
}

fn bundle(tag: &str, source: &str) -> FluentBundle<FluentResource> {
    let langid: LanguageIdentifier = tag.parse().expect("known locale tag");
    let resource = FluentResource::try_new(source.to_string()).expect("bundled catalog parses");
    let mut bundle = FluentBundle::new_concurrent(vec![langid]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(resource)
        .expect("no duplicate messages");
    bundle
}

/// Message catalogs with English fallback. `Sync`: safe in a global.
pub struct I18n {
    primary: FluentBundle<FluentResource>,
    english: FluentBundle<FluentResource>,
    lang: Lang,
}

impl I18n {
    /// Builds catalogs for `choice` (manual wins) and `detected` system
    /// language. Call once at startup, before building the interface.
    pub fn new(choice: AppLang, detected: Option<AppLang>) -> Self {
        Self::for_lang(resolve(choice, detected))
    }

    /// Builds catalogs for an explicit language (tests, previews).
    pub fn for_lang(lang: Lang) -> Self {
        let (tag, source) = match lang {
            Lang::English => ("en-US", EN_SOURCE),
            Lang::Italian => ("it", IT_SOURCE),
        };
        Self {
            primary: bundle(tag, source),
            english: bundle("en-US", EN_SOURCE),
            lang,
        }
    }

    /// Effective language (useful for tests).
    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Plain message. Falls back to English, then to the key itself
    /// (never empty, never panics).
    pub fn tr(&self, id: &str) -> String {
        self.format(id, &FluentArgs::new())
    }

    /// Message with named arguments (no English word-order imposed).
    pub fn tr_with(&self, id: &str, args: &FluentArgs) -> String {
        self.format(id, args)
    }

    /// Plural message over `$count`.
    pub fn tr_num(&self, id: &str, count: u64) -> String {
        let mut args = FluentArgs::new();
        args.set("count", FluentValue::from(count));
        self.format(id, &args)
    }

    fn format(&self, id: &str, args: &FluentArgs) -> String {
        for catalog in [&self.primary, &self.english] {
            if let Some(message) = catalog.get_message(id) {
                if let Some(pattern) = message.value() {
                    let mut errors = Vec::new();
                    return catalog
                        .format_pattern(pattern, Some(args), &mut errors)
                        .into_owned();
                }
            }
        }
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_italian_variants() {
        for tag in [
            "it",
            "it_IT",
            "it_IT.UTF-8",
            "it_CH",
            "it_CH.UTF-8@euro",
            "it-IT",
        ] {
            assert_eq!(
                detect_from(None, Some(tag)),
                Some(AppLang::Italian),
                "{tag}"
            );
        }
    }

    #[test]
    fn detects_english_variants() {
        for tag in [
            "en",
            "en_US",
            "en_US.UTF-8",
            "en_GB",
            "en-GB@euro",
            "C",
            "POSIX",
            "",
        ] {
            let expected = if tag == "C" || tag == "POSIX" || tag.is_empty() {
                None
            } else {
                Some(AppLang::English)
            };
            assert_eq!(detect_from(None, Some(tag)), expected, "{tag}");
        }
        assert_eq!(detect_from(None, None), None);
    }

    #[test]
    fn detects_unsupported_as_none() {
        for tag in ["de", "de_DE.UTF-8", "fr_FR", "es", "zh_CN", "xx_YY"] {
            assert_eq!(detect_from(None, Some(tag)), None, "{tag}");
        }
    }

    #[test]
    fn language_var_wins_over_system_tag() {
        // gettext convention: LANGUAGE overrides everything else.
        assert_eq!(
            detect_from(Some("it_IT:en_US"), Some("en_US")),
            Some(AppLang::Italian)
        );
        assert_eq!(
            detect_from(Some("de:fr:it"), Some("en_US")),
            Some(AppLang::Italian)
        );
        assert_eq!(
            detect_from(Some("de:fr"), Some("en_US")),
            Some(AppLang::English)
        );
        assert_eq!(detect_from(Some(""), Some("it_IT")), Some(AppLang::Italian));
    }

    #[test]
    fn c_and_posix_disable_language_override() {
        assert_eq!(detect_from(Some("it_IT"), Some("C")), None);
        assert_eq!(detect_from(Some("it_IT"), Some("C.UTF-8")), None);
        assert_eq!(detect_from(Some("it_IT"), Some("POSIX")), None);
        assert_eq!(detect_from(Some("it_IT"), Some("POSIX@euro")), None);
        assert_eq!(detect_from(Some("it_IT"), None), None);
    }

    #[test]
    fn resolve_prefers_manual_and_falls_back_to_english() {
        assert_eq!(
            resolve(AppLang::English, Some(AppLang::Italian)),
            Lang::English
        );
        assert_eq!(
            resolve(AppLang::Italian, Some(AppLang::English)),
            Lang::Italian
        );
        assert_eq!(
            resolve(AppLang::System, Some(AppLang::Italian)),
            Lang::Italian
        );
        assert_eq!(
            resolve(AppLang::System, Some(AppLang::English)),
            Lang::English
        );
        assert_eq!(
            resolve(AppLang::System, Some(AppLang::System)),
            Lang::English
        );
        assert_eq!(resolve(AppLang::System, None), Lang::English);
        // Switching back from a manual value to System restores detection.
        assert_eq!(
            resolve(AppLang::System, Some(AppLang::Italian)),
            Lang::Italian
        );
    }

    #[test]
    fn config_roundtrip_and_invalid_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        // Missing file: system default.
        assert_eq!(load_choice_from(&path), AppLang::System);
        save_choice_to(&path, AppLang::Italian).unwrap();
        assert_eq!(load_choice_from(&path), AppLang::Italian);
        // Unknown keys ignored, invalid values fall back to system.
        std::fs::write(&path, "# comment\ntheme = dark\nlanguage = klingon\n").unwrap();
        assert_eq!(load_choice_from(&path), AppLang::System);
        std::fs::write(&path, "language=EN\n").unwrap();
        assert_eq!(load_choice_from(&path), AppLang::English);
        std::fs::write(&path, "language=it\nlanguage=klingon\n").unwrap();
        assert_eq!(load_choice_from(&path), AppLang::System);
    }

    #[test]
    fn missing_translation_falls_back_to_english() {
        let it = I18n::for_lang(Lang::Italian);
        // Present in both.
        assert_eq!(it.tr("menu-open"), "Apri");
        assert_eq!(I18n::for_lang(Lang::English).tr("menu-open"), "Open");
        assert_eq!(it.tr("test-only-english"), "English fallback");
        // Unknown everywhere: the key itself, never empty.
        assert_eq!(it.tr("no-such-key"), "no-such-key");
    }

    #[test]
    fn plurals_in_both_languages() {
        let en = I18n::for_lang(Lang::English);
        let it = I18n::for_lang(Lang::Italian);
        assert_eq!(en.tr_num("status-items", 1), "1 item");
        assert_eq!(en.tr_num("status-items", 0), "0 items");
        assert_eq!(en.tr_num("status-items", 5), "5 items");
        assert_eq!(it.tr_num("status-items", 1), "1 elemento");
        assert_eq!(it.tr_num("status-items", 0), "0 elementi");
        assert_eq!(it.tr_num("status-items", 5), "5 elementi");
    }

    #[test]
    fn parameterized_messages_in_both_languages() {
        let en = I18n::for_lang(Lang::English);
        let it = I18n::for_lang(Lang::Italian);
        let mut args = FluentArgs::new();
        args.set("name", "Notes");
        assert_eq!(en.tr_with("created-file", &args), "Created Notes");
        assert_eq!(it.tr_with("created-file", &args), "File creato: Notes");
        assert_eq!(en.tr_num("pasted-items", 1), "Pasted 1 item");
        assert_eq!(it.tr_num("pasted-items", 3), "Incollati 3 elementi");
        let mut counts = FluentArgs::new();
        counts.set("failed", 1);
        counts.set("total", 4);
        assert_eq!(
            en.tr_with("paste-failed", &counts),
            "Failed to paste 1 of 4 items"
        );
        assert_eq!(
            it.tr_with("paste-failed", &counts),
            "Impossibile incollare 1 elemento su 4"
        );
    }

    #[test]
    fn source_keys_exist_in_both_catalogs() {
        // Every message id used in kito-gtk sources must exist in English;
        // Italian may miss some (English fallback covers it).
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let src = manifest.join("../kito-gtk/src");
        let mut used = std::collections::BTreeSet::new();
        fn collect(path: &std::path::Path, used: &mut std::collections::BTreeSet<String>) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(&path, used);
                    continue;
                }
                if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
                    continue;
                }
                let text = std::fs::read_to_string(path).unwrap();
                for pat in [
                    "tr(\"",
                    "tr_num(\"",
                    "tr_with(\"",
                    "tr_with_one(\"",
                    "tr_with_two_counts(\"",
                ] {
                    let mut rest = text.as_str();
                    let mut offset = 0;
                    while let Some(pos) = rest.find(pat) {
                        let start = offset + pos;
                        rest = &rest[pos + pat.len()..];
                        offset = start + pat.len();
                        let previous = text[..start].chars().next_back().unwrap_or(' ');
                        if previous.is_ascii_alphanumeric() || previous == '_' {
                            continue;
                        }
                        if let Some(end) = rest.find('"') {
                            used.insert(rest[..end].to_string());
                        }
                    }
                }
            }
        }
        collect(&src, &mut used);
        assert!(!used.is_empty(), "key scanner found nothing");
        let en = I18n::for_lang(Lang::English);
        let it = I18n::for_lang(Lang::Italian);
        for id in &used {
            assert_ne!(en.tr(id), *id, "missing English message: {id}");
            assert_ne!(it.tr(id), *id, "missing Italian or English fallback: {id}");
            assert!(
                it.primary.get_message(id).is_some(),
                "missing Italian message: {id}"
            );
        }
    }
}
