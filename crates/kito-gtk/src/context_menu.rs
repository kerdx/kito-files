//! Context menus.
//!
//! File and background menus use compact custom popovers with icon + label
//! rows. New File uses a second icon-row popover attached to a GTK menu button,
//! so dismissing it leaves the main menu open. No timers or pointer grabs are
//! used.

use crate::{ops, tabs::TabManager};
use adw::prelude::*;
use gtk::{gdk, gio};
use std::rc::Rc;

/// One menu row: icons, message id for the label, `win.*` action
/// and destructive flag.
struct Row {
    icons: &'static [&'static str],
    label: &'static str,
    action: &'static str,
    danger: bool,
}

const fn row(icons: &'static [&'static str], label: &'static str, action: &'static str) -> Row {
    Row {
        icons,
        label,
        action,
        danger: false,
    }
}

const fn danger_row(
    icons: &'static [&'static str],
    label: &'static str,
    action: &'static str,
) -> Row {
    Row {
        icons,
        label,
        action,
        danger: true,
    }
}

/// Menu on a selected entry (row or cell).
const ROWS: [Row; 11] = [
    row(&["document-open"], "menu-open", "open"),
    row(&["tab-new"], "menu-open-new-tab", "open-in-new-tab"),
    row(
        &["window-new"],
        "menu-open-new-window",
        "open-in-new-window",
    ),
    row(&["bookmark-new", "list-add"], "menu-pin", "pin"),
    row(&["edit-cut"], "menu-cut", "cut"),
    row(&["edit-copy"], "menu-copy", "copy"),
    row(&["edit-paste"], "menu-paste", "paste"),
    row(
        &["document-edit", "document-edit-symbolic"],
        "menu-rename",
        "rename",
    ),
    row(&["user-trash"], "menu-trash", "trash"),
    danger_row(&["edit-delete"], "menu-delete", "delete"),
    row(
        &["dialog-information", "help-about"],
        "menu-properties",
        "properties",
    ),
];

/// Groups: open, clipboard/pin, single-item actions, properties.
const SEPARATORS_AFTER: [usize; 3] = [2, 6, 9];

/// Menu on an entry inside the trash: restore and delete.
const TRASH_ROWS: [Row; 5] = [
    row(&["document-revert", "edit-undo"], "menu-restore", "restore"),
    row(&["edit-cut"], "menu-cut", "cut"),
    row(&["edit-copy"], "menu-copy", "copy"),
    danger_row(&["edit-delete"], "menu-delete", "delete"),
    row(
        &["dialog-information", "help-about"],
        "menu-properties",
        "properties",
    ),
];

/// Groups: [0..1, 1..3, 3..4, 4..5].
const TRASH_SEPARATORS_AFTER: [usize; 3] = [0, 2, 3];

/// Vertical container for a menu's rows, naturally sized to fit both
/// translations without a fixed width.
fn menu_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(6)
        .margin_end(6)
        .margin_top(6)
        .margin_bottom(6)
        .build()
}

/// Menu row: flat button with icon and label.
fn row_button(r: &Row) -> gtk::Button {
    let button = gtk::Button::builder().has_frame(false).build();
    button.add_css_class("ctx-row");
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .margin_start(8)
        .margin_end(8)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    let image = gtk::Image::from_gicon(&gio::ThemedIcon::from_names(r.icons));
    image.set_pixel_size(18);
    content.append(&image);
    let text = gtk::Label::builder()
        .label(crate::l10n::tr(r.label))
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    if r.danger {
        text.add_css_class("error");
    }
    content.append(&text);
    button.set_child(Some(&content));
    button
}

