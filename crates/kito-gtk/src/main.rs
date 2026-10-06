//! Kito Files: Adwaita window + classic view (one folder at a time).
//! Version note: gtk `gnome_50` but adw `v1_9` because Fedora 44 ships
//! system libadwaita 1.9 (v1_10 only when the runtime updates).

mod context_menu;
mod file_list;
mod l10n;
mod ops;
mod path_completion;
mod preferences;
mod preferences_dialog;
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

fn view_icon_name(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Icons => "view-grid-symbolic",
        ViewMode::Compact => "view-list-symbolic",
        ViewMode::Details => "view-list-bullet-symbolic",
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
    let icon = gtk::Image::from_icon_name(view_icon_name(mode));
    icon.set_pixel_size(20);
    let check = gtk::Image::from_icon_name("object-select-symbolic");
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

    icon.set_icon_name(Some(view_icon_name(mode)));
    let current = crate::l10n::tr_with_one("view-current", "view", &view_label(mode));
    button.set_tooltip_text(Some(&current));
    let selector = crate::l10n::tr("view-selector");
    button.update_property(&[
        gtk::accessible::Property::Label(&selector),
        gtk::accessible::Property::Description(&current),
    ]);
}

fn app_menu_model() -> gio::Menu {
    let menu = gio::Menu::new();
    update_app_menu_model(&menu);
    menu
}

fn update_app_menu_model(menu: &gio::Menu) {
    menu.remove_all();
    menu.append(Some(&tr("menu-preferences")), Some("win.preferences"));
    menu.append(Some(&tr("menu-about")), Some("win.about"));
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
) {
    let group = gio::SimpleActionGroup::new();
    let defs: [ActionDef; 22] = [
        ("open", ops::Ctx::open_selected),
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
    ];
    for (name, run) in defs {
        let action = gio::SimpleAction::new(name, None);
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

    app.set_accels_for_action("win.copy", &["<Control>c"]);
    app.set_accels_for_action("win.cut", &["<Control>x"]);
    app.set_accels_for_action("win.paste", &["<Control>v"]);
    app.set_accels_for_action("win.trash", &["Delete"]);
    app.set_accels_for_action("win.delete", &["<Shift>Delete"]);
    app.set_accels_for_action("win.rename", &["F2"]);
    app.set_accels_for_action("win.edit-path", &["<Control>l"]);
    app.set_accels_for_action("win.reload", &["F5", "<Control>r"]);
    app.set_accels_for_action("win.new-folder", &["<Control><Shift>n"]);
    app.set_accels_for_action("win.preferences", &["<Control>comma"]);
}

/// Button with a system-theme icon: base name first (full theme style,
/// e.g. Papirus/Breeze), `-symbolic` variant as fallback.
fn themed_button(names: &[&str], tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&gtk::Image::from_gicon(&gio::ThemedIcon::from_names(
        names,
    ))));
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

    let app_menu_model = app_menu_model();
    let app_menu_popover = gtk::PopoverMenu::from_model(Some(&app_menu_model));
    let app_menu_button = gtk::MenuButton::builder()
        .tooltip_text(tr("menu-application"))
        .popover(&app_menu_popover)
        .build();
    let app_menu_icon = gtk::Image::from_icon_name("open-menu-symbolic");
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
    let back_button = nav_button(&["go-previous", "go-previous-symbolic"], &tr("nav-back"));
    back_button.set_sensitive(false);
    let forward_button = nav_button(&["go-next", "go-next-symbolic"], &tr("nav-forward"));
    forward_button.set_sensitive(false);
    let up_button = nav_button(&["go-up", "go-up-symbolic"], &tr("nav-up"));
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
    let view_popover = gtk::Popover::new();
    view_popover.set_child(Some(&view_contents));

    let view_icon = gtk::Image::from_icon_name(view_icon_name(initial_view));
    view_icon.set_pixel_size(18);
    let view_arrow = gtk::Image::from_icon_name("pan-down-symbolic");
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
    let new_tab_button = themed_button(&["tab-new", "tab-new-symbolic"], &tr("nav-new-tab"));
    header_contents.append(&new_tab_button);
    header_contents.append(&view_button);
    let end_window_controls = gtk::WindowControls::new(gtk::PackType::End);
    end_window_controls.set_visible(!end_window_controls.is_empty());
    end_window_controls.connect_empty_notify(|controls| {
        controls.set_visible(!controls.is_empty());
    });
    header_contents.append(&end_window_controls);

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

    // Status bar: item + selection counter, thin row.
    let status = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .margin_start(14)
        .margin_end(14)
        .margin_top(7)
        .margin_bottom(7)
        .css_classes(["dim-label", "caption"])
        .build();
    let status_bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["status-bar"])
        .build();
    status_bar.append(&status);
    right.append(&status_bar);

    // Manager slot: needed by breadcrumbs (created before the manager).
    let manager_slot: Rc<RefCell<Option<Rc<tabs::TabManager>>>> = Rc::new(RefCell::new(None));
    let path_completion_invalidator: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));
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
    // Sidebar created later: slot to highlight the current folder.
    let sidebar_slot: Rc<RefCell<Option<sidebar::Sidebar>>> = Rc::new(RefCell::new(None));
    // Status bar: totals + selection, text recomposed on every change.
    let set_status: tabs::OnStatus = Rc::new({
        let status = status.clone();
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
        move |uri: &str, n: usize, mode: ViewMode| {
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
    // Hidden files (those starting with `.`): state shared with tabs.
    let show_hidden = Rc::new(Cell::new(false));
    let manager = tabs::TabManager::new(
        tab_view,
        window.clone(),
        on_navigate.clone(),
        on_history,
        set_status,
        show_hidden.clone(),
        preferences.shared(),
    );
    *manager_slot.borrow_mut() = Some(manager.clone());
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
    register_actions(
        app,
        &window,
        ctx.clone(),
        preferences.clone(),
        preferences_dialog.clone(),
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

    let sidebar = sidebar::build_sidebar(load.clone(), window.clone());
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
        let app_menu_model = app_menu_model.downgrade();
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
            if let Some(menu) = app_menu_model.upgrade() {
                update_app_menu_model(&menu);
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
        move |_| {
            let uri = manager
                .selected_uri()
                .unwrap_or_else(|| format!("file://{}", glib::home_dir().display()));
            manager.open_tab(&uri);
        }
    });

    toast_overlay.set_child(Some(&content));
    window.set_content(Some(&toast_overlay));

    window.present();

    if initial_uris.is_empty() {
        let home = glib::home_dir();
        manager.open_tab(&format!("file://{}", home.display()));
    } else {
        for uri in &initial_uris {
            manager.open_tab(uri);
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
