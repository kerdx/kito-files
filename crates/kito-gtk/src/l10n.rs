//! Runtime language access. The active catalog is initialized before
//! constructing widgets and can be replaced while the application runs.

use std::sync::{OnceLock, RwLock};

static CURRENT: OnceLock<RwLock<kito_i18n::I18n>> = OnceLock::new();

/// Installs the resolved catalogs. Call once in `main`, before UI.
pub fn init(i18n: kito_i18n::I18n) {
    let _ = CURRENT.set(RwLock::new(i18n));
}

/// Replaces the active catalog, then lets every open window refresh its
/// already-created labels and views.
pub fn set_language(choice: kito_i18n::AppLang) {
    let i18n = kito_i18n::I18n::new(choice, kito_i18n::detect_system());
    if let Some(current) = CURRENT.get() {
        if let Ok(mut current) = current.write() {
            *current = i18n;
        }
    } else {
        let _ = CURRENT.set(RwLock::new(i18n));
    }
}

/// Plain message (falls back to English, then to the key itself).
pub(crate) fn tr(id: &str) -> String {
    CURRENT
        .get()
        .and_then(|i| i.read().ok())
        .map(|i| i.tr(id))
        .unwrap_or_else(|| id.to_string())
}

/// Plural message over `$count`.
pub(crate) fn tr_num(id: &str, count: u64) -> String {
    CURRENT
        .get()
        .and_then(|i| i.read().ok())
        .map(|i| i.tr_num(id, count))
        .unwrap_or_else(|| format!("{id} {count}"))
}

/// Message with named arguments.
pub(crate) fn tr_with(id: &str, args: &kito_i18n::FluentArgs) -> String {
    CURRENT
        .get()
        .and_then(|i| i.read().ok())
        .map(|i| i.tr_with(id, args))
        .unwrap_or_else(|| id.to_string())
}

/// Convenience for the common single named value case.
pub(crate) fn tr_with_one(id: &str, name: &str, value: &str) -> String {
    let mut args = kito_i18n::FluentArgs::new();
    args.set(name, value);
    tr_with(id, &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_can_be_switched_while_running() {
        set_language(kito_i18n::AppLang::English);
        assert_eq!(tr("menu-about"), "About Kito Files");

        set_language(kito_i18n::AppLang::Italian);
        assert_eq!(tr("menu-about"), "Informazioni su Kito Files");

        set_language(kito_i18n::AppLang::English);
        assert_eq!(tr("menu-about"), "About Kito Files");
    }
}