/// Builds the popover with its rows, without opening it.
fn build(
    anchor: &gtk::Widget,
    rows: &[Row],
    separators_after: &[usize],
    window: &adw::ApplicationWindow,
    selected_count: usize,
    folders_only: bool,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.add_css_class("ctx-menu");
    let list = menu_box();
    for (i, r) in rows.iter().enumerate() {
        let button = row_button(r);
        if matches!(r.action, "rename" | "properties") && selected_count != 1 {
            button.set_sensitive(false);
        }
        if matches!(r.action, "open-in-new-tab" | "open-in-new-window") && !folders_only {
            button.set_sensitive(false);
        }
        let window = window.clone();
        let action = r.action.to_string();
        let popover = popover.downgrade();
        button.connect_clicked(move |_| {
            if let Some(popover) = popover.upgrade() {
                popover.popdown();
            }
            let _ =
                gtk::prelude::WidgetExt::activate_action(&window, &format!("win.{action}"), None);
        });
        list.append(&button);
        if separators_after.contains(&i) {
            list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
    }
    popover.set_child(Some(&list));
    popover.set_parent(anchor);
    popover
}

/// Builds and opens the menu anchored at `(x, y)` on `anchor`.
/// The popover unparents itself on close, so repeated openings never
/// accumulate widgets.
struct PopupOptions<'a> {
    window: &'a adw::ApplicationWindow,
    rows: &'a [Row],
    separators_after: &'a [usize],
    selected_count: usize,
    folders_only: bool,
}

fn popup(anchor: &gtk::Widget, x: f64, y: f64, options: PopupOptions<'_>) {
    let popover = build(
        anchor,
        options.rows,
        options.separators_after,
        options.window,
        options.selected_count,
        options.folders_only,
    );
    popover.connect_closed(|popover| popover.unparent());
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.set_position(side_for(anchor, x));
    popover.popup();
}

/// Right of the click if the screen has room, otherwise left.
fn side_for(anchor: &gtk::Widget, x: f64) -> gtk::PositionType {
    /// Width of the main menu.
    const NEEDED: f32 = 240.0;
    let Some(root) = anchor.root() else {
        return gtk::PositionType::Right;
    };
    let x_root = anchor
        .compute_bounds(&root)
        .map(|rect| rect.x() + x as f32)
        .unwrap_or(x as f32);
    let free = root.width() as f32 - x_root;
    if free < NEEDED && x_root > free {
        gtk::PositionType::Left
    } else {
        gtk::PositionType::Right
    }
}

/// Right-click on an entry: full menu, the entry is already selected.
pub fn show(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    window: &adw::ApplicationWindow,
    selected_count: usize,
    folders_only: bool,
) {
    popup(
        anchor,
        x,
        y,
        PopupOptions {
            window,
            rows: &ROWS,
            separators_after: &SEPARATORS_AFTER,
            selected_count,
            folders_only,
        },
    );
}

/// Right-click on a trash entry: restore, copy, delete.
pub fn show_trash(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    window: &adw::ApplicationWindow,
    selected_count: usize,
) {
    popup(
        anchor,
        x,
        y,
        PopupOptions {
            window,
            rows: &TRASH_ROWS,
            separators_after: &TRASH_SEPARATORS_AFTER,
            selected_count,
            folders_only: false,
        },
    );
}

// ---------------------------------------------------------------------------
// Background menu: compact custom popover with icon + label rows, matching
// file menus. New File has its own icon-row popover. Actions live in a
// transient `bg` group bound to the captured folder; shared `win.*` actions
// keep their availability.
// ---------------------------------------------------------------------------

/// `true` for destinations the menu treats as optimistically writable
/// before the async access check answers. No I/O, never blocks.
pub(crate) fn writable_initial(dest: &str) -> bool {
    dest.starts_with("file://")
}

/// Paste availability from synchronously known state: valid internal
/// entries (never stale: external changes clear them) or system clipboard
/// formats suggesting file content. The paste operation still validates
/// and reports unsupported content; this only drives the menu item.
pub(crate) fn paste_available(has_internal: bool, offered: &[&str]) -> bool {
    has_internal
        || offered
            .iter()
            .any(|mime| *mime == "text/uri-list" || *mime == "text/plain")
}

