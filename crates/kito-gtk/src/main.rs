//! Kito Files: Adwaita window + classic view (one folder at a time).
//! Version note: gtk `gnome_50` but adw `v1_9` because Fedora 44 ships
//! system libadwaita 1.9 (v1_10 only when the runtime updates).

mod context_menu;
mod dnd;
mod file_list;
mod icons;
mod l10n;
mod ops;
mod path_completion;
mod preferences;
mod preferences_dialog;
mod shortcuts;
mod sidebar;
mod tabs;
mod terminal;

use adw::prelude::*;
use file_list::ViewMode;
use gtk::{gdk, gio, glib};
use l10n::{tr, tr_num};
use preferences::PreferenceStore;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn view_icon_names(mode: ViewMode) -> &'static [&'static str] {
    match mode {
        ViewMode::Icons => &[
            "view-grid-symbolic",
            "view-list-symbolic",
            "view-grid",
            "view-list",
        ],
        ViewMode::Compact => &[
            "view-list-symbolic",
            "view-grid-symbolic",
            "view-list",
            "view-grid",
        ],
        ViewMode::Details => &[
            "view-list-bullet-symbolic",
            "view-list-symbolic",
            "view-list-bullet",
            "view-list",
        ],
    }
}

fn view_label(mode: ViewMode) -> String {
    crate::l10n::tr(match mode {
        ViewMode::Icons => "view-icons",
        ViewMode::Compact => "view-compact",
        ViewMode::Details => "view-details",
    })
}

fn view_choice_button(mode: ViewMode) -> gtk::ToggleButton {
    let label = view_label(mode);
    let icon = gtk::Image::from_gicon(&icons::control_icon(view_icon_names(mode)));
    icon.set_pixel_size(20);
    let check = gtk::Image::from_gicon(&icons::control_icon(&[
        "object-select-symbolic",
        "emblem-ok-symbolic",
        "object-select",
        "emblem-ok",
    ]));
    check.set_pixel_size(12);
    check.set_opacity(0.0);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(5)
        .halign(gtk::Align::Center)
        .build();
    content.append(&icon);
    content.append(&check);

    let button = gtk::ToggleButton::builder()
        .hexpand(true)
        .tooltip_text(&label)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&label)]);
    button.set_child(Some(&content));
    button.connect_toggled(move |button| {
        check.set_opacity(if button.is_active() { 1.0 } else { 0.0 });
    });
    button
}

fn update_view_choice_label(button: &gtk::ToggleButton, label: &str) {
    button.set_tooltip_text(Some(label));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
}

fn update_view_control(
    mode: ViewMode,
    icons: &gtk::ToggleButton,
    compact: &gtk::ToggleButton,
    details: &gtk::ToggleButton,
    icon: &gtk::Image,
    button: &gtk::MenuButton,
    syncing: &Cell<bool>,
) {
    syncing.set(true);
    icons.set_active(mode == ViewMode::Icons);
    compact.set_active(mode == ViewMode::Compact);
    details.set_active(mode == ViewMode::Details);
    syncing.set(false);

    icon.set_from_gicon(&icons::control_icon(view_icon_names(mode)));
    let current = crate::l10n::tr_with_one("view-current", "view", &view_label(mode));
    button.set_tooltip_text(Some(&current));
    let selector = crate::l10n::tr("view-selector");
    button.update_property(&[
        gtk::accessible::Property::Label(&selector),
        gtk::accessible::Property::Description(&current),
    ]);
}

fn preference_switch_row(label_id: &str, switch: &gtk::Switch) -> gtk::Box {
    let label = tr(label_id);
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_start(8)
        .margin_end(8)
        .margin_top(5)
        .margin_bottom(5)
        .build();
    let text = gtk::Label::builder()
        .label(&label)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    switch.update_property(&[gtk::accessible::Property::Label(&label)]);
    row.append(&text);
    row.append(switch);
    row
}

/// Applies the window-controls preference to a window's start/end controls
/// with native decoration-layout APIs. Automatic mode clears the app
/// override (`None`) so GTK/libadwaita follows the system layout live,
/// including system changes at runtime. Custom mode shows only the selected
/// buttons on the right (`":minimize,maximize,close"` filtered), leaving the
/// left side empty. Appearance stays with the theme and actions with the
/// window manager; dragging, window menu, shortcuts and other header buttons
/// are untouched.
fn apply_window_controls(
    start: &gtk::WindowControls,
    end: &gtk::WindowControls,
    controls: &crate::preferences::model::WindowControls,
) {
    let layout = controls.decoration_layout();
    start.set_decoration_layout(layout.as_deref());
    end.set_decoration_layout(layout.as_deref());
}

const APP_MENU_ITEMS: [(&str, &str, &[&str]); 2] = [
    (
        "menu-preferences",
        "win.preferences",
        &[
            "preferences-system-symbolic",
            "applications-system-symbolic",
            "preferences-system",
            "preferences",
        ],
    ),
    (
        "menu-about",
        "win.about",
        &[
            "help-about-symbolic",
            "dialog-information-symbolic",
            "help-about",
            "dialog-information",
        ],
    ),
];

fn update_app_menu_popover(popover: &gtk::Popover, window: &adw::ApplicationWindow) {
    let Some(app) = window.application() else {
        return;
    };
    let actions = APP_MENU_ITEMS
        .iter()
        .map(|(_, action, _)| (*action).to_string())
        .collect::<Vec<_>>();
    let shortcut_width = shortcuts::shortcut_column_width(&app, &actions);
    let list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(6)
        .margin_end(6)
        .margin_top(6)
        .margin_bottom(6)
        .build();

    for (label_id, action, icon_names) in APP_MENU_ITEMS {
        let label = tr(label_id);
        let button = gtk::Button::builder().has_frame(false).build();
        button.add_css_class("ctx-row");
        button.update_property(&[gtk::accessible::Property::Label(&label)]);

        let content = gtk::Grid::builder()
            .column_spacing(10)
            .margin_start(8)
            .margin_end(8)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        let image = gtk::Image::from_gicon(&icons::control_icon(icon_names));
        image.set_pixel_size(18);
        content.attach(&image, 0, 0, 1, 1);
        let text = gtk::Label::builder()
            .label(&label)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        content.attach(&text, 1, 0, 1, 1);
        content.attach(
            &shortcuts::shortcut_cell(&app, Some(action), shortcut_width),
            2,
            0,
            1,
            1,
        );
        button.set_child(Some(&content));

        let weak_window = window.downgrade();
        let weak_popover = popover.downgrade();
        let action = action.to_owned();
        button.connect_clicked(move |_| {
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
            if let Some(window) = weak_window.upgrade() {
                let _ = gtk::prelude::WidgetExt::activate_action(&window, &action, None);
            }
        });
        list.append(&button);
    }

    popover.set_child(Some(&list));
}

