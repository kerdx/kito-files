//! Kito Files: Adwaita window + classic view (one folder at a time).
//! Version note: gtk `gnome_50` but adw `v1_9` because Fedora 44 ships
//! system libadwaita 1.9 (v1_10 only when the runtime updates).

mod context_menu;
mod file_list;
mod ops;
mod sidebar;
mod tabs;
mod terminal;

use adw::prelude::*;
use file_list::ViewMode;
use gtk::{gdk, gio, glib};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Menu row for the view choice: icon + label + check on the right
/// visible only on the active item (Nautilus style). The toggle is a
/// `ToggleButton`: mutual exclusion is handled by `build_window`.
fn view_row(names: &[&str], label: &str) -> gtk::ToggleButton {
    let icon = gtk::Image::from_gicon(&gio::ThemedIcon::from_names(names));
    icon.set_pixel_size(18);

    let check = gtk::Image::from_gicon(&gio::ThemedIcon::from_names(&[
        "object-select-symbolic",
        "emblem-ok-symbolic",
        "emblem-default",
    ]));
    check.set_pixel_size(16);
    check.set_visible(false);

    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    row.append(&icon);
    row.append(
        &gtk::Label::builder()
            .label(label)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build(),
    );
    row.append(&check);

    let button = gtk::ToggleButton::builder().has_frame(false).build();
    button.set_child(Some(&row));
    button.add_css_class("ctx-row");
    button.connect_toggled(move |b| check.set_visible(b.is_active()));
    button
}