/// Synchronously known clipboard state for the background menu: internal
/// entries plus a peek at the offered system formats. No content is read,
/// nothing blocks.
fn clipboard_hint(ctx: &ops::Ctx) -> bool {
    if ctx.clipboard.borrow().has_usable_entries() {
        return true;
    }
    let offered: Vec<String> = gdk::Display::default()
        .map(|display| {
            display
                .clipboard()
                .formats()
                .mime_types()
                .iter()
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    let offered_refs: Vec<&str> = offered.iter().map(String::as_str).collect();
    paste_available(false, &offered_refs)
}

/// One background row: symbolic icons, label key, full detailed action
/// (`bg.*`, or `win.*` for the trash menu) and an optional submenu.
pub(crate) struct BgRow {
    icons: &'static [&'static str],
    label: &'static str,
    action: &'static str,
    sub: Option<&'static [BgRow]>,
}

/// New-file entries: placeholders only, hence no Word/Spreadsheet rows.
const BG_NEW_FILE_ROWS: [BgRow; 3] = [
    BgRow {
        icons: &["document-new"],
        label: "menu-new-empty-file",
        action: "bg.new-empty-file",
        sub: None,
    },
    BgRow {
        icons: &["text-x-generic"],
        label: "menu-new-text-file",
        action: "bg.new-text-file",
        sub: None,
    },
    BgRow {
        icons: &["text-html"],
        label: "menu-new-html",
        action: "bg.new-html-page",
        sub: None,
    },
];

const BG_CREATE_ROWS: [BgRow; 2] = [
    BgRow {
        icons: &["folder-new", "folder-new-symbolic"],
        label: "bg-new-folder",
        action: "bg.new-folder",
        sub: None,
    },
    BgRow {
        icons: &["document-new", "document-new-symbolic"],
        label: "bg-new-file",
        action: "bg.can-create-file",
        sub: Some(&BG_NEW_FILE_ROWS),
    },
];

const BG_PASTE_ROWS: [BgRow; 1] = [BgRow {
    icons: &["edit-paste"],
    label: "menu-paste",
    action: "bg.paste",
    sub: None,
}];

const BG_SORT_ROWS: [BgRow; 5] = [
    BgRow {
        icons: &[
            "format-text-direction-ltr-symbolic",
            "format-text-direction-ltr",
        ],
        label: "sort-menu-name",
        action: "win.sort-name",
        sub: None,
    },
    BgRow {
        icons: &["view-sort-descending-symbolic"],
        label: "sort-menu-size",
        action: "win.sort-size",
        sub: None,
    },
    BgRow {
        icons: &["application-x-executable", "application-x-generic"],
        label: "sort-menu-type",
        action: "win.sort-type",
        sub: None,
    },
    BgRow {
        icons: &["document-properties", "document-edit"],
        label: "sort-menu-modified",
        action: "win.sort-modified",
        sub: None,
    },
    BgRow {
        icons: &["view-sort-descending-symbolic"],
        label: "sort-menu-toggle-direction",
        action: "win.sort-direction",
        sub: None,
    },
];

const BG_SORT_SECTION: [BgRow; 1] = [BgRow {
    icons: &["view-sort-ascending-symbolic"],
    label: "sort-selector",
    action: "",
    sub: Some(&BG_SORT_ROWS),
}];

const BG_TERM_ROWS: [BgRow; 2] = [
    BgRow {
        icons: &["utilities-terminal", "terminal"],
        label: "menu-open-terminal",
        action: "bg.open-terminal",
        sub: None,
    },
    BgRow {
        icons: &["utilities-terminal", "terminal"],
        label: "menu-open-terminal-root",
        action: "bg.open-terminal-root",
        sub: None,
    },
];

const BG_PROPS_ROWS: [BgRow; 1] = [BgRow {
    icons: &["dialog-information", "help-about"],
    label: "bg-folder-properties",
    action: "bg.folder-properties",
    sub: None,
}];

const BG_SELECTION_ROWS: [BgRow; 3] = [
    BgRow {
        icons: &["edit-select-all"],
        label: "menu-select-all",
        action: "win.select-all",
        sub: None,
    },
    BgRow {
        icons: &["edit-select-all"],
        label: "menu-invert-selection",
        action: "win.invert-selection",
        sub: None,
    },
    BgRow {
        icons: &["edit-clear"],
        label: "menu-deselect-all",
        action: "win.deselect-all",
        sub: None,
    },
];

const BG_TRASH_ROWS: [BgRow; 1] = [BgRow {
    icons: &["user-trash-full", "user-trash"],
    label: "menu-empty-trash",
    action: "win.empty-trash",
    sub: None,
}];

/// Sections in order; separators fall between sections only, never
/// trailing. The terminal section exists only for local paths.
pub(crate) fn bg_sections(local: bool) -> Vec<&'static [BgRow]> {
    let mut sections: Vec<&'static [BgRow]> = vec![
        &BG_CREATE_ROWS[..],
        &BG_PASTE_ROWS[..],
        &BG_SORT_SECTION[..],
        &BG_SELECTION_ROWS[..],
    ];
    if local {
        sections.push(&BG_TERM_ROWS[..]);
    }
    sections.push(&BG_PROPS_ROWS[..]);
    sections
}

