//! Context menus.
//!
//! File menus stay a compact custom popover with icon + label rows.
//! The background (and trash background) menus are native instead:
//! a `gio::Menu` model in a `gtk::PopoverMenu` with `has_arrow(false)`,
//! so the toolkit owns placement, hover between item and submenu,
//! keyboard navigation, dismissal and accessibility. No timers, no grab
//! handling, no `autohide` tricks.

use crate::{ops, tabs::TabManager};
use adw::prelude::*;
use gtk::{gdk, gio};
use std::{cell::RefCell, rc::Rc};

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
const ROWS: [Row; 9] = [
    row(&["document-open"], "menu-open", "open"),
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

/// Groups: [0..2, 2..5, 5..8, 8..9].
const SEPARATORS_AFTER: [usize; 3] = [1, 4, 7];

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

/// Vertical container for a menu's rows.
fn menu_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(6)
        .margin_end(6)
        .margin_top(6)
        .margin_bottom(6)
        .width_request(216)
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
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("ctx-menu");
    let list = menu_box();
    for (i, r) in rows.iter().enumerate() {
        let button = row_button(r);
        let window = window.clone();
        let action = r.action.to_string();
        let popover = popover.clone();
        button.connect_clicked(move |_| {
            popover.popdown();
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
fn popup(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    window: &adw::ApplicationWindow,
    rows: &[Row],
    separators_after: &[usize],
) {
    let popover = build(anchor, rows, separators_after, window);
    popover.connect_closed({
        let popover = popover.clone();
        move |_| popover.unparent()
    });
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
pub fn show(anchor: &gtk::Widget, x: f64, y: f64, window: &adw::ApplicationWindow) {
    popup(anchor, x, y, window, &ROWS, &SEPARATORS_AFTER);
}

/// Right-click on a trash entry: restore, copy, delete.
pub fn show_trash(anchor: &gtk::Widget, x: f64, y: f64, window: &adw::ApplicationWindow) {
    popup(anchor, x, y, window, &TRASH_ROWS, &TRASH_SEPARATORS_AFTER);
}

// ---------------------------------------------------------------------------
// Background menu: compact custom popover with icon + label rows.
//
// Native `GtkPopoverMenu` model buttons deliberately hide their icon when
// they carry text (see `update_visibility` in gtkmodelbutton.c), so rows
// with an icon on every entry are plain buttons instead — the same rows
// the file menu uses. The "New File" submenu is a second popover beside
// the first one. No timers: it opens on hover or click and closes when
// another row is hovered, when an action runs, or through the toolkit's
// own outside-click/Escape dismissal. Closing only the submenu never
// closes the main menu, except through that native dismissal. No
// `autohide` juggling. Actions live in a transient `bg` group bound to
// the captured folder, so shared `win.*` actions keep their availability.
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
    danger: bool,
    sub: Option<&'static [BgRow]>,
}

/// New-file entries: placeholders only, hence no Word/Spreadsheet rows.
const BG_NEW_FILE_ROWS: [BgRow; 3] = [
    BgRow {
        icons: &["document-new"],
        label: "menu-new-empty-file",
        action: "bg.new-empty-file",
        danger: false,
        sub: None,
    },
    BgRow {
        icons: &["text-x-generic"],
        label: "menu-new-text-file",
        action: "bg.new-text-file",
        danger: false,
        sub: None,
    },
    BgRow {
        icons: &["text-html"],
        label: "menu-new-html",
        action: "bg.new-html-page",
        danger: false,
        sub: None,
    },
];

const BG_CREATE_ROWS: [BgRow; 2] = [
    BgRow {
        icons: &["folder-new", "folder-new-symbolic"],
        label: "bg-new-folder",
        action: "bg.new-folder",
        danger: false,
        sub: None,
    },
    BgRow {
        icons: &["document-new", "document-new-symbolic"],
        label: "bg-new-file",
        action: "",
        danger: false,
        sub: Some(&BG_NEW_FILE_ROWS),
    },
];

const BG_PASTE_ROWS: [BgRow; 1] = [BgRow {
    icons: &["edit-paste"],
    label: "menu-paste",
    action: "bg.paste",
    danger: false,
    sub: None,
}];

const BG_TERM_ROWS: [BgRow; 2] = [
    BgRow {
        icons: &["utilities-terminal", "terminal"],
        label: "menu-open-terminal",
        action: "bg.open-terminal",
        danger: false,
        sub: None,
    },
    BgRow {
        icons: &["utilities-terminal", "terminal"],
        label: "menu-open-terminal-root",
        action: "bg.open-terminal-root",
        danger: false,
        sub: None,
    },
];

const BG_PROPS_ROWS: [BgRow; 1] = [BgRow {
    icons: &["dialog-information", "help-about"],
    label: "bg-folder-properties",
    action: "bg.folder-properties",
    danger: false,
    sub: None,
}];

const BG_TRASH_ROWS: [BgRow; 1] = [BgRow {
    icons: &["user-trash-full", "user-trash"],
    label: "menu-empty-trash",
    action: "win.empty-trash",
    danger: true,
    sub: None,
}];

/// Sections in order; separators fall between sections only, never
/// trailing. The terminal section exists only for local paths.
pub(crate) fn bg_sections(local: bool) -> Vec<&'static [BgRow]> {
    let mut sections: Vec<&'static [BgRow]> = vec![&BG_CREATE_ROWS[..], &BG_PASTE_ROWS[..]];
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
    ctx: &ops::Ctx,
    dest: &str,
    run: fn(&ops::Ctx, &str),
) -> gio::SimpleAction {
    let action = gio::SimpleAction::new(name, None);
    action.set_enabled(enabled);
    let ctx = ctx.clone();
    let dest = dest.to_string();
    action.connect_activate(move |_, _| run(&ctx, &dest));
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

/// Refines creation/paste availability once `access::can-write` answers,
/// without ever blocking the UI for it. Unknown backends keep the opening
/// assumption; errors at operation time are still reported by the actions.
/// Stale answers (tab switched or navigated meanwhile) are ignored.
fn refine_writability(
    dest: &str,
    manager: &Rc<TabManager>,
    popover: &gtk::Popover,
    actions: &[gio::SimpleAction],
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
            for action in &actions {
                action.set_enabled(writable);
            }
        },
    );
}

/// Vertical placement for the native menu: below the click when the
/// menu's natural height fits there, above it otherwise, so GTK rarely
/// needs to shrink it (shrinking is what draws the scrollbar). The window
/// area is the only geometry known on Wayland; the compositor still keeps
/// the menu inside the work area. The classic row popovers keep their own
/// left/right logic below.
fn vertical_side(anchor: &gtk::Widget, y: f64, natural_h: i32) -> gtk::PositionType {
    // Without a measurement fall back to the roomier half.
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

/// Compact menu container, naturally sized to its rows so both
/// translations fit without a fixed width.
fn bg_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(6)
        .margin_end(6)
        .margin_top(6)
        .margin_bottom(6)
        .build()
}

/// Icon + label row; a submenu adds the lateral `›` indicator.
fn bg_row_button(row: &BgRow) -> gtk::Button {
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
    let image = gtk::Image::from_gicon(&gio::ThemedIcon::from_names(row.icons));
    image.set_pixel_size(18);
    content.append(&image);
    let text = gtk::Label::builder()
        .label(crate::l10n::tr(row.label))
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    if row.danger {
        text.add_css_class("error");
    }
    content.append(&text);
    if row.sub.is_some() {
        content.append(
            &gtk::Label::builder()
                .label("›")
                .css_classes(["dim-label"])
                .build(),
        );
    }
    button.set_child(Some(&content));
    button
}

/// Short `bg` action name behind a detailed one, if any.
fn bg_short(detailed: &str) -> Option<&str> {
    detailed.strip_prefix("bg.")
}

/// Follows the action's enabled state on the button, live (the
/// writability check updates it after opening).
fn follow_enabled(button: &gtk::Button, group: &gio::SimpleActionGroup, short: &str) {
    let Some(action) = group
        .lookup_action(short)
        .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
    else {
        button.set_sensitive(false);
        return;
    };
    button.set_sensitive(action.is_enabled());
    let weak = button.downgrade();
    action.connect_notify_local(Some("enabled"), move |action, _| {
        if let Some(button) = weak.upgrade() {
            button.set_sensitive(action.is_enabled());
        }
    });
}

/// Closes the open submenu, if any. Takes it out before popping down so a
/// reentrant `closed` emission cannot borrow twice.
fn close_bg_sub(open: &Rc<RefCell<Option<gtk::Popover>>>) {
    let sub = open.borrow_mut().take();
    if let Some(sub) = sub {
        sub.popdown();
    }
}

/// Builds and opens the background popover for ready-made sections.
/// Separators fall between sections only. Rows activate their detailed
/// action on the popover (`bg.*`) or the window (`win.*`); sensitivity
/// follows the action live. The popover unparents itself on close;
/// repeated openings accumulate nothing.
#[allow(clippy::too_many_arguments)]
fn popup_bg(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    sections: &[&[BgRow]],
    group: &gio::SimpleActionGroup,
    window: &adw::ApplicationWindow,
    manager: &Rc<TabManager>,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.add_css_class("ctx-menu");
    popover.insert_action_group("bg", Some(group));
    let list = bg_box();
    let open_sub: Rc<RefCell<Option<gtk::Popover>>> = Rc::new(RefCell::new(None));
    for (index, section) in sections.iter().enumerate() {
        for row in section.iter() {
            let button = bg_row_button(row);
            if let Some(sub_rows) = row.sub {
                wire_submenu_parent(&button, sub_rows, &popover, group, window, &open_sub);
            } else {
                wire_bg_button(&button, row, group, window, &popover, &open_sub);
                // Hovering any plain row closes a stray submenu.
                let open_sub = open_sub.clone();
                let motion = gtk::EventControllerMotion::new();
                motion.connect_enter(move |_, _, _| close_bg_sub(&open_sub));
                button.add_controller(motion);
            }
            list.append(&button);
        }
        if index + 1 < sections.len() {
            list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
    }
    popover.set_child(Some(&list));
    popover.set_parent(anchor);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    let (_, natural_h, _, _) = popover.measure(gtk::Orientation::Vertical, -1);
    popover.set_position(vertical_side(anchor, y, natural_h));
    manager.track_bg_menu(&popover);
    let tracked = popover.clone();
    let manager_weak = Rc::downgrade(manager);
    let open_sub_close = open_sub.clone();
    popover.connect_closed(move |popover| {
        close_bg_sub(&open_sub_close);
        popover.unparent();
        if let Some(manager) = manager_weak.upgrade() {
            manager.forget_bg_menu(&tracked);
        }
    });
    popover.popup();
    popover
}

/// Sensitivity of a submenu parent: any entry enabled, kept current.
fn watch_submenu_parent(
    button: &gtk::Button,
    sub_rows: &'static [BgRow],
    group: &gio::SimpleActionGroup,
) {
    let actions: Vec<gio::SimpleAction> = sub_rows
        .iter()
        .filter_map(|entry| bg_short(entry.action))
        .filter_map(|short| {
            group
                .lookup_action(short)
                .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
        })
        .collect();
    let update = Rc::new({
        let button = button.clone();
        let actions = actions.clone();
        move || {
            button.set_sensitive(actions.iter().any(|action| action.is_enabled()));
        }
    });
    update();
    for action in actions {
        let update = update.clone();
        action.connect_notify_local(Some("enabled"), move |_, _| update());
    }
}

/// Opens the submenu beside its parent row, on the roomier side. At most
/// one is ever open; reopening the same row is a no-op.
fn open_bg_sub(
    parent_row: &gtk::Button,
    main: &gtk::Popover,
    rows: &'static [BgRow],
    group: &gio::SimpleActionGroup,
    window: &adw::ApplicationWindow,
    open_sub: &Rc<RefCell<Option<gtk::Popover>>>,
) {
    if open_sub.borrow().is_some() {
        return;
    }
    let sub = gtk::Popover::new();
    sub.set_has_arrow(false);
    sub.add_css_class("ctx-menu");
    let list = bg_box();
    for row in rows {
        let button = bg_row_button(row);
        wire_bg_button(&button, row, group, window, main, open_sub);
        list.append(&button);
    }
    sub.set_child(Some(&list));
    sub.set_parent(parent_row.upcast_ref::<gtk::Widget>());
    let side = side_for(
        parent_row.upcast_ref::<gtk::Widget>(),
        parent_row.width() as f64,
    );
    sub.set_position(side);
    let edge = if matches!(side, gtk::PositionType::Left) {
        0
    } else {
        parent_row.width()
    };
    sub.set_pointing_to(Some(&gdk::Rectangle::new(edge, 0, 1, 1)));
    sub.set_margin_start(6);
    sub.set_margin_end(6);
    sub.set_margin_top(6);
    sub.set_margin_bottom(6);
    *open_sub.borrow_mut() = Some(sub.clone());
    sub.popup();
}

/// Moves keyboard focus to the first row of the open submenu, so a menu
/// opened from the keyboard stays operable without the pointer.
fn focus_first_sub_row(open_sub: &Rc<RefCell<Option<gtk::Popover>>>) {
    let first = open_sub.borrow().as_ref().and_then(|sub| {
        sub.child()
            .and_downcast::<gtk::Box>()
            .and_then(|list| list.first_child())
            .and_downcast::<gtk::Button>()
    });
    if let Some(button) = first {
        button.grab_focus();
    }
}

/// Wires one row: sensitivity follows its action, activation pops both
/// menus down and runs the detailed action.
fn wire_bg_button(
    button: &gtk::Button,
    row: &BgRow,
    group: &gio::SimpleActionGroup,
    window: &adw::ApplicationWindow,
    main: &gtk::Popover,
    open_sub: &Rc<RefCell<Option<gtk::Popover>>>,
) {
    // Rows without their own action only open the submenu; their
    // sensitivity is the OR of their entries, kept current below.
    if row.sub.is_none() {
        if let Some(short) = bg_short(row.action) {
            follow_enabled(button, group, short);
        }
        let main = main.clone();
        let open_sub = open_sub.clone();
        let window = window.clone();
        let detailed = row.action.to_string();
        button.connect_clicked(move |_| {
            main.popdown();
            close_bg_sub(&open_sub);
            let _ = if detailed.starts_with("bg.") {
                gtk::prelude::WidgetExt::activate_action(&main, &detailed, None)
            } else {
                gtk::prelude::WidgetExt::activate_action(&window, &detailed, None)
            };
        });
    }
}

/// Wires a submenu parent row: hover or activation opens the submenu
/// (activation also moves keyboard focus into it); sensitivity is the OR
/// of its entries.
fn wire_submenu_parent(
    button: &gtk::Button,
    sub_rows: &'static [BgRow],
    main: &gtk::Popover,
    group: &gio::SimpleActionGroup,
    window: &adw::ApplicationWindow,
    open_sub: &Rc<RefCell<Option<gtk::Popover>>>,
) {
    watch_submenu_parent(button, sub_rows, group);
    let motion = gtk::EventControllerMotion::new();
    {
        let open_sub = open_sub.clone();
        let main = main.clone();
        let group = group.clone();
        let window = window.clone();
        let button_weak = button.downgrade();
        motion.connect_enter(move |_, _, _| {
            if let Some(button) = button_weak.upgrade() {
                open_bg_sub(&button, &main, sub_rows, &group, &window, &open_sub);
            }
        });
    }
    button.add_controller(motion);
    {
        let open_sub = open_sub.clone();
        let main = main.clone();
        let group = group.clone();
        let window = window.clone();
        let button_weak = button.downgrade();
        button.connect_clicked(move |_| {
            if let Some(button) = button_weak.upgrade() {
                open_bg_sub(&button, &main, sub_rows, &group, &window, &open_sub);
                focus_first_sub_row(&open_sub);
            }
        });
    }
}

/// Right-click on the background: native menu for the tab folder captured
/// at open time. Returns the open popover and its `bg` action group.
pub fn show_background_for(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    ctx: &ops::Ctx,
    dest: &str,
    manager: &Rc<TabManager>,
) -> (gtk::Popover, gio::SimpleActionGroup) {
    let local = kito_core::uri_to_path(dest).is_some();
    let group = gio::SimpleActionGroup::new();
    let writable = writable_initial(dest);
    let paste = clipboard_hint(ctx);
    let new_folder = bg_action(
        &group,
        "new-folder",
        writable,
        ctx,
        dest,
        ops::Ctx::new_folder_at,
    );
    let new_empty = bg_action(
        &group,
        "new-empty-file",
        writable,
        ctx,
        dest,
        |ctx, dest| ctx.new_file_at(dest, crate::l10n::tr("suggest-empty-file")),
    );
    let new_text = bg_action(&group, "new-text-file", writable, ctx, dest, |ctx, dest| {
        ctx.new_file_at(
            dest,
            format!("{}.txt", crate::l10n::tr("suggest-text-file")),
        )
    });
    let new_html = bg_action(&group, "new-html-page", writable, ctx, dest, |ctx, dest| {
        ctx.new_file_at(dest, format!("{}.html", crate::l10n::tr("suggest-html")))
    });
    let paste_action = bg_action(
        &group,
        "paste",
        writable && paste,
        ctx,
        dest,
        |ctx, dest| ctx.paste_at(dest),
    );
    if local {
        let available = crate::terminal::available_terminal_programs();
        let choice = ctx.preferences.snapshot().terminal;
        bg_action(
            &group,
            "open-terminal",
            crate::terminal::can_open(false, &choice, &available),
            ctx,
            dest,
            |ctx, dest| ctx.open_terminal_at(dest, false),
        );
        bg_action(
            &group,
            "open-terminal-root",
            crate::terminal::can_open(true, &choice, &available),
            ctx,
            dest,
            |ctx, dest| ctx.open_terminal_at(dest, true),
        );
    }
    bg_action(
        &group,
        "folder-properties",
        true,
        ctx,
        dest,
        ops::Ctx::show_folder_properties,
    );
    let sections = bg_sections(local);
    let popover = popup_bg(anchor, x, y, &sections, &group, &ctx.window, manager);
    refine_writability(
        dest,
        manager,
        &popover,
        &[new_folder, new_empty, new_text, new_html, paste_action],
    );
    (popover, group)
}

/// Right-click on the trash background: the dedicated empty action with
/// its destructive confirmation.
pub fn show_trash_background_for(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    ctx: &ops::Ctx,
    manager: &Rc<TabManager>,
) {
    let sections = [&BG_TRASH_ROWS[..]];
    popup_bg(
        anchor,
        x,
        y,
        &sections,
        &gio::SimpleActionGroup::new(),
        &ctx.window,
        manager,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Row actions per section, in order. The submenu parent carries no
    /// action of its own.
    fn section_actions(section: &[BgRow]) -> Vec<&str> {
        section.iter().map(|row| row.action).collect()
    }

    #[test]
    fn background_structure_matches_the_spec() {
        let sections = bg_sections(true);
        // Four sections: create, paste, terminal, properties.
        assert_eq!(
            sections
                .iter()
                .map(|section| section.len())
                .collect::<Vec<_>>(),
            vec![2, 1, 2, 1]
        );
        assert_eq!(section_actions(sections[0]), vec!["bg.new-folder", ""]);
        assert_eq!(section_actions(sections[1]), vec!["bg.paste"]);
        assert_eq!(
            section_actions(sections[2]),
            vec!["bg.open-terminal", "bg.open-terminal-root"]
        );
        // Last section is folder properties: separators only sit between.
        assert_eq!(section_actions(sections[3]), vec!["bg.folder-properties"]);
    }

    #[test]
    fn background_hides_terminal_off_local_paths() {
        let sections = bg_sections(false);
        // Remote: create, paste, properties only.
        assert_eq!(sections.len(), 3);
        assert_eq!(
            sections[2].iter().map(|row| row.action).collect::<Vec<_>>(),
            vec!["bg.folder-properties"]
        );
    }

    #[test]
    fn background_submenu_holds_only_valid_creations() {
        let sections = bg_sections(true);
        let parent = &sections[0][1];
        // Second create entry is the New File submenu (no action of its own).
        let sub = parent.sub.expect("new-file submenu");
        assert_eq!(
            sub.iter().map(|row| row.action).collect::<Vec<_>>(),
            vec!["bg.new-empty-file", "bg.new-text-file", "bg.new-html-page"]
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
    fn trash_background_holds_only_empty_trash() {
        assert_eq!(BG_TRASH_ROWS.len(), 1);
        let row = &BG_TRASH_ROWS[0];
        assert_eq!(row.action, "win.empty-trash");
        assert!(row.danger);
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

    /// The custom background menu: icon+label rows in spec order with the
    /// submenu indicator, separators only between sections, working
    /// submenu open, and no leftover popovers after closing. Needs a
    /// display; skipped headless. Label values are not asserted: parallel
    /// localization tests share the global catalog.
    #[test]
    fn bg_menu_opens_with_icons_and_a_working_submenu() {
        let _ = gtk::init();
        let _display = gdk::Display::default()
            .or_else(|| gdk::Display::open(std::env::var("WAYLAND_DISPLAY").ok().as_deref()));
        let Some(_) = gdk::Display::default() else {
            return;
        };
        /// Direct row buttons of a menu box, in order.
        fn row_buttons(list: &gtk::Box) -> Vec<gtk::Button> {
            let mut out = Vec::new();
            let mut child = list.first_child();
            while let Some(widget) = child {
                if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
                    out.push(button);
                }
                child = widget.next_sibling();
            }
            out
        }
        /// (main label text, has icon, has submenu indicator) per row.
        fn row_shape(button: &gtk::Button) -> (String, bool, bool) {
            let content = button
                .child()
                .and_downcast::<gtk::Box>()
                .expect("row content");
            let mut text = String::new();
            let mut icon = false;
            let mut sub = false;
            let mut child = content.first_child();
            while let Some(widget) = child {
                if widget.clone().downcast::<gtk::Image>().is_ok() {
                    icon = true;
                }
                if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
                    let label_text = label.text().to_string();
                    if label_text == "›" {
                        sub = true;
                    } else if text.is_empty() {
                        text = label_text;
                    }
                }
                child = widget.next_sibling();
            }
            (text, icon, sub)
        }
        fn popovers_under(widget: &gtk::Widget, out: &mut Vec<gtk::Popover>) {
            if let Ok(popover) = widget.clone().downcast::<gtk::Popover>() {
                out.push(popover);
            }
            let children = widget.observe_children();
            for i in 0..children.n_items() {
                if let Some(child) = children.item(i).and_downcast::<gtk::Widget>() {
                    popovers_under(&child, out);
                }
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let dest = format!("file://{}", tmp.path().display());
        // No application loop: the custom menu builds synchronously, so the
        // whole open/close cycle is exercised without contending for the
        // default main context with other tests.
        let app = adw::Application::builder()
            .application_id("it.kito.BgMenuTest")
            .build();
        let window: adw::ApplicationWindow =
            adw::ApplicationWindow::builder().application(&app).build();
        let tab_view = adw::TabView::new();
        let store = crate::preferences::PreferenceStore::load();
        let manager = crate::tabs::TabManager::new(
            tab_view,
            window.clone(),
            Rc::new(|_, _, _| {}),
            Rc::new(|_, _| {}),
            Rc::new(|_, _| {}),
            Rc::new(std::cell::Cell::new(false)),
            store.shared(),
        );
        let ctx = Rc::new(ops::Ctx {
            window: window.clone(),
            manager: manager.clone(),
            toast: adw::ToastOverlay::new(),
            clipboard: Rc::new(std::cell::RefCell::new(ops::ClipTracker::default())),
            preferences: store.clone(),
            focus_path: Rc::new(|| {}),
        });
        let (popover, group) =
            show_background_for(window.upcast_ref(), 10.0, 10.0, &ctx, &dest, &manager);
        assert!(!popover.has_arrow());
        // Actions behind the rows resolve and start enabled.
        for name in ["new-folder", "new-empty-file", "folder-properties"] {
            let action = group
                .lookup_action(name)
                .unwrap_or_else(|| panic!("missing bg action {name}"))
                .downcast::<gio::SimpleAction>()
                .expect("bg actions are simple actions");
            assert!(action.is_enabled(), "bg action {name} starts enabled");
        }
        // Six icon+label rows, exactly one submenu indicator, three
        // separators between the four sections.
        let list = popover
            .child()
            .and_downcast::<gtk::Box>()
            .expect("menu list");
        let rows = row_buttons(&list);
        assert_eq!(rows.len(), 6);
        let mut separators = 0;
        let mut child = list.first_child();
        while let Some(widget) = child {
            if widget.clone().downcast::<gtk::Separator>().is_ok() {
                separators += 1;
            }
            child = widget.next_sibling();
        }
        assert_eq!(separators, 3);
        let mut sub_index = None;
        for (index, button) in rows.iter().enumerate() {
            let (text, icon, sub) = row_shape(button);
            assert!(!text.is_empty(), "row {index} labelled");
            assert!(icon, "row {index} has an icon");
            if sub {
                assert_eq!(sub_index, None, "single submenu parent");
                sub_index = Some(index);
            }
        }
        assert_eq!(sub_index, Some(1), "New File is the second row");
        // Opening the submenu yields three rows, then everything closes
        // without leftovers.
        rows[sub_index.unwrap()].emit_clicked();
        let anchor = popover.parent().expect("menu anchored");
        let mut found = Vec::new();
        popovers_under(&anchor, &mut found);
        assert_eq!(found.len(), 2, "main plus submenu");
        let sub = found
            .into_iter()
            .find(|menu| menu != &popover)
            .expect("submenu popover");
        assert!(!sub.has_arrow());
        let sub_list = sub
            .child()
            .and_downcast::<gtk::Box>()
            .expect("submenu list");
        let sub_rows = row_buttons(&sub_list);
        assert_eq!(sub_rows.len(), 3);
        for (index, button) in sub_rows.iter().enumerate() {
            let (text, icon, sub) = row_shape(button);
            assert!(!text.is_empty(), "sub row {index} labelled");
            assert!(icon, "sub row {index} has an icon");
            assert!(!sub, "no nested sub-submenu");
        }
        popover.popdown();
        let mut leftovers = Vec::new();
        popovers_under(&anchor, &mut leftovers);
        assert!(leftovers.is_empty(), "no popover accumulates");
    }
}
