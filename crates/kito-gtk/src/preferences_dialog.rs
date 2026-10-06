//! GTK interface for the process-wide preferences model.

use crate::preferences::model::{saved_terminal_unavailable, OpenItems, TerminalChoice, ViewMode};
use crate::preferences::PreferenceStore;
use adw::prelude::*;
use std::{cell::RefCell, rc::Rc};

/// Show the one preferences dialog associated with this window. Repeated
/// activation presents the existing dialog instead of creating another.
pub fn present(
    window: &adw::ApplicationWindow,
    store: &Rc<PreferenceStore>,
    existing: &Rc<RefCell<Option<adw::PreferencesDialog>>>,
) {
    if let Some(dialog) = existing.borrow().as_ref() {
        dialog.present(Some(window));
        return;
    }

    let dialog = adw::PreferencesDialog::builder()
        .title(crate::l10n::tr("prefs-title"))
        .content_width(560)
        .build();

    let general_page = adw::PreferencesPage::builder()
        .title(crate::l10n::tr("prefs-general"))
        .icon_name("preferences-system-symbolic")
        .build();
    let general_group = adw::PreferencesGroup::new();

    let view_labels = [
        crate::l10n::tr("view-icons"),
        crate::l10n::tr("view-compact"),
        crate::l10n::tr("view-details"),
    ];
    let view_refs = view_labels.iter().map(String::as_str).collect::<Vec<_>>();
    let view_options = gtk::StringList::new(&view_refs);
    let view_row = adw::ComboRow::builder()
        .title(crate::l10n::tr("prefs-default-view"))
        .subtitle(crate::l10n::tr("prefs-new-tabs-note"))
        .model(&view_options)
        .build();
    view_row.set_selected(view_index(store.snapshot().default_view));
    {
        let store = store.clone();
        view_row.connect_selected_notify(move |row| {
            let view = match row.selected() {
                1 => ViewMode::Compact,
                2 => ViewMode::Details,
                _ => ViewMode::Icons,
            };
            match store.set_default_view(view) {
                Ok(()) => row.set_subtitle(&crate::l10n::tr("prefs-new-tabs-note")),
                Err(error) => row.set_subtitle(&save_error(&error)),
            }
        });
    }
    general_group.add(&view_row);

    let open_labels = [
        crate::l10n::tr("prefs-double-click"),
        crate::l10n::tr("prefs-single-click"),
    ];
    let open_refs = open_labels.iter().map(String::as_str).collect::<Vec<_>>();
    let open_options = gtk::StringList::new(&open_refs);
    let open_row = adw::ComboRow::builder()
        .title(crate::l10n::tr("prefs-open-items"))
        .model(&open_options)
        .build();
    open_row.set_selected(match store.snapshot().open_items {
        OpenItems::DoubleClick => 0,
        OpenItems::SingleClick => 1,
    });
    {
        let store = store.clone();
        open_row.connect_selected_notify(move |row| {
            let behavior = if row.selected() == 1 {
                OpenItems::SingleClick
            } else {
                OpenItems::DoubleClick
            };
            row.set_subtitle(&save_status(store.set_open_items(behavior)));
        });
    }
    general_group.add(&open_row);

    let language_labels = [
        crate::l10n::tr("prefs-lang-system"),
        crate::l10n::tr("prefs-lang-english"),
        crate::l10n::tr("prefs-lang-italian"),
    ];
    let language_refs = language_labels
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let language_options = gtk::StringList::new(&language_refs);
    let language_row = adw::ComboRow::builder()
        .title(crate::l10n::tr("prefs-language"))
        .subtitle(crate::l10n::tr("prefs-restart-note"))
        .model(&language_options)
        .build();
    language_row.set_selected(match store.snapshot().language {
        kito_i18n::AppLang::System => 0,
        kito_i18n::AppLang::English => 1,
        kito_i18n::AppLang::Italian => 2,
    });
    {
        let store = store.clone();
        language_row.connect_selected_notify(move |row| {
            let language = match row.selected() {
                1 => kito_i18n::AppLang::English,
                2 => kito_i18n::AppLang::Italian,
                _ => kito_i18n::AppLang::System,
            };
            let subtitle = match store.set_language(language) {
                Ok(()) => crate::l10n::tr("prefs-restart-note"),
                Err(error) => save_error(&error),
            };
            row.set_subtitle(&subtitle);
        });
    }
    general_group.add(&language_row);
    general_page.add(&general_group);
    dialog.add(&general_page);

    let integration_page = adw::PreferencesPage::builder()
        .title(crate::l10n::tr("prefs-integration"))
        .icon_name("applications-system-symbolic")
        .build();
    let terminal_group = adw::PreferencesGroup::new();
    let available = crate::terminal::available_terminal_programs();
    let labels = std::iter::once(crate::l10n::tr("prefs-terminal-automatic"))
        .chain(
            available
                .iter()
                .map(|program| crate::terminal::display_name(program)),
        )
        .collect::<Vec<_>>();
    let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let terminal_options = gtk::StringList::new(&label_refs);
    let terminal_row = adw::ComboRow::builder()
        .title(crate::l10n::tr("prefs-terminal"))
        .model(&terminal_options)
        .build();
    let saved_terminal = store.snapshot().terminal;
    let terminal_missing = saved_terminal_unavailable(&saved_terminal, &available);
    let initial_terminal = match &saved_terminal {
        TerminalChoice::Automatic => 0,
        TerminalChoice::Emulator(program) => available
            .iter()
            .position(|candidate| candidate == program)
            .map(|index| index as u32 + 1)
            .unwrap_or(0),
    };
    terminal_row.set_selected(initial_terminal);
    if terminal_missing {
        if let TerminalChoice::Emulator(program) = &saved_terminal {
            terminal_row.set_subtitle(&format!(
                "{}",
                crate::l10n::tr_with_one(
                    "prefs-terminal-missing",
                    "terminal",
                    &crate::terminal::display_name(program)
                )
            ));
        }
    }
    {
        let store = store.clone();
        let available = available.clone();
        terminal_row.connect_selected_notify(move |row| {
            let choice = row
                .selected()
                .checked_sub(1)
                .and_then(|index| available.get(index as usize))
                .map(|program| TerminalChoice::Emulator(program.clone()))
                .unwrap_or(TerminalChoice::Automatic);
            row.set_subtitle(&save_status(store.set_terminal(choice)));
        });
    }
    terminal_group.add(&terminal_row);
    integration_page.add(&terminal_group);
    dialog.add(&integration_page);

    *existing.borrow_mut() = Some(dialog.clone());
    dialog.present(Some(window));
}

fn view_index(mode: ViewMode) -> u32 {
    match mode {
        ViewMode::Icons => 0,
        ViewMode::Compact => 1,
        ViewMode::Details => 2,
    }
}

fn save_error(error: &std::io::Error) -> String {
    crate::l10n::tr_with_one("prefs-save-error", "error", &error.to_string())
}

fn save_status(result: std::io::Result<()>) -> String {
    match result {
        Ok(()) => String::new(),
        Err(error) => save_error(&error),
    }
}