/// Registers one `bg.*` action enabled as stated, running `run` with the
/// captured folder. Actions die with the popover; shared `win.*` actions
/// keep their own availability for shortcuts and file menus.
fn bg_action(
    group: &gio::SimpleActionGroup,
    name: &'static str,
    enabled: bool,
    ctx: &Rc<ops::Ctx>,
    dest: &str,
    manager: &Rc<TabManager>,
    run: fn(&ops::Ctx, &str),
) -> gio::SimpleAction {
    let action = gio::SimpleAction::new(name, None);
    action.set_enabled(enabled);
    let ctx = Rc::downgrade(ctx);
    let dest = dest.to_string();
    let manager = Rc::downgrade(manager);
    action.connect_activate(move |_, _| {
        if let Some(manager) = manager.upgrade() {
            manager.close_bg_menu();
        }
        if let Some(ctx) = ctx.upgrade() {
            run(&ctx, &dest);
        }
    });
    group.add_action(&action);
    action
}

/// Decides the writability refinement: `None` keeps the opening
/// assumption (backend error, closed menu, or stale folder after a tab
/// switch/navigation). Pure over ids so staleness stays testable.
pub(crate) fn writability_update(
    menu_dest: &str,
    current_dest: Option<&str>,
    answer: Result<bool, ()>,
) -> Option<bool> {
    if current_dest != Some(menu_dest) {
        return None;
    }
    answer.ok()
}

fn action_available(writable: bool, other_requirement: bool) -> bool {
    writable && other_requirement
}

/// Refines creation/paste availability once `access::can-write` answers,
/// without ever blocking the UI for it. Unknown backends keep the opening
/// assumption; errors at operation time are still reported by the actions.
/// Stale answers (tab switched or navigated meanwhile) are ignored.
fn refine_writability(
    dest: &str,
    manager: &Rc<TabManager>,
    popover: &gtk::Popover,
    actions: &[(gio::SimpleAction, bool)],
) {
    use gio::prelude::FileExt as _;
    let file = gio::File::for_uri(dest);
    let dest = dest.to_string();
    let manager = Rc::downgrade(manager);
    let popover = popover.downgrade();
    let actions = actions.to_vec();
    file.query_info_async(
        "access::can-write",
        gio::FileQueryInfoFlags::NONE,
        glib::Priority::DEFAULT,
        gio::Cancellable::NONE,
        move |result| {
            let answer = result
                .map(|info| info.boolean("access::can-write"))
                .map_err(|_| ());
            let Some(popover) = popover.upgrade() else {
                return;
            };
            if !popover.is_visible() {
                return;
            }
            let current = manager.upgrade().and_then(|manager| manager.selected_uri());
            let Some(writable) = writability_update(&dest, current.as_deref(), answer) else {
                return;
            };
            for (action, other_requirement) in &actions {
                action.set_enabled(action_available(writable, *other_requirement));
            }
        },
    );
}

