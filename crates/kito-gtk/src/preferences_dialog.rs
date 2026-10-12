//! GTK interface for the process-wide preferences model.

use crate::preferences::model::{saved_terminal_unavailable, OpenItems, TerminalChoice, ViewMode};
use crate::preferences::PreferenceStore;
use adw::prelude::*;
use std::{cell::RefCell, rc::Rc};

/// The one preferences dialog owned by a window. Keeping references to its
/// rows lets a language change update the currently open dialog in place.
pub struct PreferencesDialogState {
    pub dialog: adw::PreferencesDialog,
    general_page: adw::PreferencesPage,
    integration_page: adw::PreferencesPage,
    shortcuts_page: adw::PreferencesPage,
    shortcuts_group: adw::PreferencesGroup,
    shortcut_rows: Vec<(
        adw::ActionRow,
        gtk::Button,
        gtk::Button,
        gtk::Button,
        &'static str,
    )>,
    shortcut_reset_all: gtk::Button,
    status_group: adw::PreferencesGroup,
    status_count_row: adw::SwitchRow,
    status_zoom_row: adw::SwitchRow,
    status_count_handler: glib::SignalHandlerId,
    status_zoom_handler: glib::SignalHandlerId,
    default_view_row: adw::ComboRow,
    default_view_options: gtk::StringList,
    default_view_handler: glib::SignalHandlerId,
    open_items_row: adw::ComboRow,
    open_items_options: gtk::StringList,
    open_items_handler: glib::SignalHandlerId,
    language_row: adw::ComboRow,
    language_options: gtk::StringList,
    language_handler: glib::SignalHandlerId,
    window_group: adw::PreferencesGroup,
    follow_system_row: adw::SwitchRow,
    follow_system_handler: glib::SignalHandlerId,
    minimize_row: adw::SwitchRow,
    minimize_handler: glib::SignalHandlerId,
    maximize_row: adw::SwitchRow,
    maximize_handler: glib::SignalHandlerId,
    close_row: adw::SwitchRow,
    close_handler: glib::SignalHandlerId,
    terminal_row: adw::ComboRow,
    terminal_options: gtk::StringList,
    terminal_handler: glib::SignalHandlerId,
    available_terminals: Vec<String>,
}

pub type SharedDialog = Rc<RefCell<Option<PreferencesDialogState>>>;

