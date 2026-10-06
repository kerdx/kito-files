//! Runtime language access: a single [`kito_i18n::I18n`] initialized
//! once at startup (before any widget is built) and read everywhere.
//! The language applies on next launch by design (see Preferences),
//! so a read-only global is sufficient and keeps call sites unchanged.

use std::sync::OnceLock;

static CURRENT: OnceLock<kito_i18n::I18n> = OnceLock::new();

/// Installs the resolved catalogs. Call once in `main`, before UI.
pub fn init(i18n: kito_i18n::I18n) {
    let _ = CURRENT.set(i18n);
}

/// Plain message (falls back to English, then to the key itself).
pub(crate) fn tr(id: &str) -> String {
    CURRENT
        .get()
        .map(|i| i.tr(id))
        .unwrap_or_else(|| id.to_string())
}

/// Plural message over `$count`.
pub(crate) fn tr_num(id: &str, count: u64) -> String {
    CURRENT
        .get()
        .map(|i| i.tr_num(id, count))
        .unwrap_or_else(|| format!("{id} {count}"))
}

/// Message with named arguments.
pub(crate) fn tr_with(id: &str, args: &kito_i18n::FluentArgs) -> String {
    CURRENT
        .get()
        .map(|i| i.tr_with(id, args))
        .unwrap_or_else(|| id.to_string())
}

/// Convenience for the common single named value case.
pub(crate) fn tr_with_one(id: &str, name: &str, value: &str) -> String {
    let mut args = kito_i18n::FluentArgs::new();
    args.set(name, value);
    tr_with(id, &args)
}

/// Pluralized message with both `$failed` and `$total` parameters.
pub(crate) fn tr_with_two_counts(id: &str, failed: u64, total: u64) -> String {
    let mut args = kito_i18n::FluentArgs::new();
    args.set("failed", failed);
    args.set("total", total);
    tr_with(id, &args)
}