/// Detects terminals off the GTK thread. Until the cheap filesystem probe
/// completes, terminal rows remain disabled; operation-time launch errors
/// are still reported by the action itself.
fn refine_terminal_actions(
    dest: &str,
    manager: &Rc<TabManager>,
    popover: &gtk::Popover,
    choice: crate::preferences::model::TerminalChoice,
    actions: &[(gio::SimpleAction, bool)],
) {
    let (tx, rx) = async_channel::bounded::<Vec<String>>(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(crate::terminal::available_terminal_programs());
    });
    let dest = dest.to_string();
    let manager = Rc::downgrade(manager);
    let popover = popover.downgrade();
    let actions = actions.to_vec();
    glib::spawn_future_local(async move {
        let Ok(available) = rx.recv().await else {
            return;
        };
        let (Some(manager), Some(popover)) = (manager.upgrade(), popover.upgrade()) else {
            return;
        };
        if !popover.is_visible() || manager.selected_uri().as_deref() != Some(&dest) {
            return;
        }
        for (action, root) in actions {
            action.set_enabled(crate::terminal::can_open(root, &choice, &available));
        }
    });
}

/// Vertical placement for the native menu: below the click when its natural
/// height fits there, above it otherwise.
fn vertical_side(anchor: &gtk::Widget, y: f64, natural_h: i32) -> gtk::PositionType {
    let fits_below = if natural_h > 0 {
        anchor.height() as f64 - y >= natural_h as f64
    } else {
        y <= anchor.height() as f64 / 2.0
    };
    if fits_below {
        gtk::PositionType::Bottom
    } else {
        gtk::PositionType::Top
    }
}

fn bg_row_content(row: &BgRow, submenu: bool) -> gtk::Box {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .margin_start(8)
        .margin_end(8)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    let image = gtk::Image::from_gicon(&gio::ThemedIcon::from_names(row.icons));
    image.set_pixel_size(18);
    content.append(&image);
    let text = gtk::Label::builder()
        .label(crate::l10n::tr(row.label))
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    content.append(&text);
    if submenu {
        let arrow = gtk::Image::from_icon_name("pan-end-symbolic");
        arrow.add_css_class("dim-label");
        content.append(&arrow);
    }
    content
}

fn append_bg_widget(
    list: &gtk::Box,
    row: &BgRow,
    group: &gio::SimpleActionGroup,
    create_file_action: &gio::SimpleAction,
    manager: &Rc<TabManager>,
    window: &adw::ApplicationWindow,
) {
    if let Some(children) = row.sub {
        let button = gtk::MenuButton::new();
        button.set_has_frame(false);
        button.set_hexpand(true);
        button.add_css_class("ctx-row");
        button.set_child(Some(&bg_row_content(row, true)));
        if row.action == "bg.can-create-file" {
            button.set_sensitive(create_file_action.is_enabled());
            let weak_button = button.downgrade();
            create_file_action.connect_notify_local(Some("enabled"), move |action, _| {
                if let Some(button) = weak_button.upgrade() {
                    button.set_sensitive(action.is_enabled());
                }
            });
        }

        let submenu = gtk::Popover::new();
        submenu.insert_action_group("bg", Some(group));
        submenu.set_has_arrow(false);
        submenu.add_css_class("ctx-menu");
        submenu.set_position(gtk::PositionType::Right);
        let submenu_list = menu_box();
        for child in children {
            let child_button = gtk::Button::builder().has_frame(false).build();
            child_button.add_css_class("ctx-row");
            child_button.set_hexpand(true);
            child_button.set_child(Some(&bg_row_content(child, false)));
            let local_action = child
                .action
                .strip_prefix("bg.")
                .and_then(|name| group.lookup_action(name))
                .and_downcast::<gio::SimpleAction>();
            if let Some(action) = local_action {
                child_button.set_sensitive(action.is_enabled());
                let weak_button = child_button.downgrade();
                action.connect_notify_local(Some("enabled"), move |action, _| {
                    if let Some(button) = weak_button.upgrade() {
                        button.set_sensitive(action.is_enabled());
                    }
                });
            } else {
                assert!(
                    child.action.starts_with("win."),
                    "submenu action must be registered in bg or win"
                );
            }
            let weak_submenu = submenu.downgrade();
            let group = group.clone();
            let manager = Rc::downgrade(manager);
            let window = window.downgrade();
            let action_name = child.action.to_string();
            child_button.connect_clicked(move |_| {
                if let Some(submenu) = weak_submenu.upgrade() {
                    submenu.popdown();
                }
                if let Some(action_name) = action_name.strip_prefix("bg.") {
                    group.activate_action(action_name, None);
                } else {
                    if let Some(manager) = manager.upgrade() {
                        manager.close_bg_menu();
                    }
                    if let Some(window) = window.upgrade() {
                        let _ =
                            gtk::prelude::WidgetExt::activate_action(&window, &action_name, None);
                    }
                }
            });
            submenu_list.append(&child_button);
        }
        submenu.set_child(Some(&submenu_list));
        button.set_popover(Some(&submenu));
        list.append(&button);
    } else {
        let button = gtk::Button::builder().has_frame(false).build();
        button.add_css_class("ctx-row");
        button.set_hexpand(true);
        button.set_child(Some(&bg_row_content(row, false)));
        if row.action.starts_with("bg.") {
            button.set_action_name(Some(row.action));
        } else {
            let action = row.action.to_string();
            let manager = Rc::downgrade(manager);
            let window = window.downgrade();
            button.connect_clicked(move |_| {
                if let Some(manager) = manager.upgrade() {
                    manager.close_bg_menu();
                }
                if let Some(window) = window.upgrade() {
                    let _ = gtk::prelude::WidgetExt::activate_action(&window, &action, None);
                }
            });
        }
        list.append(&button);
    }
}