/// Show the one preferences dialog associated with this window. Repeated
/// activation presents the existing dialog instead of creating another.
pub fn present(
    window: &adw::ApplicationWindow,
    store: &Rc<PreferenceStore>,
    existing: &SharedDialog,
) {
    if let Some(state) = existing.borrow().as_ref() {
        state.dialog.present(Some(window));
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

    let view_labels = view_labels();
    let view_refs = view_labels.iter().map(String::as_str).collect::<Vec<_>>();
    let default_view_options = gtk::StringList::new(&view_refs);
    let default_view_row = adw::ComboRow::builder()
        .title(crate::l10n::tr("prefs-default-view"))
        .subtitle(crate::l10n::tr("prefs-new-tabs-note"))
        .model(&default_view_options)
        .build();
    default_view_row.set_selected(view_index(store.snapshot().default_view));
    let default_view_handler = {
        let store = store.clone();
        default_view_row.connect_selected_notify(move |row| {
            let view = match row.selected() {
                1 => ViewMode::Compact,
                2 => ViewMode::Details,
                _ => ViewMode::Icons,
            };
            match store.set_default_view(view) {
                Ok(()) => row.set_subtitle(&crate::l10n::tr("prefs-new-tabs-note")),
                Err(error) => row.set_subtitle(&save_error(&error)),
            }
        })
    };
    general_group.add(&default_view_row);

    let open_labels = open_labels();
    let open_refs = open_labels.iter().map(String::as_str).collect::<Vec<_>>();
    let open_items_options = gtk::StringList::new(&open_refs);
    let open_items_row = adw::ComboRow::builder()
        .title(crate::l10n::tr("prefs-open-items"))
        .model(&open_items_options)
        .build();
    open_items_row.set_selected(open_items_index(store.snapshot().open_items));
    let open_items_handler = {
        let store = store.clone();
        open_items_row.connect_selected_notify(move |row| {
            let behavior = if row.selected() == 1 {
                OpenItems::SingleClick
            } else {
                OpenItems::DoubleClick
            };
            row.set_subtitle(&save_status(store.set_open_items(behavior)));
        })
    };
    general_group.add(&open_items_row);

    let language_labels = language_labels();
    let language_refs = language_labels
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let language_options = gtk::StringList::new(&language_refs);
    let language_row = adw::ComboRow::builder()
        .title(crate::l10n::tr("prefs-language"))
        .subtitle(crate::l10n::tr("prefs-language-applied"))
        .model(&language_options)
        .build();
    language_row.set_selected(language_index(store.snapshot().language));
    let language_handler = {
        let store = store.clone();
        language_row.connect_selected_notify(move |row| {
            let language = language_at(row.selected());
            let subtitle = match store.set_language(language) {
                Ok(()) => crate::l10n::tr("prefs-language-applied"),
                Err(error) => save_error(&error),
            };
            row.set_subtitle(&subtitle);
        })
    };
    general_group.add(&language_row);
    general_page.add(&general_group);

    let window_controls = store.snapshot().window_controls;
    let window_group = adw::PreferencesGroup::builder()
        .title(crate::l10n::tr("prefs-window-controls"))
        .build();
    let follow_system_row = adw::SwitchRow::builder()
        .title(crate::l10n::tr("prefs-follow-system"))
        .active(window_controls.follow_system)
        .build();
    let minimize_row = adw::SwitchRow::builder()
        .title(crate::l10n::tr("prefs-show-minimize"))
        .active(window_controls.show_minimize)
        .sensitive(!window_controls.follow_system)
        .build();
    let maximize_row = adw::SwitchRow::builder()
        .title(crate::l10n::tr("prefs-show-maximize"))
        .active(window_controls.show_maximize)
        .sensitive(!window_controls.follow_system)
        .build();
    let close_row = adw::SwitchRow::builder()
        .title(crate::l10n::tr("prefs-show-close"))
        .active(window_controls.show_close)
        .sensitive(!window_controls.follow_system)
        .build();
    let minimize_weak = minimize_row.downgrade();
    let maximize_weak = maximize_row.downgrade();
    let close_weak = close_row.downgrade();
    let follow_system_handler = {
        let store = store.clone();
        follow_system_row.connect_active_notify(move |row| {
            let follow = row.is_active();
            row.set_subtitle(&save_status(
                store.set_window_controls_follow_system(follow),
            ));
            if let Some(minimize) = minimize_weak.upgrade() {
                minimize.set_sensitive(!follow);
            }
            if let Some(maximize) = maximize_weak.upgrade() {
                maximize.set_sensitive(!follow);
            }
            if let Some(close) = close_weak.upgrade() {
                close.set_sensitive(!follow);
            }
        })
    };
    let minimize_handler = {
        let store = store.clone();
        minimize_row.connect_active_notify(move |row| {
            row.set_subtitle(&save_status(
                store.set_window_controls_minimize(row.is_active()),
            ));
        })
    };
    let maximize_handler = {
        let store = store.clone();
        maximize_row.connect_active_notify(move |row| {
            row.set_subtitle(&save_status(
                store.set_window_controls_maximize(row.is_active()),
            ));
        })
    };
    let close_handler = {
        let store = store.clone();
        close_row.connect_active_notify(move |row| {
            row.set_subtitle(&save_status(
                store.set_window_controls_close(row.is_active()),
            ));
        })
    };
    window_group.add(&follow_system_row);
    window_group.add(&minimize_row);
    window_group.add(&maximize_row);
    window_group.add(&close_row);
    general_page.add(&window_group);
    let status_group = adw::PreferencesGroup::builder()
        .title(crate::l10n::tr("prefs-status-bar"))
        .build();
    let status_count_row = adw::SwitchRow::builder()
        .title(crate::l10n::tr("prefs-status-item-count"))
        .active(store.snapshot().show_status_item_count)
        .build();
    let status_zoom_row = adw::SwitchRow::builder()
        .title(crate::l10n::tr("prefs-status-zoom-controls"))
        .active(store.snapshot().show_status_zoom_controls)
        .build();
    let status_count_handler = {
        let store = store.clone();
        status_count_row.connect_active_notify(move |row| {
            row.set_subtitle(&save_status(
                store.set_show_status_item_count(row.is_active()),
            ))
        })
    };
    let status_zoom_handler = {
        let store = store.clone();
        status_zoom_row.connect_active_notify(move |row| {
            row.set_subtitle(&save_status(
                store.set_show_status_zoom_controls(row.is_active()),
            ))
        })
    };
    status_group.add(&status_count_row);
    status_group.add(&status_zoom_row);
    general_page.add(&status_group);
    dialog.add(&general_page);

    let shortcuts_page = adw::PreferencesPage::builder()
        .title(crate::l10n::tr("prefs-shortcuts"))
        .icon_name("input-keyboard-symbolic")
        .build();
    let shortcuts_group = adw::PreferencesGroup::builder()
        .title(crate::l10n::tr("prefs-shortcuts-group"))
        .description(crate::l10n::tr("prefs-shortcuts-note"))
        .build();
    let reset_all = gtk::Button::with_label(&crate::l10n::tr("prefs-shortcuts-reset-all"));
    {
        let store = store.clone();
        let weak_group = shortcuts_group.downgrade();
        reset_all.connect_clicked(move |_| {
            let result = store.reset_shortcuts();
            if let Some(group) = weak_group.upgrade() {
                group.set_description(Some(&match result {
                    Ok(()) => crate::l10n::tr("prefs-shortcuts-note"),
                    Err(error) => save_error(&error),
                }));
            }
        });
    }
    shortcuts_group.set_header_suffix(Some(&reset_all));
    let mut shortcut_rows = Vec::new();
    for binding in crate::shortcuts::CATALOG {
        let row = adw::ActionRow::builder()
            .title(crate::l10n::tr(&format!("shortcut-{}", binding.id)))
            .build();
        let current = crate::shortcuts::effective(binding, &store.snapshot())
            .and_then(|accel| {
                gtk::accelerator_parse(&accel)
                    .map(|(key, mods)| gtk::accelerator_get_label(key, mods).to_string())
            })
            .unwrap_or_else(|| crate::l10n::tr("prefs-shortcuts-disabled"));
        row.set_subtitle(&current);
        let edit = gtk::Button::with_label(&crate::l10n::tr("prefs-shortcuts-edit"));
        let clear = gtk::Button::with_label(&crate::l10n::tr("prefs-shortcuts-clear"));
        let reset = gtk::Button::with_label(&crate::l10n::tr("prefs-shortcuts-reset"));
        let store_edit = store.clone();
        let id = binding.id;
        edit.connect_clicked(move |button| {
            let Some(root) = button.root().and_downcast::<gtk::Window>() else {
                return;
            };
            capture_shortcut(&root, store_edit.clone(), id);
        });
        let store_clear = store.clone();
        let id = binding.id;
        let weak_row = row.downgrade();
        clear.connect_clicked(move |_| {
            if let Err(error) = store_clear.disable_shortcut(id) {
                if let Some(row) = weak_row.upgrade() {
                    row.set_subtitle(&save_error(&error));
                }
            }
        });
        let store_reset = store.clone();
        let id = binding.id;
        let weak_row = row.downgrade();
        reset.connect_clicked(move |_| {
            if let Err(error) = store_reset.set_shortcut(id, None) {
                if let Some(row) = weak_row.upgrade() {
                    row.set_subtitle(&save_error(&error));
                }
            }
        });
        row.add_suffix(&edit);
        row.add_suffix(&clear);
        row.add_suffix(&reset);
        shortcuts_group.add(&row);
        shortcut_rows.push((row, edit, clear, reset, binding.id));
    }
    let weak_rows = shortcut_rows
        .iter()
        .map(|(row, _, _, _, id)| (row.downgrade(), *id))
        .collect::<Vec<_>>();
    store.subscribe_shortcuts(Rc::new(move |prefs| {
        for (weak, id) in &weak_rows {
            if let Some(row) = weak.upgrade() {
                row.set_subtitle(&shortcut_label(id, &prefs));
            }
        }
    }));
    shortcuts_page.add(&shortcuts_group);
    dialog.add(&shortcuts_page);

    let integration_page = adw::PreferencesPage::builder()
        .title(crate::l10n::tr("prefs-integration"))
        .icon_name("applications-system-symbolic")
        .build();
    let terminal_group = adw::PreferencesGroup::new();
    let available_terminals = crate::terminal::available_terminal_programs();
    let terminal_labels = terminal_labels(&available_terminals);
    let terminal_refs = terminal_labels
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let terminal_options = gtk::StringList::new(&terminal_refs);
    let terminal_row = adw::ComboRow::builder()
        .title(crate::l10n::tr("prefs-terminal"))
        .model(&terminal_options)
        .build();
    let saved_terminal = store.snapshot().terminal;
    terminal_row.set_selected(terminal_index(&saved_terminal, &available_terminals));
    if let TerminalChoice::Emulator(program) = &saved_terminal {
        if saved_terminal_unavailable(&saved_terminal, &available_terminals) {
            terminal_row.set_subtitle(&terminal_missing(program));
        }
    }
    let terminal_handler = {
        let store = store.clone();
        let available = available_terminals.clone();
        terminal_row.connect_selected_notify(move |row| {
            let choice = row
                .selected()
                .checked_sub(1)
                .and_then(|index| available.get(index as usize))
                .map(|program| TerminalChoice::Emulator(program.clone()))
                .unwrap_or(TerminalChoice::Automatic);
            row.set_subtitle(&save_status(store.set_terminal(choice)));
        })
    };
    terminal_group.add(&terminal_row);
    integration_page.add(&terminal_group);
    dialog.add(&integration_page);

    *existing.borrow_mut() = Some(PreferencesDialogState {
        dialog: dialog.clone(),
        general_page,
        integration_page,
        shortcuts_page,
        shortcuts_group,
        shortcut_rows,
        shortcut_reset_all: reset_all,
        status_group,
        status_count_row,
        status_zoom_row,
        status_count_handler,
        status_zoom_handler,
        default_view_row,
        default_view_options,
        default_view_handler,
        open_items_row,
        open_items_options,
        open_items_handler,
        language_row,
        language_options,
        language_handler,
        window_group,
        follow_system_row,
        follow_system_handler,
        minimize_row,
        minimize_handler,
        maximize_row,
        maximize_handler,
        close_row,
        close_handler,
        terminal_row,
        terminal_options,
        terminal_handler,
        available_terminals,
    });
    dialog.present(Some(window));
}

fn capture_shortcut(parent: &gtk::Window, store: Rc<PreferenceStore>, id: &'static str) {
    let dialog = gtk::Window::builder()
        .title(crate::l10n::tr("prefs-shortcuts-capture-title"))
        .transient_for(parent)
        .modal(true)
        .default_width(360)
        .default_height(120)
        .build();
    let label = gtk::Label::new(Some(&crate::l10n::tr("prefs-shortcuts-capture-note")));
    label.set_margin_top(24);
    label.set_margin_bottom(24);
    label.set_margin_start(18);
    label.set_margin_end(18);
    dialog.set_child(Some(&label));
    let controller = gtk::EventControllerKey::new();
    let weak = dialog.downgrade();
    controller.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gtk::gdk::Key::Escape {
            if let Some(dialog) = weak.upgrade() {
                dialog.close();
            }
            return gtk::glib::Propagation::Stop;
        }
        let relevant = modifiers
            & (gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SHIFT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK);
        let printable = key.to_unicode().is_some_and(|c| !c.is_control());
        if relevant.is_empty() && printable {
            return gtk::glib::Propagation::Stop;
        }
        let accelerator = gtk::accelerator_name(key, relevant).to_string();
        let Some((parsed_key, parsed_mods)) = gtk::accelerator_parse(&accelerator) else {
            return gtk::glib::Propagation::Stop;
        };
        if !crate::shortcuts::valid_accelerator(&accelerator) {
            return gtk::glib::Propagation::Stop;
        }
        if let Some(conflict) = crate::shortcuts::CATALOG
            .iter()
            .filter(|item| item.id != id)
            .find(|item| {
                crate::shortcuts::effective(item, &store.snapshot()).is_some_and(|value| {
                    gtk::accelerator_parse(&value) == Some((parsed_key, parsed_mods))
                })
            })
        {
            label.set_text(&crate::l10n::tr_with_one(
                "prefs-shortcuts-conflict",
                "action",
                &crate::l10n::tr(&format!("shortcut-{}", conflict.id)),
            ));
            return gtk::glib::Propagation::Stop;
        }
        if let Err(error) = store.set_shortcut(id, Some(accelerator)) {
            label.set_text(&save_error(&error));
            return gtk::glib::Propagation::Stop;
        }
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
        gtk::glib::Propagation::Stop
    });
    dialog.add_controller(controller);
    dialog.present();
}