/// One `win.*` action entry: name + function on the context.
type ActionDef = (&'static str, fn(&ops::Ctx));

/// Registers the `win.*` actions (menu + shortcuts) on the window.
fn register_actions(app: &adw::Application, window: &adw::ApplicationWindow, ctx: Rc<ops::Ctx>) {
    let group = gio::SimpleActionGroup::new();
    let defs: [ActionDef; 22] = [
        ("open", ops::Ctx::open_selected),
        ("new-folder", ops::Ctx::new_folder),
        ("new-text-file", |c| c.new_file("New Text File.txt")),
        ("new-empty-file", |c| c.new_file("New File")),
        ("new-word-doc", |c| c.new_file("New Word Document.docx")),
        ("new-spreadsheet", |c| c.new_file("New Spreadsheet.xlsx")),
        ("new-html-page", |c| c.new_file("New HTML Page.html")),
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
            .tooltip_text("Click to edit path")
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
            "Trash".to_string()
        } else if uri.starts_with("network:") {
            "Browse network".to_string()
        } else {
            kito_core::uri_to_display(uri)
        };
        let edit = edit.clone();
        crumbs.append(&crumb_button(&label, "Edit path", true, move || edit()));
        return;
    };
    // `/` alone: already the current folder, so it edits.
    let items = crumb_items(uri);
    if items.len() == 1 {
        let edit = edit.clone();
        crumbs.append(&crumb_button("/", "Edit path", true, move || edit()));
        return;
    }
    let last = items.len() - 1;
    for (i, (label, target)) in items.iter().enumerate() {
        if i == 0 {
            let load = load.clone();
            let target = target.clone();
            crumbs.append(&crumb_button(label, "Filesystem root", false, move || {
                load(&target)
            }));
            continue;
        }
        crumbs.append(&crumb_sep());
        if i == last {
            let edit = edit.clone();
            crumbs.append(&crumb_button(label, "Edit path", true, move || edit()));
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
        .comments("A lightweight Wayland file manager in Rust + GTK4")
        .developer_name("Kito Files contributors")
        .license_type(gtk::License::MitX11)
        .build();
    dialog.present(Some(window));
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id("it.kito.KitoFiles")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_activate(|app| {
        build_window(app, Vec::new());
    });
    // `kito-files ~/Downloads`, "open folder with Kito Files": one tab per file.
    // If it is a file (not a folder), opens the folder containing it.
    app.connect_open(|app, files, _hint| {
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
        build_window(app, uris);
    });

    app.run()
}

/// Builds a window. Empty `initial_uris` = opens home.
fn build_window(app: &adw::Application, initial_uris: Vec<String>) {
    load_pathbar_css();
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Kito Files")
        .default_width(900)
        .default_height(600)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();

    // Path: clickable breadcrumbs <-> writable entry (Ctrl+L).
    // It is the header bar title: centered in the single bar and
    // stretching to the side buttons.
    let path_stack = gtk::Stack::builder()
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .build();

    let crumbs_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .margin_start(6)
        .margin_end(6)
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
        .placeholder_text("Type a path, Enter to go")
        .hexpand(true)
        .css_classes(["path-pill"])
        .build();
    path_stack.add_named(&path_entry, Some("edit"));
    path_stack.set_visible_child_name("crumbs");

    // Single Nautilus-style bar: left (name + menu + navigation),
    // path in the center, view and tabs on the right. No second row.
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&path_stack));
    content.append(&header);

    let app_label = gtk::Label::builder()
        .label("Kito Files")
        .margin_start(4)
        .css_classes(["app-title"])
        .build();
    header.pack_start(&app_label);

    // Navigation: separate flat round arrows, like Nautilus (no linked).
    let nav = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .margin_start(4)
        .build();
    let back_button = nav_button(&["go-previous", "go-previous-symbolic"], "Back");
    back_button.set_sensitive(false);
    let forward_button = nav_button(&["go-next", "go-next-symbolic"], "Forward");
    forward_button.set_sensitive(false);
    let up_button = nav_button(&["go-up", "go-up-symbolic"], "Go up");
    nav.append(&back_button);
    nav.append(&forward_button);
    nav.append(&up_button);
    header.pack_start(&nav);

    // Menu on the right: view + settings in a single button (Nautilus style).
    // The view rows are CheckButtons with mutual exclusion wired later.
    let overflow = gtk::Popover::new();
    overflow.add_css_class("ctx-menu");
    let overflow_list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(6)
        .margin_end(6)
        .margin_top(6)
        .margin_bottom(6)
        .width_request(216)
        .build();
    let icons_btn = view_row(&["view-grid", "view-grid-symbolic"], "Icons");
    let compact_btn = view_row(&["view-list", "view-list-symbolic"], "Compact");
    let details_btn = view_row(&["view-list-details", "view-list-symbolic"], "Details");
    compact_btn.set_active(true);
    overflow_list.append(&icons_btn);
    overflow_list.append(&compact_btn);
    overflow_list.append(&details_btn);
    overflow_list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let hidden_check = gtk::CheckButton::builder()
        .label("Show Hidden Files")
        .active(false)
        .build();
    hidden_check.add_css_class("ctx-row");
    overflow_list.append(&hidden_check);
    overflow_list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let about_button = gtk::Button::builder().has_frame(false).build();
    about_button.add_css_class("ctx-row");
    let about_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .margin_start(2)
        .margin_end(8)
        .build();
    let about_icon = gtk::Image::from_gicon(&gio::ThemedIcon::from_names(&[
        "help-about",
        "help-info",
        "dialog-information",
    ]));
    about_icon.set_pixel_size(18);
    about_row.append(&about_icon);
    about_row.append(
        &gtk::Label::builder()
            .label("About Kito Files")
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build(),
    );
    about_button.set_child(Some(&about_row));
    overflow_list.append(&about_button);
    overflow.set_child(Some(&overflow_list));
    let menu_button = gtk::MenuButton::builder()
        .tooltip_text("View and settings")
        .popover(&overflow)
        .build();
    menu_button.set_child(Some(&gtk::Image::from_gicon(&gio::ThemedIcon::from_names(
        &["view-more", "view-more-symbolic"],
    ))));
    // First pack_end = rightmost: menu next to the window controls.
    header.pack_end(&menu_button);

    let new_tab_button = themed_button(&["tab-new", "tab-new-symbolic"], "New tab");
    header.pack_end(&new_tab_button);

    // TEMP-VERIFY: opens the menu for the screenshot.
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
                    let text = kito_core::uri_to_display(&uri);
                    *shown_text_uri.borrow_mut() = (text.clone(), uri);
                    path_entry.set_text(&text);
                }
            }
            path_stack.set_visible_child_name("edit");
            path_entry.grab_focus();
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
                format!("{items} items · {selected} selected")
            } else if items == 1 {
                "1 item".to_string()
            } else {
                format!("{items} items")
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
        let window = window.clone();
        let sidebar_slot = sidebar_slot.clone();
        let set_status = set_status.clone();
        move |uri: &str, n: usize, mode: ViewMode| {
            rebuild_crumbs(&crumbs, uri, &slot_load, &show_path_entry);
            path_stack.set_visible_child_name("crumbs");
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
            syncing_views.set(true);
            icons_btn.set_active(mode == ViewMode::Icons);
            compact_btn.set_active(mode == ViewMode::Compact);
            details_btn.set_active(mode == ViewMode::Details);
            syncing_views.set(false);
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
    );
    *manager_slot.borrow_mut() = Some(manager.clone());

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
        move |entry| {
            let text = entry.text().to_string();
            let (shown_text, shown_uri) = shown_text_uri.borrow().clone();
            let Some(uri) = kito_core::resolve_path_text(&text, &shown_text, &shown_uri) else {
                return;
            };
            manager.load_selected(&uri);
        }
    });
    // Esc in the entry: back to breadcrumbs.
    let esc_key = gtk::EventControllerKey::new();
    esc_key.connect_key_pressed({
        let path_stack = path_stack.clone();
        move |_, keyval, _, _| {
            if keyval == gdk::Key::Escape {
                path_stack.set_visible_child_name("crumbs");
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    path_entry.add_controller(esc_key);

    let toast_overlay = adw::ToastOverlay::new();
    let ctx = Rc::new(ops::Ctx {
        window: window.clone(),
        manager: manager.clone(),
        toast: toast_overlay.clone(),
        clipboard: Rc::new(RefCell::new(ops::ClipTracker::default())),
        focus_path: show_path_entry.clone(),
    });
    register_actions(app, &window, ctx.clone());

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

    // Menu ⋮: hidden check + about.
    hidden_check.connect_toggled({
        let manager = manager.clone();
        move |check| manager.set_show_hidden(check.is_active())
    });
    about_button.connect_clicked({
        let window = window.clone();
        let overflow = overflow.clone();
        move |_| {
            overflow.popdown();
            show_about(&window);
        }
    });

    // Sidebar and arrows operate on the selected tab.
    let load: Rc<dyn Fn(&str)> = slot_load.clone();

    let sidebar = sidebar::build_sidebar(load.clone(), window.clone());
    paned.set_start_child(Some(sidebar.widget()));
    paned.set_position(170);
    *sidebar_slot.borrow_mut() = Some(sidebar);

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