/// Builds the same compact, icon-row layout as the file menu. The main
/// popover contains no scrolled window. The child popover has separate icon
/// rows and can be dismissed independently with Escape or an outside click.
fn popup_bg(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    sections: &[&[BgRow]],
    group: &gio::SimpleActionGroup,
    manager: &Rc<TabManager>,
    window: &adw::ApplicationWindow,
) -> gtk::Popover {
    let menu = gtk::Popover::new();
    menu.set_has_arrow(false);
    menu.add_css_class("ctx-menu");
    menu.insert_action_group("bg", Some(group));
    let list = menu_box();
    let create_file_action = group
        .lookup_action("can-create-file")
        .and_downcast::<gio::SimpleAction>()
        .expect("background group has can-create-file action");
    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
        for row in *section {
            append_bg_widget(&list, row, group, &create_file_action, manager, window);
        }
    }
    menu.set_child(Some(&list));
    menu.set_parent(anchor);
    menu.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    let (_, natural_h, _, _) = menu.measure(gtk::Orientation::Vertical, -1);
    menu.set_position(vertical_side(anchor, y, natural_h));
    let popover: gtk::Popover = menu.clone().upcast();
    manager.track_bg_menu(&popover);
    let tracked = popover.downgrade();
    let manager_weak = Rc::downgrade(manager);
    menu.connect_closed(move |menu| {
        menu.unparent();
        if let (Some(manager), Some(tracked)) = (manager_weak.upgrade(), tracked.upgrade()) {
            manager.forget_bg_menu(&tracked);
        }
    });
    menu.popup();
    popover
}