/// Relabels the open dialog in place without changing the selected values.
pub fn retranslate(state: &PreferencesDialogState, store: &PreferenceStore) {
    let snapshot = store.snapshot();
    state.dialog.set_title(&crate::l10n::tr("prefs-title"));
    state
        .general_page
        .set_title(&crate::l10n::tr("prefs-general"));
    state
        .integration_page
        .set_title(&crate::l10n::tr("prefs-integration"));
    state
        .shortcuts_page
        .set_title(&crate::l10n::tr("prefs-shortcuts"));
    state
        .shortcuts_group
        .set_title(&crate::l10n::tr("prefs-shortcuts-group"));
    state
        .shortcuts_group
        .set_description(Some(&crate::l10n::tr("prefs-shortcuts-note")));
    state
        .shortcut_reset_all
        .set_label(&crate::l10n::tr("prefs-shortcuts-reset-all"));
    for (row, edit, clear, reset, id) in &state.shortcut_rows {
        row.set_title(&crate::l10n::tr(&format!("shortcut-{id}")));
        row.set_subtitle(&shortcut_label(id, &snapshot));
        edit.set_label(&crate::l10n::tr("prefs-shortcuts-edit"));
        clear.set_label(&crate::l10n::tr("prefs-shortcuts-clear"));
        reset.set_label(&crate::l10n::tr("prefs-shortcuts-reset"));
    }
    state
        .status_group
        .set_title(&crate::l10n::tr("prefs-status-bar"));
    for (row, handler, title, active) in [
        (
            &state.status_count_row,
            &state.status_count_handler,
            "prefs-status-item-count",
            snapshot.show_status_item_count,
        ),
        (
            &state.status_zoom_row,
            &state.status_zoom_handler,
            "prefs-status-zoom-controls",
            snapshot.show_status_zoom_controls,
        ),
    ] {
        row.set_title(&crate::l10n::tr(title));
        row.block_signal(handler);
        row.set_active(active);
        row.unblock_signal(handler);
        row.set_subtitle("");
    }
    state
        .default_view_row
        .set_title(&crate::l10n::tr("prefs-default-view"));
    state
        .default_view_row
        .block_signal(&state.default_view_handler);
    replace_strings(&state.default_view_options, &view_labels());
    state
        .default_view_row
        .set_selected(view_index(snapshot.default_view));
    state
        .default_view_row
        .unblock_signal(&state.default_view_handler);
    state
        .default_view_row
        .set_subtitle(&crate::l10n::tr("prefs-new-tabs-note"));

    state
        .open_items_row
        .set_title(&crate::l10n::tr("prefs-open-items"));
    state.open_items_row.block_signal(&state.open_items_handler);
    replace_strings(&state.open_items_options, &open_labels());
    state
        .open_items_row
        .set_selected(open_items_index(snapshot.open_items));
    state
        .open_items_row
        .unblock_signal(&state.open_items_handler);
    state.open_items_row.set_subtitle("");

    state
        .language_row
        .set_title(&crate::l10n::tr("prefs-language"));
    state.language_row.block_signal(&state.language_handler);
    replace_strings(&state.language_options, &language_labels());
    state
        .language_row
        .set_selected(language_index(snapshot.language));
    state.language_row.unblock_signal(&state.language_handler);
    state
        .language_row
        .set_subtitle(&crate::l10n::tr("prefs-language-applied"));

    state
        .window_group
        .set_title(&crate::l10n::tr("prefs-window-controls"));
    state
        .follow_system_row
        .set_title(&crate::l10n::tr("prefs-follow-system"));
    state
        .follow_system_row
        .block_signal(&state.follow_system_handler);
    state
        .follow_system_row
        .set_active(snapshot.window_controls.follow_system);
    state
        .follow_system_row
        .unblock_signal(&state.follow_system_handler);
    state.follow_system_row.set_subtitle("");
    for (row, handler, title, active) in [
        (
            &state.minimize_row,
            &state.minimize_handler,
            "prefs-show-minimize",
            snapshot.window_controls.show_minimize,
        ),
        (
            &state.maximize_row,
            &state.maximize_handler,
            "prefs-show-maximize",
            snapshot.window_controls.show_maximize,
        ),
        (
            &state.close_row,
            &state.close_handler,
            "prefs-show-close",
            snapshot.window_controls.show_close,
        ),
    ] {
        row.set_title(&crate::l10n::tr(title));
        row.block_signal(handler);
        row.set_active(active);
        row.unblock_signal(handler);
        row.set_sensitive(!snapshot.window_controls.follow_system);
        row.set_subtitle("");
    }

    state
        .terminal_row
        .set_title(&crate::l10n::tr("prefs-terminal"));
    state.terminal_row.block_signal(&state.terminal_handler);
    replace_strings(
        &state.terminal_options,
        &terminal_labels(&state.available_terminals),
    );
    state.terminal_row.set_selected(terminal_index(
        &snapshot.terminal,
        &state.available_terminals,
    ));
    state.terminal_row.unblock_signal(&state.terminal_handler);
    let subtitle = match &snapshot.terminal {
        TerminalChoice::Emulator(program)
            if saved_terminal_unavailable(&snapshot.terminal, &state.available_terminals) =>
        {
            terminal_missing(program)
        }
        _ => String::new(),
    };
    state.terminal_row.set_subtitle(&subtitle);
}