fn build_app_menu_popover(window: &adw::ApplicationWindow) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.add_css_class("ctx-menu");
    update_app_menu_popover(&popover, window);
    let weak_popover = popover.downgrade();
    let weak_window = window.downgrade();
    popover.connect_show(move |_| {
        if let (Some(popover), Some(window)) = (weak_popover.upgrade(), weak_window.upgrade()) {
            update_app_menu_popover(&popover, &window);
        }
    });
    popover
}

/// One `win.*` action entry: name + function on the context.
type ActionDef = (&'static str, fn(&ops::Ctx));

/// Registers the `win.*` actions (menu + shortcuts) on the window.
fn register_actions(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    ctx: Rc<ops::Ctx>,
    preferences: Rc<PreferenceStore>,
    preferences_dialog: preferences_dialog::SharedDialog,
    single_item_actions: Rc<RefCell<Vec<gio::SimpleAction>>>,
) {
    let group = gio::SimpleActionGroup::new();
    let defs: [ActionDef; 43] = [
        ("open", ops::Ctx::open_selected),
        ("open-in-new-tab", ops::Ctx::open_selected_in_new_tabs),
        ("open-in-new-window", ops::Ctx::open_selected_in_new_window),
        ("new-folder", ops::Ctx::new_folder),
        ("new-text-file", |c| {
            c.new_file(format!("{}.txt", tr("suggest-text-file")))
        }),
        ("new-empty-file", |c| c.new_file(tr("suggest-empty-file"))),
        ("new-word-doc", |c| {
            c.new_file(format!("{}.docx", tr("suggest-word-doc")))
        }),
        ("new-spreadsheet", |c| {
            c.new_file(format!("{}.xlsx", tr("suggest-spreadsheet")))
        }),
        ("new-html-page", |c| {
            c.new_file(format!("{}.html", tr("suggest-html")))
        }),
        ("open-terminal", ops::Ctx::open_terminal),
        ("open-terminal-root", ops::Ctx::open_terminal_root),
        ("properties", ops::Ctx::show_properties),
        ("empty-trash", ops::Ctx::empty_trash),
        ("restore", ops::Ctx::restore_selected),
        ("pin", ops::Ctx::toggle_pin),
        ("cut", |c| c.copy_selected(true)),
        ("copy", |c| c.copy_selected(false)),
        ("paste", ops::Ctx::paste),
        ("trash", ops::Ctx::trash_selected),
        ("delete", ops::Ctx::delete_selected),
        ("rename", ops::Ctx::rename_selected),
        ("edit-path", |c| c.focus_path()),
        ("reload", |c| c.manager.reload_selected()),
        ("about", |c| show_about(&c.window)),
        ("select-all", |c| c.manager.select_all()),
        ("deselect-all", |c| c.manager.deselect_all()),
        ("invert-selection", |c| c.manager.invert_selection()),
        ("new-tab", |c| c.manager.new_tab_here()),
        ("close-tab", |c| c.manager.close_selected_tab()),
        ("reopen-tab", |c| c.manager.reopen_last_closed()),
        ("next-tab", |c| c.manager.select_next_tab()),
        ("previous-tab", |c| c.manager.select_previous_tab()),
        ("back", |c| c.manager.go_back()),
        ("forward", |c| c.manager.go_forward()),
        ("up", ops::Ctx::go_up),
        ("sort-name", |c| {
            c.manager.set_sort_field(kito_core::SortField::Name)
        }),
        ("sort-size", |c| {
            c.manager.set_sort_field(kito_core::SortField::Size)
        }),
        ("sort-type", |c| {
            c.manager.set_sort_field(kito_core::SortField::Type)
        }),
        ("sort-modified", |c| {
            c.manager.set_sort_field(kito_core::SortField::Modified)
        }),
        ("sort-direction", |c| c.manager.toggle_sort_direction()),
        ("zoom-in", ops::Ctx::zoom_in),
        ("zoom-out", ops::Ctx::zoom_out),
        ("zoom-reset", ops::Ctx::zoom_reset),
    ];
    for (name, run) in defs {
        let action = gio::SimpleAction::new(name, None);
        if matches!(name, "rename" | "properties") {
            single_item_actions.borrow_mut().push(action.clone());
        }
        let ctx = ctx.clone();
        action.connect_activate(move |_, _| run(&ctx));
        group.add_action(&action);
    }
    let action = gio::SimpleAction::new("preferences", None);
    let preferences_window = window.clone();
    action.connect_activate(move |_, _| {
        preferences_dialog::present(&preferences_window, &preferences, &preferences_dialog);
    });
    group.add_action(&action);
    window.insert_action_group("win", Some(&group));

    shortcuts::register_application_accelerators(app);
}

/// Button with ordered system-theme icon candidates. Callers put symbolic
/// names before semantic symbolic fallbacks and regular variants.
fn themed_button(names: &[&str], tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&gtk::Image::from_gicon(&icons::control_icon(names))));
    button
}

/// Navigation arrow: flat and round, like Nautilus.
fn nav_button(names: &[&str], tooltip: &str) -> gtk::Button {
    let button = themed_button(names, tooltip);
    button.add_css_class("flat");
    button.add_css_class("nav-btn");
    button
}

/// Builds the `(label, target URI)` crumbs for `uri` without widgets:
/// labels are GIO-decoded (`My Folder`, not `My%20Folder`), targets keep
/// the exact encoded URIs. Non-local schemes yield a single crumb.
fn crumb_items(uri: &str) -> Vec<(String, String)> {
    let Some(path) = uri.strip_prefix("file://") else {
        return Vec::new();
    };
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut out = Vec::with_capacity(segments.len() + 1);
    out.push(("/".to_string(), "file:///".to_string()));
    let mut prefix = String::from("file://");
    for segment in segments {
        prefix = format!("{prefix}/{segment}");
        let label = kito_core::uri_file_name(&prefix).unwrap_or_else(|| segment.to_string());
        out.push((label, prefix.clone()));
    }
    out
}