/// Right-click on the background: compact icon-row menu for the tab folder
/// captured at open time. Returns the popover and its `bg` action group.
pub fn show_background_for(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    ctx: &Rc<ops::Ctx>,
    dest: &str,
    manager: &Rc<TabManager>,
) -> (gtk::Popover, gio::SimpleActionGroup) {
    let local = kito_core::uri_to_path(dest).is_some();
    let group = gio::SimpleActionGroup::new();
    let writable = writable_initial(dest);
    let paste = clipboard_hint(ctx);
    let can_create_file = gio::SimpleAction::new("can-create-file", None);
    can_create_file.set_enabled(writable);
    group.add_action(&can_create_file);
    let new_folder = bg_action(
        &group,
        "new-folder",
        writable,
        ctx,
        dest,
        manager,
        ops::Ctx::new_folder_at,
    );
    let new_empty = bg_action(
        &group,
        "new-empty-file",
        writable,
        ctx,
        dest,
        manager,
        |ctx, dest| ctx.new_file_at(dest, crate::l10n::tr("suggest-empty-file")),
    );
    let new_text = bg_action(
        &group,
        "new-text-file",
        writable,
        ctx,
        dest,
        manager,
        |ctx, dest| {
            ctx.new_file_at(
                dest,
                format!("{}.txt", crate::l10n::tr("suggest-text-file")),
            )
        },
    );
    let new_html = bg_action(
        &group,
        "new-html-page",
        writable,
        ctx,
        dest,
        manager,
        |ctx, dest| ctx.new_file_at(dest, format!("{}.html", crate::l10n::tr("suggest-html"))),
    );
    let paste_action = bg_action(
        &group,
        "paste",
        writable && paste,
        ctx,
        dest,
        manager,
        |ctx, dest| ctx.paste_at(dest),
    );
    let mut terminal_actions = Vec::new();
    if local {
        terminal_actions.push((
            bg_action(
                &group,
                "open-terminal",
                false,
                ctx,
                dest,
                manager,
                |ctx, dest| ctx.open_terminal_at(dest, false),
            ),
            false,
        ));
        terminal_actions.push((
            bg_action(
                &group,
                "open-terminal-root",
                false,
                ctx,
                dest,
                manager,
                |ctx, dest| ctx.open_terminal_at(dest, true),
            ),
            true,
        ));
    }
    bg_action(
        &group,
        "folder-properties",
        true,
        ctx,
        dest,
        manager,
        ops::Ctx::show_folder_properties,
    );
    let sections = bg_sections(local);
    let popover = popup_bg(anchor, x, y, &sections, &group, manager, &ctx.window);
    refine_writability(
        dest,
        manager,
        &popover,
        &[
            (new_folder, true),
            (new_empty, true),
            (new_text, true),
            (new_html, true),
            (paste_action, paste),
            (can_create_file, true),
        ],
    );
    if local {
        refine_terminal_actions(
            dest,
            manager,
            &popover,
            ctx.preferences.snapshot().terminal,
            &terminal_actions,
        );
    }
    (popover, group)
}