fn replace_strings(model: &gtk::StringList, values: &[String]) {
    let refs = values.iter().map(String::as_str).collect::<Vec<_>>();
    model.splice(0, model.n_items(), &refs);
}

fn view_labels() -> Vec<String> {
    ["view-icons", "view-compact", "view-details"]
        .into_iter()
        .map(crate::l10n::tr)
        .collect()
}

fn open_labels() -> Vec<String> {
    ["prefs-double-click", "prefs-single-click"]
        .into_iter()
        .map(crate::l10n::tr)
        .collect()
}

fn language_labels() -> Vec<String> {
    [
        "prefs-lang-system",
        "prefs-lang-english",
        "prefs-lang-italian",
    ]
    .into_iter()
    .map(crate::l10n::tr)
    .collect()
}

fn terminal_labels(available: &[String]) -> Vec<String> {
    std::iter::once(crate::l10n::tr("prefs-terminal-automatic"))
        .chain(
            available
                .iter()
                .map(|program| crate::terminal::display_name(program)),
        )
        .collect()
}

fn view_index(mode: ViewMode) -> u32 {
    match mode {
        ViewMode::Icons => 0,
        ViewMode::Compact => 1,
        ViewMode::Details => 2,
    }
}

fn open_items_index(mode: OpenItems) -> u32 {
    match mode {
        OpenItems::DoubleClick => 0,
        OpenItems::SingleClick => 1,
    }
}