/// Premium style: pill pathbar, context menu, sidebar, status bar.
fn load_pathbar_css() {
    let css = "\
        .app-title {\
            font-weight: 700;\
            font-size: 1.05em;\
            margin-right: 6px;\
        }\
        .path-pill {\
            background-color: alpha(@window_fg_color, 0.06);\
            border-radius: 10px;\
        }\
        .path-pill:focus-within {\
            background-color: alpha(@accent_bg_color, 0.16);\
        }\
        .crumb {\
            border-radius: 7px;\
            padding: 3px 7px;\
            color: alpha(@window_fg_color, 0.85);\
        }\
        .crumb:hover {\
            background-color: alpha(@window_fg_color, 0.10);\
            color: @window_fg_color;\
        }\
        .current-crumb {\
            background-color: alpha(@accent_bg_color, 0.20);\
            border-radius: 7px;\
            padding: 3px 9px;\
            font-weight: 600;\
            color: @window_fg_color;\
        }\
        .current-crumb:hover {\
            background-color: alpha(@accent_bg_color, 0.34);\
        }\
        .crumb-sep {\
            color: alpha(@window_fg_color, 0.35);\
        }\
        .nav-btn {\
            border-radius: 999px;\
            padding: 6px;\
            transition: background-color 120ms ease-out;\
        }\
        .nav-btn:hover:not(:disabled) {\
            background-color: alpha(@window_fg_color, 0.10);\
        }\
        .ctx-menu {\
            padding: 0;\
        }\
        .ctx-sub {\
            background-color: alpha(@window_fg_color, 0.04);\
            border-top-right-radius: 9px;\
            border-bottom-right-radius: 9px;\
        }\
        .ctx-divider {\
            background-color: alpha(@window_fg_color, 0.10);\
        }\
        .ctx-row {\
            border-radius: 8px;\
            min-height: 22px;\
            padding: 0;\
        }\
        .ctx-row:hover {\
            background-color: alpha(@accent_bg_color, 0.14);\
        }\
        .ctx-row:checked:not(:hover) {\
            background-color: transparent;\
        }\
        .side-row {\
            background-color: transparent;\
            border-radius: 8px;\
            padding: 4px 8px;\
            transition: background-color 120ms ease-out;\
        }\
        .side-row:hover {\
            background-color: alpha(@window_fg_color, 0.07);\
        }\
        .side-row.active {\
            background-color: alpha(@accent_bg_color, 0.16);\
        }\
        .side-row.active:hover {\
            background-color: alpha(@accent_bg_color, 0.26);\
        }\
        .side-label {\
            color: alpha(@window_fg_color, 0.85);\
        }\
        .side-label.active {\
            color: @window_fg_color;\
        }\
        .status-bar {\
            background-color: alpha(@window_fg_color, 0.025);\
            border-top-width: 1px;\
            border-top-style: solid;\
            border-top-color: alpha(@window_fg_color, 0.08);\
        }\
        .zoom-button {\
            min-width: 24px;\
            min-height: 24px;\
            padding: 0;\
        }\
        .zoom-slider {\
            min-width: 92px;\
            min-height: 18px;\
            margin: 0 2px;\
        }";
    let provider = gtk::CssProvider::new();
    provider.load_from_string(css);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Breadcrumb button. If `current`, highlighted pill that opens editing.
fn crumb_button(
    label: &str,
    tooltip: &str,
    current: bool,
    on_click: impl Fn() + 'static,
) -> gtk::Button {
    let button = if current {
        gtk::Button::builder()
            .css_classes(["current-crumb"])
            .tooltip_text(tr("crumb-edit-current"))
            .build()
    } else {
        gtk::Button::builder()
            .css_classes(["flat", "crumb"])
            .tooltip_text(tooltip)
            .build()
    };
    button.set_label(label);
    button.connect_clicked(move |_| on_click());
    button
}

/// Faint `›` separator between segments.
fn crumb_sep() -> gtk::Label {
    gtk::Label::builder()
        .label("›")
        .css_classes(["dim-label", "crumb-sep"])
        .margin_start(2)
        .margin_end(2)
        .build()
}

/// Rebuilds the breadcrumbs for `uri`. Each segment opens its prefix,
/// except the last one (current folder, `/` included): it opens editing.
fn rebuild_crumbs(crumbs: &gtk::Box, uri: &str, load: &Rc<dyn Fn(&str)>, edit: &Rc<dyn Fn()>) {
    while let Some(child) = crumbs.first_child() {
        crumbs.remove(&child);
    }
    let Some(_path) = uri.strip_prefix("file://") else {
        // Non-file schemes (trash, network): readable name, clicking
        // opens path writing like for file:// paths.
        let label: String = if uri.starts_with("trash:") {
            tr("side-trash")
        } else if uri.starts_with("network:") {
            tr("side-browse-network")
        } else {
            kito_core::uri_to_display(uri)
        };
        let edit = edit.clone();
        crumbs.append(&crumb_button(
            &label,
            &tr("crumb-edit-path"),
            true,
            move || edit(),
        ));
        return;
    };
    // `/` alone: already the current folder, so it edits.
    let items = crumb_items(uri);
    if items.len() == 1 {
        let edit = edit.clone();
        crumbs.append(&crumb_button(
            "/",
            &tr("crumb-edit-path"),
            true,
            move || edit(),
        ));
        return;
    }
    let last = items.len() - 1;
    for (i, (label, target)) in items.iter().enumerate() {
        if i == 0 {
            let load = load.clone();
            let target = target.clone();
            crumbs.append(&crumb_button(label, &tr("crumb-root"), false, move || {
                load(&target)
            }));
            continue;
        }
        crumbs.append(&crumb_sep());
        if i == last {
            let edit = edit.clone();
            crumbs.append(&crumb_button(
                label,
                &tr("crumb-edit-path"),
                true,
                move || edit(),
            ));
        } else {
            let target = target.clone();
            let label = label.clone();
            let load = load.clone();
            crumbs.append(&crumb_button(&label, &label, false, move || load(&target)));
        }
    }
}

/// Minimal About dialog.
fn show_about(window: &adw::ApplicationWindow) {
    let dialog = adw::AboutDialog::builder()
        .application_name("Kito Files")
        .application_icon("it.kito.KitoFiles")
        .version("0.1.0")
        .comments(tr("about-comments"))
        .developer_name(tr("about-developer"))
        .license_type(gtk::License::MitX11)
        .build();
    dialog.present(Some(window));
}

fn main() -> glib::ExitCode {
    let preferences = PreferenceStore::load();
    l10n::init(kito_i18n::I18n::new(
        preferences.snapshot().language,
        kito_i18n::detect_system(),
    ));
    preferences.subscribe_language(Rc::new(l10n::set_language));
    let app = adw::Application::builder()
        .application_id("it.kito.KitoFiles")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_activate({
        let preferences = preferences.clone();
        move |app| build_window(app, Vec::new(), preferences.clone())
    });
    // `kito-files ~/Downloads`, "open folder with Kito Files": one tab per file.
    // If it is a file (not a folder), opens the folder containing it.
    app.connect_open({
        let preferences = preferences.clone();
        move |app, files, _hint| {
            let mut uris = Vec::new();
            for file in files {
                let is_dir = file
                    .query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
                    == gio::FileType::Directory;
                if is_dir {
                    uris.push(file.uri().to_string());
                } else if let Some(parent) = file.parent() {
                    uris.push(parent.uri().to_string());
                }
            }
            build_window(app, uris, preferences.clone());
        }
    });

    app.run()
}

/// Builds a window. Empty `initial_uris` = opens home.
fn build_window(
    app: &adw::Application,
    initial_uris: Vec<String>,
    preferences: Rc<PreferenceStore>,
) {
    load_pathbar_css();
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Kito Files")
        .default_width(900)
        .default_height(600)
        .build();
    let preferences_dialog: preferences_dialog::SharedDialog = Rc::new(RefCell::new(None));

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();

    // Path: clickable breadcrumbs <-> writable entry (Ctrl+L).
    // The expandable path bar must share one row with New Tab so it can end
    // immediately before it. Window controls are placed in that row too, so
    // the title slot can fill the width without centering the app controls.
    let path_stack = gtk::Stack::builder()
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .build();
    let path_bar_name = tr("path-bar-name");
    let path_bar_description = tr("path-bar-description");
    path_stack.update_property(&[
        gtk::accessible::Property::Label(&path_bar_name),
        gtk::accessible::Property::Description(&path_bar_description),
    ]);

    let crumbs_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .min_content_width(1)
        .css_classes(["path-pill"])
        .build();
    let crumbs = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(2)
        .margin_start(4)
        .margin_end(4)
        .margin_top(3)
        .margin_bottom(3)
        .build();
    crumbs_scroll.set_child(Some(&crumbs));
    path_stack.add_named(&crumbs_scroll, Some("crumbs"));

    let path_entry = gtk::Entry::builder()
        .placeholder_text(tr("path-placeholder"))
        .hexpand(true)
        .width_chars(1)
        .css_classes(["path-pill"])
        .build();
    path_stack.add_named(&path_entry, Some("edit"));
    path_stack.set_visible_child_name("crumbs");

    // Single Nautilus-style bar: left (name + menu + navigation),
    // expanding path, view and tabs on the right. No second row.
    let header = adw::HeaderBar::new();
    let header_contents = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .build();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    header.set_title_widget(Some(&header_contents));
    content.append(&header);

    let start_window_controls = gtk::WindowControls::new(gtk::PackType::Start);
    start_window_controls.set_visible(!start_window_controls.is_empty());
    start_window_controls.connect_empty_notify(|controls| {
        controls.set_visible(!controls.is_empty());
    });
    header_contents.append(&start_window_controls);

    let app_label = gtk::Label::builder()
        .label("Kito Files")
        .margin_start(4)
        .css_classes(["app-title"])
        .build();
    header_contents.append(&app_label);

    let app_menu_popover = build_app_menu_popover(&window);
    let app_menu_button = gtk::MenuButton::builder()
        .tooltip_text(tr("menu-application"))
        .popover(&app_menu_popover)
        .build();
    let app_menu_icon = gtk::Image::from_gicon(&icons::control_icon(&[
        "open-menu-symbolic",
        "view-more-symbolic",
        "open-menu",
        "view-more",
    ]));
    app_menu_icon.set_pixel_size(18);
    app_menu_button.set_child(Some(&app_menu_icon));
    let app_menu_name = tr("menu-application");
    app_menu_button.update_property(&[gtk::accessible::Property::Label(&app_menu_name)]);
    header_contents.append(&app_menu_button);

    // Navigation: separate flat round arrows, like Nautilus (no linked).
    let nav = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .margin_start(4)
        .build();
    let back_button = nav_button(
        &[
            "go-previous-symbolic",
            "pan-start-symbolic",
            "go-previous",
            "pan-start",
        ],
        &tr("nav-back"),
    );
    back_button.set_sensitive(false);
    let forward_button = nav_button(
        &["go-next-symbolic", "pan-end-symbolic", "go-next", "pan-end"],
        &tr("nav-forward"),
    );
    forward_button.set_sensitive(false);
    let up_button = nav_button(
        &["go-up-symbolic", "folder-symbolic", "go-up", "folder"],
        &tr("nav-up"),
    );
    nav.append(&back_button);
    nav.append(&forward_button);
    nav.append(&up_button);
    header_contents.append(&nav);
    header_contents.append(&path_stack);

    // The view popover contains only view modes and the hidden-file switch.
    let initial_view = preferences.snapshot().default_view;
    let icons_btn = view_choice_button(ViewMode::Icons);
    let compact_btn = view_choice_button(ViewMode::Compact);
    compact_btn.set_group(Some(&icons_btn));
    let details_btn = view_choice_button(ViewMode::Details);
    details_btn.set_group(Some(&icons_btn));
    match initial_view {
        ViewMode::Icons => icons_btn.set_active(true),
        ViewMode::Compact => compact_btn.set_active(true),
        ViewMode::Details => details_btn.set_active(true),
    }
    let view_choices = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .homogeneous(true)
        .build();
    view_choices.append(&icons_btn);
    view_choices.append(&compact_btn);
    view_choices.append(&details_btn);

    let hidden_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_start(8)
        .margin_end(8)
        .margin_top(5)
        .margin_bottom(5)
        .build();
    let hidden_label = gtk::Label::builder()
        .label(tr("view-hidden"))
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    let hidden_switch = gtk::Switch::builder()
        .valign(gtk::Align::Center)
        .active(false)
        .tooltip_text(tr("view-hidden"))
        .build();
    let hidden_accessible_name = tr("view-hidden");
    hidden_switch.update_property(&[gtk::accessible::Property::Label(&hidden_accessible_name)]);
    hidden_row.append(&hidden_label);
    hidden_row.append(&hidden_switch);

    let column_options = preferences.snapshot();
    let column_section = gtk::Label::builder()
        .label(tr("view-columns"))
        .halign(gtk::Align::Start)
        .css_classes(["heading"])
        .margin_start(8)
        .build();
    let size_column_switch = gtk::Switch::builder()
        .active(column_options.show_size_column)
        .tooltip_text(tr("column-size"))
        .valign(gtk::Align::Center)
        .build();
    let type_column_switch = gtk::Switch::builder()
        .active(column_options.show_type_column)
        .tooltip_text(tr("column-type"))
        .valign(gtk::Align::Center)
        .build();
    let modified_column_switch = gtk::Switch::builder()
        .active(column_options.show_modified_column)
        .tooltip_text(tr("column-modified"))
        .valign(gtk::Align::Center)
        .build();
    let size_column_row = preference_switch_row("column-size", &size_column_switch);
    let type_column_row = preference_switch_row("column-type", &type_column_switch);
    let modified_column_row = preference_switch_row("column-modified", &modified_column_switch);

    let view_contents = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(8)
        .margin_end(8)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    view_contents.append(&view_choices);
    view_contents.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    view_contents.append(&hidden_row);
    view_contents.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    view_contents.append(&column_section);
    view_contents.append(&size_column_row);
    view_contents.append(&type_column_row);
    view_contents.append(&modified_column_row);
    let view_popover = gtk::Popover::new();
    view_popover.set_child(Some(&view_contents));

    let view_icon = gtk::Image::from_gicon(&icons::control_icon(view_icon_names(initial_view)));
    view_icon.set_pixel_size(18);
    let view_arrow =
        gtk::Image::from_gicon(&icons::control_icon(&["pan-down-symbolic", "pan-down"]));
    view_arrow.set_pixel_size(12);
    let view_button_content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(5)
        .build();
    view_button_content.append(&view_icon);
    view_button_content.append(&view_arrow);
    let current_view = crate::l10n::tr_with_one("view-current", "view", &view_label(initial_view));
    let view_button = gtk::MenuButton::builder()
        .tooltip_text(&current_view)
        .popover(&view_popover)
        .build();
    view_button.set_child(Some(&view_button_content));
    let view_selector_name = tr("view-selector");
    view_button.update_property(&[
        gtk::accessible::Property::Label(&view_selector_name),
        gtk::accessible::Property::Description(&current_view),
    ]);
    let new_tab_button = themed_button(
        &[
            "tab-new-symbolic",
            "document-new-symbolic",
            "tab-new",
            "document-new",
        ],
        &tr("nav-new-tab"),
    );
    header_contents.append(&new_tab_button);
    header_contents.append(&view_button);
    let end_window_controls = gtk::WindowControls::new(gtk::PackType::End);
    end_window_controls.set_visible(!end_window_controls.is_empty());
    end_window_controls.connect_empty_notify(|controls| {
        controls.set_visible(!controls.is_empty());
    });
    header_contents.append(&end_window_controls);

    // Window controls follow the shared preference immediately and for new
    // windows. Weak refs: closing a window releases its widgets.
    apply_window_controls(
        &start_window_controls,
        &end_window_controls,
        &preferences.snapshot().window_controls,
    );
    preferences.subscribe_window_controls({
        let start_weak = start_window_controls.downgrade();
        let end_weak = end_window_controls.downgrade();
        Rc::new(move |controls| {
            if let (Some(start), Some(end)) = (start_weak.upgrade(), end_weak.upgrade()) {
                apply_window_controls(&start, &end, &controls);
            }
        })
    });

    // Body: sidebar on the left, tabs + status on the right.
    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();
    paned.set_shrink_start_child(true);
    paned.set_resize_start_child(false);
    content.append(&paned);

    let right = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    paned.set_end_child(Some(&right));

    let tab_view = adw::TabView::new();
    tab_view.set_vexpand(true);
    let tab_bar = adw::TabBar::builder().view(&tab_view).build();
    right.append(&tab_bar);
    right.append(&tab_view);

    // Status bar: item + selection counter on the left, compact zoom control
    // on the right.
    let status = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .hexpand(true)
        .margin_start(14)
        .margin_end(14)
        .margin_top(7)
        .margin_bottom(7)
        .css_classes(["dim-label", "caption"])
        .build();
    let zoom_initial = preferences.snapshot().icon_zoom;
    let zoom_label = gtk::Label::builder()
        .label(format!("{zoom_initial}%"))
        .width_chars(4)
        .css_classes(["numeric", "dim-label", "caption"])
        .build();
    let zoom_out_button = themed_button(&["zoom-out-symbolic", "zoom-out"], &tr("zoom-out"));
    zoom_out_button.add_css_class("flat");
    zoom_out_button.add_css_class("zoom-button");
    let zoom_in_button = themed_button(&["zoom-in-symbolic", "zoom-in"], &tr("zoom-in"));
    zoom_in_button.add_css_class("flat");
    zoom_in_button.add_css_class("zoom-button");
    let zoom_scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        crate::preferences::model::ICON_ZOOM_MIN as f64,
        crate::preferences::model::ICON_ZOOM_MAX as f64,
        crate::preferences::model::ICON_ZOOM_STEP as f64,
    );
    zoom_scale.set_draw_value(false);
    zoom_scale.set_value(zoom_initial as f64);
    zoom_scale.set_width_request(100);
    zoom_scale.add_css_class("zoom-slider");
    let zoom_selector = tr("zoom-selector");
    zoom_scale.set_tooltip_text(Some(&zoom_selector));
    zoom_scale.update_property(&[gtk::accessible::Property::Label(&zoom_selector)]);
    let zoom_controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(3)
        .valign(gtk::Align::Center)
        .margin_end(10)
        .build();
    zoom_controls.append(&zoom_out_button);
    zoom_controls.append(&zoom_scale);
    zoom_controls.append(&zoom_in_button);
    zoom_controls.append(&zoom_label);
    let syncing_zoom = Rc::new(Cell::new(false));
    let zoom_generation = Rc::new(Cell::new(0_u64));
    let synced_zoom = Rc::new(Cell::new(zoom_initial));
    let status_bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["status-bar"])
        .build();
    status_bar.append(&status);
    status_bar.append(&zoom_controls);
    right.append(&status_bar);

    // Manager slot: needed by breadcrumbs (created before the manager).
    let manager_slot: Rc<RefCell<Option<Rc<tabs::TabManager>>>> = Rc::new(RefCell::new(None));
    type PathCompletionInvalidator = Rc<RefCell<Option<Rc<dyn Fn()>>>>;
    let path_completion_invalidator: PathCompletionInvalidator = Rc::new(RefCell::new(None));
    let slot_load: Rc<dyn Fn(&str)> = Rc::new({
        let manager_slot = manager_slot.clone();
        move |uri: &str| {
            if let Some(manager) = manager_slot.borrow().as_ref() {
                manager.load_selected(uri);
            }
        }
    });

    // Ctrl+L or click on the last segment: entry with the current path.
    // The shown (text, uri) pair is stored: Enter unchanged navigates to
    // the stored URI, so Ctrl+L + Enter is always a no-op round-trip
    // (even for lossy-shown non-UTF-8 paths: never opens another folder).
    let shown_text_uri: Rc<RefCell<(String, String)>> =
        Rc::new(RefCell::new((String::new(), String::new())));
    let show_path_entry: Rc<dyn Fn()> = Rc::new({
        let path_stack = path_stack.clone();
        let path_entry = path_entry.clone();
        let manager_slot = manager_slot.clone();
        let shown_text_uri = shown_text_uri.clone();
        move || {
            if let Some(manager) = manager_slot.borrow().as_ref() {
                if let Some(uri) = manager.selected_uri() {
                    let mut text = kito_core::uri_to_display(&uri);
                    // The path bar edits a directory prefix. Keep a trailing
                    // separator so typing starts the next segment instead of
                    // appending to the current directory's name.
                    if kito_core::uri_to_path(&uri).is_some() && !text.ends_with('/') {
                        text.push('/');
                    }
                    *shown_text_uri.borrow_mut() = (text.clone(), uri);
                    path_entry.set_text(&text);
                }
            }
            path_stack.set_visible_child_name("edit");
            path_entry.grab_focus();
            // Keep the existing path as context when editing by clicking the
            // breadcrumb bar, so typing appends to the current location.
            path_entry.set_position(-1);
        }
    });
    // Ctrl+L retains the familiar full-selection behavior for quickly
    // replacing the entire path.
    let select_path_entry: Rc<dyn Fn()> = Rc::new({
        let show_path_entry = show_path_entry.clone();
        let path_entry = path_entry.clone();
        move || {
            show_path_entry();
            path_entry.select_region(0, -1);
        }
    });

    // Pathbar + title + status + view follow the selected tab.
    let syncing_views = Rc::new(Cell::new(false));
    // Rename and selected-item properties are unavailable for a group.
    let single_item_actions: Rc<RefCell<Vec<gio::SimpleAction>>> =
        Rc::new(RefCell::new(Vec::new()));
    // Sidebar created later: slot to highlight the current folder.
    let sidebar_slot: Rc<RefCell<Option<sidebar::Sidebar>>> = Rc::new(RefCell::new(None));
    // Status bar: totals + selection, text recomposed on every change.
    let set_status: tabs::OnStatus = Rc::new({
        let status = status.clone();
        let single_item_actions = single_item_actions.clone();
        move |items: usize, selected: usize| {
            let text = if selected > 0 {
                format!(
                    "{} · {}",
                    tr_num("status-items", items as u64),
                    tr_num("status-selected", selected as u64)
                )
            } else {
                tr_num("status-items", items as u64)
            };
            status.set_text(&text);
            for action in single_item_actions.borrow().iter() {
                action.set_enabled(selected <= 1);
            }
        }
    });
    let on_navigate: tabs::OnNavigate = Rc::new({
        let crumbs = crumbs.clone();
        let path_stack = path_stack.clone();
        let slot_load = slot_load.clone();
        let show_path_entry = show_path_entry.clone();
        let syncing_views = syncing_views.clone();
        let icons_btn = icons_btn.clone();
        let compact_btn = compact_btn.clone();
        let details_btn = details_btn.clone();
        let view_icon = view_icon.clone();
        let view_button = view_button.clone();
        let window = window.clone();
        let sidebar_slot = sidebar_slot.clone();
        let path_completion_invalidator = path_completion_invalidator.clone();
        let crumbs_scroll = crumbs_scroll.clone();
        let set_status = set_status.clone();
        let manager_slot = manager_slot.clone();
        move |uri: &str, n: usize, mode: ViewMode| {
            // A completed navigation (or tab switch) retires any open
            // background menu: its captured folder may no longer apply.
            if let Some(manager) = manager_slot.borrow().as_ref() {
                manager.close_bg_menu();
            }
            if let Some(invalidate) = path_completion_invalidator.borrow().as_ref() {
                invalidate();
            }
            rebuild_crumbs(&crumbs, uri, &slot_load, &show_path_entry);
            path_stack.set_visible_child_name("crumbs");
            let adjustment = crumbs_scroll.hadjustment();
            glib::idle_add_local_once(move || {
                adjustment.set_value((adjustment.upper() - adjustment.page_size()).max(0.0));
            });
            // Window title: folder name, not the raw URI.
            let name = gio::File::for_uri(uri)
                .basename()
                .map(|b| b.display().to_string())
                .unwrap_or_else(|| "Kito Files".to_string());
            window.set_title(Some(&format!("{name} — Kito Files")));
            if let Some(sidebar) = sidebar_slot.borrow().as_ref() {
                sidebar.set_active(uri);
            }
            set_status(n, 0);
            update_view_control(
                mode,
                &icons_btn,
                &compact_btn,
                &details_btn,
                &view_icon,
                &view_button,
                &syncing_views,
            );
        }
    });
    let on_history: tabs::OnHistory = Rc::new({
        let back_button = back_button.clone();
        let forward_button = forward_button.clone();
        move |can_back: bool, can_forward: bool| {
            back_button.set_sensitive(can_back);
            forward_button.set_sensitive(can_forward);
        }
    });
    let on_sort: tabs::OnSort = Rc::new(|_| {});
    // Hidden files (those starting with `.`): state shared with tabs.
    let show_hidden = Rc::new(Cell::new(false));
    let open_window: Rc<dyn Fn(Vec<String>)> = Rc::new({
        let app = app.clone();
        let preferences = preferences.clone();
        move |uris| build_window(&app, uris, preferences.clone())
    });
    let manager = tabs::TabManager::new(
        tab_view,
        window.clone(),
        on_navigate.clone(),
        on_history,
        set_status,
        on_sort,
        show_hidden.clone(),
        preferences.shared(),
        open_window,
    );
    *manager_slot.borrow_mut() = Some(manager.clone());
    preferences.subscribe_display({
        let manager = Rc::downgrade(&manager);
        let zoom_scale = zoom_scale.downgrade();
        let zoom_label = zoom_label.downgrade();
        let syncing_zoom = syncing_zoom.clone();
        let zoom_generation = zoom_generation.clone();
        let synced_zoom = synced_zoom.clone();
        let size_switch = size_column_switch.downgrade();
        let type_switch = type_column_switch.downgrade();
        let modified_switch = modified_column_switch.downgrade();
        Rc::new(move |prefs| {
            if prefs.icon_zoom != synced_zoom.get() {
                zoom_generation.set(zoom_generation.get().wrapping_add(1));
                syncing_zoom.set(true);
                if let Some(scale) = zoom_scale.upgrade() {
                    scale.set_value(prefs.icon_zoom as f64);
                }
                if let Some(label) = zoom_label.upgrade() {
                    label.set_text(&format!("{}%", prefs.icon_zoom));
                }
                syncing_zoom.set(false);
                synced_zoom.set(prefs.icon_zoom);
            }
            if let Some(switch) = size_switch.upgrade() {
                if switch.is_active() != prefs.show_size_column {
                    switch.set_active(prefs.show_size_column);
                }
            }
            if let Some(switch) = type_switch.upgrade() {
                if switch.is_active() != prefs.show_type_column {
                    switch.set_active(prefs.show_type_column);
                }
            }
            if let Some(switch) = modified_switch.upgrade() {
                if switch.is_active() != prefs.show_modified_column {
                    switch.set_active(prefs.show_modified_column);
                }
            }
            if let Some(manager) = manager.upgrade() {
                manager.refresh_display_preferences();
            }
        })
    });
    size_column_switch.connect_active_notify({
        let preferences = preferences.clone();
        move |switch| {
            if let Err(error) = preferences.set_show_size_column(switch.is_active()) {
                eprintln!("save size-column preference: {error}");
            }
        }
    });
    type_column_switch.connect_active_notify({
        let preferences = preferences.clone();
        move |switch| {
            if let Err(error) = preferences.set_show_type_column(switch.is_active()) {
                eprintln!("save type-column preference: {error}");
            }
        }
    });
    modified_column_switch.connect_active_notify({
        let preferences = preferences.clone();
        move |switch| {
            if let Err(error) = preferences.set_show_modified_column(switch.is_active()) {
                eprintln!("save modified-column preference: {error}");
            }
        }
    });
    let autocomplete = path_completion::PathAutocomplete::new(
        &path_entry,
        &path_stack,
        window.upcast_ref(),
        {
            let manager = Rc::downgrade(&manager);
            Rc::new(move || manager.upgrade().and_then(|manager| manager.selected_uri()))
        },
        {
            let show_hidden = show_hidden.clone();
            Rc::new(move || show_hidden.get())
        },
    );
    *path_completion_invalidator.borrow_mut() = Some(autocomplete.weak_invalidator());
    let path_completion_retranslator = autocomplete.weak_retranslator();
    preferences.subscribe_open_items({
        let manager = Rc::downgrade(&manager);
        Rc::new(move |behavior| {
            if let Some(manager) = manager.upgrade() {
                manager.set_open_items(behavior);
            }
        })
    });

    // View selector: a single active button, the tab follows.
    let view_buttons = [
        (icons_btn.clone(), ViewMode::Icons),
        (compact_btn.clone(), ViewMode::Compact),
        (details_btn.clone(), ViewMode::Details),
    ];
    for (button, mode) in view_buttons.clone() {
        let view_buttons = view_buttons.clone();
        let manager = manager.clone();
        let syncing_views = syncing_views.clone();
        button.connect_toggled(move |toggled| {
            if syncing_views.get() {
                return;
            }
            if toggled.is_active() {
                for (other, _) in &view_buttons {
                    if other != toggled {
                        other.set_active(false);
                    }
                }
                manager.set_mode(mode);
            } else if view_buttons.iter().all(|(other, _)| !other.is_active()) {
                toggled.set_active(true);
            }
        });
    }
    *manager_slot.borrow_mut() = Some(manager.clone());

    // Enter in the entry: unchanged text reloads the stored URI,
    // edited text resolves as URI (with "://") or local path.
    path_entry.connect_activate({
        let manager = manager.clone();
        let shown_text_uri = shown_text_uri.clone();
        let autocomplete = autocomplete.clone();
        let window = window.clone();
        move |entry| {
            let text = entry.text().to_string();
            let (shown_text, shown_uri) = shown_text_uri.borrow().clone();
            autocomplete.invalidate_and_close();
            let Some(current_uri) = manager.selected_uri() else {
                return;
            };
            match path_completion::resolve_path_input(
                &text,
                &shown_text,
                &shown_uri,
                &current_uri,
                &glib::home_dir(),
            ) {
                Some(uri) => manager.load_selected(&uri),
                None if !text.is_empty() => {
                    let dialog = adw::AlertDialog::builder()
                        .heading(tr("error-open-folder"))
                        .body(tr("error-invalid-path"))
                        .build();
                    dialog.add_response("ok", &tr("dialog-ok"));
                    dialog.present(Some(&window));
                }
                None => {}
            }
        }
    });

    // Empty space in the breadcrumb viewport enters path editing. Inspect
    // the picked child so breadcrumb and separator clicks keep their action.
    let blank_path_click = gtk::GestureClick::builder().button(1).build();
    blank_path_click.connect_pressed({
        let crumbs_scroll = crumbs_scroll.downgrade();
        let edit = show_path_entry.clone();
        move |_, _, x, y| {
            let Some(scroll) = crumbs_scroll.upgrade() else {
                return;
            };
            let mut picked = scroll.pick(x, y, gtk::PickFlags::DEFAULT);
            while let Some(widget) = picked {
                if widget.downcast_ref::<gtk::Button>().is_some()
                    || widget.downcast_ref::<gtk::Label>().is_some()
                {
                    return;
                }
                if widget == scroll {
                    break;
                }
                picked = widget.parent();
            }
            edit();
        }
    });
    crumbs_scroll.add_controller(blank_path_click);

    let toast_overlay = adw::ToastOverlay::new();
    let ctx = Rc::new(ops::Ctx {
        window: window.clone(),
        manager: manager.clone(),
        toast: toast_overlay.clone(),
        clipboard: Rc::new(RefCell::new(ops::ClipTracker::default())),
        preferences: preferences.clone(),
        focus_path: select_path_entry,
    });
    manager.setup_tab_bar(&tab_bar, &ctx);
    zoom_scale.connect_value_changed({
        let preferences = preferences.clone();
        let syncing_zoom = syncing_zoom.clone();
        let zoom_generation = zoom_generation.clone();
        let zoom_label = zoom_label.downgrade();
        move |scale| {
            if syncing_zoom.get() {
                return;
            }
            let zoom = scale.value().round().clamp(
                crate::preferences::model::ICON_ZOOM_MIN as f64,
                crate::preferences::model::ICON_ZOOM_MAX as f64,
            ) as u8;
            if let Some(label) = zoom_label.upgrade() {
                label.set_text(&format!("{zoom}%"));
            }
            let generation = zoom_generation.get().wrapping_add(1);
            zoom_generation.set(generation);
            let zoom_generation = zoom_generation.clone();
            let preferences = preferences.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                if zoom_generation.get() == generation {
                    if let Err(error) = preferences.set_icon_zoom(zoom) {
                        eprintln!("save icon zoom preference: {error}");
                    }
                }
            });
        }
    });
    zoom_out_button.connect_clicked({
        let ctx = ctx.clone();
        move |_| ctx.zoom_out()
    });
    zoom_in_button.connect_clicked({
        let ctx = ctx.clone();
        move |_| ctx.zoom_in()
    });
    register_actions(
        app,
        &window,
        ctx.clone(),
        preferences.clone(),
        preferences_dialog.clone(),
        single_item_actions,
    );

    // System clipboard decides what to paste: track ownership changes.
    // Weak reference: the display outlives the window, no cycle.
    if let Some(display) = gdk::Display::default() {
        let weak = Rc::downgrade(&ctx);
        display.clipboard().connect_changed(move |_| {
            if let Some(ctx) = weak.upgrade() {
                ctx.on_clipboard_changed();
            }
        });
    }

    hidden_switch.connect_active_notify({
        let manager = manager.clone();
        move |switch| manager.set_show_hidden(switch.is_active())
    });
    // Sidebar and arrows operate on the selected tab.
    let load: Rc<dyn Fn(&str)> = slot_load.clone();

    let sidebar_drop: dnd::DropHandler = {
        let ctx = Rc::downgrade(&ctx);
        Rc::new(move |uris, destination, action, internal, drop| {
            if let Some(ctx) = ctx.upgrade() {
                ctx.transfer_uris(uris, destination, action, internal, Some(drop));
            } else {
                drop.finish(gdk::DragAction::empty());
            }
        })
    };
    let sidebar = sidebar::build_sidebar(load.clone(), window.clone(), sidebar_drop);
    paned.set_start_child(Some(sidebar.widget()));
    paned.set_position(170);
    *sidebar_slot.borrow_mut() = Some(sidebar);

    // Preferences are shared process-wide. Each window keeps only weak
    // references here so closing a window also releases its widgets.
    preferences.subscribe_language({
        let manager = Rc::downgrade(&manager);
        let sidebar_slot = Rc::downgrade(&sidebar_slot);
        let preferences_dialog = Rc::downgrade(&preferences_dialog);
        let preferences = Rc::downgrade(&preferences);
        let back_button = back_button.downgrade();
        let forward_button = forward_button.downgrade();
        let up_button = up_button.downgrade();
        let path_entry = path_entry.downgrade();
        let path_stack = path_stack.downgrade();
        let path_completion_retranslator = path_completion_retranslator.clone();
        let app_menu_button = app_menu_button.downgrade();
        let app_menu_popover = app_menu_popover.downgrade();
        let window = window.downgrade();
        let view_button = view_button.downgrade();
        let new_tab_button = new_tab_button.downgrade();
        let icons_btn = icons_btn.downgrade();
        let compact_btn = compact_btn.downgrade();
        let details_btn = details_btn.downgrade();
        let hidden_label = hidden_label.downgrade();
        let hidden_switch = hidden_switch.downgrade();
        Rc::new(move |_| {
            if let Some(button) = back_button.upgrade() {
                button.set_tooltip_text(Some(&tr("nav-back")));
            }
            if let Some(button) = forward_button.upgrade() {
                button.set_tooltip_text(Some(&tr("nav-forward")));
            }
            if let Some(button) = up_button.upgrade() {
                button.set_tooltip_text(Some(&tr("nav-up")));
            }
            if let Some(entry) = path_entry.upgrade() {
                entry.set_placeholder_text(Some(&tr("path-placeholder")));
            }
            if let Some(stack) = path_stack.upgrade() {
                let label = tr("path-bar-name");
                let description = tr("path-bar-description");
                stack.update_property(&[
                    gtk::accessible::Property::Label(&label),
                    gtk::accessible::Property::Description(&description),
                ]);
            }
            path_completion_retranslator();
            if let Some(button) = app_menu_button.upgrade() {
                let label = tr("menu-application");
                button.set_tooltip_text(Some(&label));
                button.update_property(&[gtk::accessible::Property::Label(&label)]);
            }
            if let (Some(popover), Some(window)) = (app_menu_popover.upgrade(), window.upgrade()) {
                update_app_menu_popover(&popover, &window);
            }
            if let Some(button) = view_button.upgrade() {
                let label = tr("view-selector");
                button.update_property(&[gtk::accessible::Property::Label(&label)]);
            }
            if let Some(button) = new_tab_button.upgrade() {
                button.set_tooltip_text(Some(&tr("nav-new-tab")));
            }
            if let Some(button) = icons_btn.upgrade() {
                update_view_choice_label(&button, &tr("view-icons"));
            }
            if let Some(button) = compact_btn.upgrade() {
                update_view_choice_label(&button, &tr("view-compact"));
            }
            if let Some(button) = details_btn.upgrade() {
                update_view_choice_label(&button, &tr("view-details"));
            }
            if let Some(label) = hidden_label.upgrade() {
                label.set_text(&tr("view-hidden"));
            }
            if let Some(switch) = hidden_switch.upgrade() {
                let label = tr("view-hidden");
                switch.set_tooltip_text(Some(&label));
                switch.update_property(&[gtk::accessible::Property::Label(&label)]);
            }
            if let Some(slot) = sidebar_slot.upgrade() {
                if let Some(sidebar) = slot.borrow().as_ref() {
                    sidebar.retranslate();
                }
            }
            if let Some(state) = preferences_dialog.upgrade() {
                if let Some(preferences) = preferences.upgrade() {
                    if let Some(dialog) = state.borrow().as_ref() {
                        preferences_dialog::retranslate(dialog, &preferences);
                    }
                }
            }
            if let Some(manager) = manager.upgrade() {
                manager.retranslate();
            }
        })
    });

    back_button.connect_clicked({
        let manager = manager.clone();
        move |_| manager.go_back()
    });
    forward_button.connect_clicked({
        let manager = manager.clone();
        move |_| manager.go_forward()
    });
    // Extra mouse buttons: 8 = back, 9 = forward.
    for (button_no, go) in [
        (8u32, tabs::TabManager::go_back as fn(&tabs::TabManager)),
        (9u32, tabs::TabManager::go_forward as fn(&tabs::TabManager)),
    ] {
        let gesture = gtk::GestureClick::builder().button(button_no).build();
        let manager = manager.clone();
        gesture.connect_pressed(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            go(&manager);
        });
        content.add_controller(gesture);
    }
    up_button.connect_clicked({
        let load = load.clone();
        let manager = manager.clone();
        move |_| {
            if let Some(uri) = manager.selected_uri() {
                if let Some(parent) = gio::File::for_uri(&uri).parent() {
                    load(&parent.uri());
                }
            }
        }
    });

    new_tab_button.connect_clicked({
        let manager = manager.clone();
        let ctx = ctx.clone();
        move |_| {
            let uri = manager
                .selected_uri()
                .unwrap_or_else(|| format!("file://{}", glib::home_dir().display()));
            manager.open_tab(&uri, &ctx);
        }
    });

    toast_overlay.set_child(Some(&content));
    window.set_content(Some(&toast_overlay));

    window.present();

    if initial_uris.is_empty() {
        let home = glib::home_dir();
        manager.open_tab(&format!("file://{}", home.display()), &ctx);
    } else {
        for uri in &initial_uris {
            manager.open_tab(uri, &ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crumb_items_decode_labels_keep_uris() {
        assert_eq!(
            crumb_items("file:///tmp/My%20Folder/sub%23dir"),
            vec![
                ("/".to_string(), "file:///".to_string()),
                ("tmp".to_string(), "file:///tmp".to_string()),
                (
                    "My Folder".to_string(),
                    "file:///tmp/My%20Folder".to_string()
                ),
                (
                    "sub#dir".to_string(),
                    "file:///tmp/My%20Folder/sub%23dir".to_string()
                ),
            ]
        );
    }

    #[test]
    fn crumb_items_root_and_non_local() {
        assert_eq!(
            crumb_items("file:///"),
            vec![("/".to_string(), "file:///".to_string())]
        );
        assert!(crumb_items("trash:///").is_empty());
        assert!(crumb_items("network:///").is_empty());
    }
}