/// Right-click on the trash background: the dedicated empty action with
/// its destructive confirmation.
pub fn show_trash_background_for(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    ctx: &Rc<ops::Ctx>,
    manager: &Rc<TabManager>,
) {
    let sections = [&BG_TRASH_ROWS[..]];
    let group = gio::SimpleActionGroup::new();
    group.add_action(&gio::SimpleAction::new("can-create-file", None));
    popup_bg(anchor, x, y, &sections, &group, manager, &ctx.window);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Row actions per section, in order. A submenu parent may carry an
    /// action used only to control its sensitivity.
    fn section_actions(section: &[BgRow]) -> Vec<&str> {
        section.iter().map(|row| row.action).collect()
    }

    #[test]
    fn background_structure_matches_the_spec() {
        let sections = bg_sections(true);
        // Sections: create, paste, sort, selection, terminal, properties.
        assert_eq!(
            sections
                .iter()
                .map(|section| section.len())
                .collect::<Vec<_>>(),
            vec![2, 1, 1, 3, 2, 1]
        );
        assert_eq!(
            section_actions(sections[0]),
            vec!["bg.new-folder", "bg.can-create-file"]
        );
        assert_eq!(section_actions(sections[1]), vec!["bg.paste"]);
        assert_eq!(
            section_actions(sections[3]),
            vec!["win.select-all", "win.invert-selection", "win.deselect-all"]
        );
        assert_eq!(
            section_actions(sections[4]),
            vec!["bg.open-terminal", "bg.open-terminal-root"]
        );
        // Last section is folder properties: separators only sit between.
        assert_eq!(section_actions(sections[5]), vec!["bg.folder-properties"]);
        assert_eq!(
            sections[2][0]
                .sub
                .expect("sort submenu")
                .iter()
                .map(|row| row.action)
                .collect::<Vec<_>>(),
            vec![
                "win.sort-name",
                "win.sort-size",
                "win.sort-type",
                "win.sort-modified",
                "win.sort-direction"
            ]
        );
    }

    #[test]
    fn background_hides_terminal_off_local_paths() {
        let sections = bg_sections(false);
        // Remote: create, paste, sort, selection, properties.
        assert_eq!(sections.len(), 5);
        assert_eq!(
            sections[4].iter().map(|row| row.action).collect::<Vec<_>>(),
            vec!["bg.folder-properties"]
        );
    }

    #[test]
    fn background_submenu_holds_only_valid_creations() {
        let sections = bg_sections(true);
        let parent = &sections[0][1];
        // Second create entry is the New File submenu (no action of its own).
        let sub = parent.sub.expect("new-file submenu");
        assert!(!parent.icons.is_empty());
        assert_eq!(
            sub.iter().map(|row| row.action).collect::<Vec<_>>(),
            vec!["bg.new-empty-file", "bg.new-text-file", "bg.new-html-page"]
        );
        assert_eq!(
            sub.iter().map(|row| row.icons[0]).collect::<Vec<_>>(),
            vec!["document-new", "text-x-generic", "text-html"]
        );
        // No Word/Spreadsheet placeholders anywhere, submenu included.
        for section in bg_sections(true) {
            for row in section.iter() {
                assert!(!row.action.contains("word"));
                assert!(!row.action.contains("spreadsheet"));
                if let Some(sub) = row.sub {
                    for entry in sub {
                        assert!(!entry.action.contains("word"));
                        assert!(!entry.action.contains("spreadsheet"));
                    }
                }
            }
        }
    }

    #[test]
    fn paste_requires_both_a_writable_folder_and_clipboard_content() {
        assert!(action_available(true, true));
        assert!(!action_available(false, true));
        assert!(!action_available(true, false));
        assert!(!action_available(false, false));
    }

    #[test]
    fn trash_background_holds_only_empty_trash() {
        assert_eq!(BG_TRASH_ROWS.len(), 1);
        let row = &BG_TRASH_ROWS[0];
        assert_eq!(row.action, "win.empty-trash");
        assert!(row.sub.is_none());
    }

    #[test]
    fn paste_availability_never_uses_stale_internals() {
        let uri_list = ["text/uri-list"];
        assert!(paste_available(true, &uri_list));
        assert!(paste_available(false, &uri_list));
        assert!(paste_available(false, &["text/plain"]));
        assert!(!paste_available(false, &[]));
        assert!(!paste_available(false, &["image/png"]));
        // Internal entries alone suffice, whatever the system offers.
        assert!(paste_available(true, &[]));
    }

    #[test]
    fn writability_starts_optimistic_on_local_paths_only() {
        assert!(writable_initial("file:///tmp"));
        assert!(writable_initial("file:///home/user/My%20Folder"));
        assert!(!writable_initial("trash:///"));
        assert!(!writable_initial("network:///"));
        assert!(!writable_initial("smb://server/share"));
    }

    #[test]
    fn writability_refinement_ignores_stale_and_failed_answers() {
        // Backend error: keep the opening assumption.
        assert_eq!(
            writability_update("file:///a", Some("file:///a"), Err(())),
            None
        );
        // Tab switched or navigated meanwhile: ignore, even on success.
        assert_eq!(
            writability_update("file:///a", Some("file:///b"), Ok(true)),
            None
        );
        assert_eq!(writability_update("file:///a", None, Ok(true)), None);
        // Fresh answer for the open menu applies.
        assert_eq!(
            writability_update("file:///a", Some("file:///a"), Ok(true)),
            Some(true)
        );
        assert_eq!(
            writability_update("file:///a", Some("file:///a"), Ok(false)),
            Some(false)
        );
    }
}