fn language_index(language: kito_i18n::AppLang) -> u32 {
    match language {
        kito_i18n::AppLang::System => 0,
        kito_i18n::AppLang::English => 1,
        kito_i18n::AppLang::Italian => 2,
    }
}

fn language_at(index: u32) -> kito_i18n::AppLang {
    match index {
        1 => kito_i18n::AppLang::English,
        2 => kito_i18n::AppLang::Italian,
        _ => kito_i18n::AppLang::System,
    }
}

fn terminal_index(choice: &TerminalChoice, available: &[String]) -> u32 {
    match choice {
        TerminalChoice::Automatic => 0,
        TerminalChoice::Emulator(program) => available
            .iter()
            .position(|candidate| candidate == program)
            .map(|index| index as u32 + 1)
            .unwrap_or(0),
    }
}

fn terminal_missing(program: &str) -> String {
    crate::l10n::tr_with_one(
        "prefs-terminal-missing",
        "terminal",
        &crate::terminal::display_name(program),
    )
}

fn save_error(error: &std::io::Error) -> String {
    crate::l10n::tr_with_one("prefs-save-error", "error", &error.to_string())
}

fn shortcut_label(id: &str, preferences: &crate::preferences::model::Preferences) -> String {
    crate::shortcuts::CATALOG
        .iter()
        .find(|binding| binding.id == id)
        .and_then(|binding| crate::shortcuts::effective(binding, preferences))
        .and_then(|accelerator| {
            gtk::accelerator_parse(&accelerator)
                .map(|(key, mods)| gtk::accelerator_get_label(key, mods).to_string())
        })
        .unwrap_or_else(|| crate::l10n::tr("prefs-shortcuts-disabled"))
}

fn save_status(result: std::io::Result<()>) -> String {
    match result {
        Ok(()) => String::new(),
        Err(error) => save_error(&error),
    }
}
