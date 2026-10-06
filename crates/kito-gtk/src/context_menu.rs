//! Premium context menu: compact popover with icon + label rows.
//!
//! The submenu ("Create") is a second popover next to the first one, not
//! an inner column: this way the main menu keeps its size and stays
//! put. Both popovers always stay `autohide`: cascading close is handled
//! by our `closed` handlers, without touching `autohide` on visible
//! popovers (GTK would unrealize their surface there and unbalance the
//! grabs -> window deaf to clicks).

use adw::prelude::*;
use gtk::gio;
use std::{cell::RefCell, rc::Rc, time::Duration};

/// One menu row: icons, message id for the label, `win.*` action
/// (empty for a submenu) and destructive flag.
struct Row {
    icons: &'static [&'static str],
    label: &'static str,
    action: &'static str,
    danger: bool,
    /// Submenu: rows + separator indexes.
    sub: Option<(&'static [Row], &'static [usize])>,
}

const fn row(icons: &'static [&'static str], label: &'static str, action: &'static str) -> Row {
    Row {
        icons,
        label,
        action,
        danger: false,
        sub: None,
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
        sub: None,
    }
}

const fn menu(
    icons: &'static [&'static str],
    label: &'static str,
    sub: &'static [Row],
    separators: &'static [usize],
) -> Row {
    Row {
        icons,
        label,
        action: "",
        danger: false,
        sub: Some((sub, separators)),
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

/// "Create" column: the type is only the initial name, the dialog
/// always asks for the real name.
const CREATE_ROWS: [Row; 6] = [
    row(
        &["folder-new", "folder-new-symbolic"],
        "menu-new-folder",
        "new-folder",
    ),
    row(&["text-x-generic"], "menu-new-text-file", "new-text-file"),
    row(&["document-new"], "menu-new-empty-file", "new-empty-file"),
    row(
        &["x-office-document", "application-msword"],
        "menu-new-word-doc",
        "new-word-doc",
    ),
    row(
        &["x-office-spreadsheet"],
        "menu-new-spreadsheet",
        "new-spreadsheet",
    ),
    row(&["text-html"], "menu-new-html", "new-html-page"),
];

/// Menu on the background (empty folder area).
const BACKGROUND_ROWS: [Row; 5] = [
    menu(
        &["list-add", "folder-new"],
        "menu-create",
        &CREATE_ROWS,
        &[],
    ),
    row(&["edit-paste"], "menu-paste", "paste"),
    row(
        &["utilities-terminal", "terminal"],
        "menu-open-terminal",
        "open-terminal",
    ),
    row(
        &["utilities-terminal", "terminal"],
        "menu-open-terminal-root",
        "open-terminal-root",
    ),
    row(
        &["dialog-information", "help-about"],
        "menu-properties",
        "properties",
    ),
];

/// Groups: [0..1, 1..2, 2..4, 4..5].
const BACKGROUND_SEPARATORS_AFTER: [usize; 3] = [0, 2, 4];

/// Trash background: the only sensible action is emptying it.
const TRASH_BACKGROUND_ROWS: [Row; 1] = [danger_row(
    &["user-trash-full", "user-trash"],
    "menu-empty-trash",
    "empty-trash",
)];

/// Delay before closing the submenu when the pointer leaves its
/// rows: gives it time to actually get there.
const CLOSE_DELAY: Duration = Duration::from_millis(300);

/// Open submenu and timer that closes it. The reference is weak:
/// menu rows live inside the popovers, they don't keep them alive.
#[derive(Default)]
struct MenuState {
    submenu: RefCell<Option<glib::WeakRef<gtk::Popover>>>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl MenuState {
    /// Closes the submenu; closing the child also closes the
    /// parent (cascade in `open_submenu`), and closing the parent
    /// closes the child (handler in `popup`).
    fn close_submenu(&self) {
        self.cancel_close();
        // `take()` outside `if let`: the temporary `RefMut` dies at the
        // end of the statement, before `popdown()`. In the `if let`
        // scrutinee (edition 2021) it would live until the end of the
        // block, and `popdown()` emits `closed` synchronously reentering
        // here via `connect_closed` -> "RefCell already borrowed" -> abort.
        let submenu = self.submenu.borrow_mut().take();
        if let Some(weak) = submenu {
            if let Some(popover) = weak.upgrade() {
                popover.popdown();
            }
        }
    }

    /// The pointer is still inside the menu: no close pending.
    fn cancel_close(&self) {
        let timer = self.timer.borrow_mut().take();
        if let Some(id) = timer {
            id.remove();
        }
    }

    /// Closes after `CLOSE_DELAY`, if the pointer does not come back.
    fn schedule_close(self: &Rc<Self>) {
        self.cancel_close();
        let this = self.clone();
        *self.timer.borrow_mut() = Some(glib::timeout_add_local_once(CLOSE_DELAY, move || {
            *this.timer.borrow_mut() = None;
            this.close_submenu();
        }));
    }
}

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

/// Menu row: flat button with icon, label and arrow.
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
    if r.sub.is_some() {
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

/// Builds the popover with its rows, without opening it.
fn build(
    anchor: &gtk::Widget,
    rows: &[Row],
    separators_after: &[usize],
    window: &adw::ApplicationWindow,
    state: &Rc<MenuState>,
    parent: Option<&gtk::Popover>,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("ctx-menu");
    let list = menu_box();
    for (i, r) in rows.iter().enumerate() {
        let button = row_button(r);
        if let Some((sub_rows, sub_separators)) = r.sub {
            // The submenu opens on row hover; click stays
            // an alternative way.
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter({
                let state = state.clone();
                let anchor = button.clone();
                let popover = popover.clone();
                let window = window.clone();
                move |_, _, _| {
                    open_submenu(&anchor, &popover, sub_rows, sub_separators, &window, &state)
                }
            });
            motion.connect_leave({
                let state = state.clone();
                move |_| state.schedule_close()
            });
            button.add_controller(motion);
            let state = state.clone();
            let anchor = button.clone();
            let popover = popover.clone();
            let window = window.clone();
            button.connect_clicked(move |_| {
                open_submenu(&anchor, &popover, sub_rows, sub_separators, &window, &state)
            });
        } else {
            // On a plain row the submenu is not needed: away at once.
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter({
                let state = state.clone();
                move |_, _, _| state.close_submenu()
            });
            button.add_controller(motion);
            let window = window.clone();
            let action = r.action.to_string();
            let popover = popover.clone();
            let parent = parent.cloned();
            button.connect_clicked(move |_| {
                popover.popdown();
                if let Some(parent) = &parent {
                    parent.popdown();
                }
                let _ = gtk::prelude::WidgetExt::activate_action(
                    &window,
                    &format!("win.{action}"),
                    None,
                );
            });
        }
        list.append(&button);
        if separators_after.contains(&i) {
            list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
    }
    popover.set_child(Some(&list));
    popover.set_parent(anchor);
    popover
}

/// Opens the submenu next to the `anchor` row, only once.
/// Cascading close lives in the two `closed` handlers: the child closes
/// the parent and the parent closes the child. No `set_autohide`: on a
/// visible popover GTK unrealizes its surface and grabs stay
/// unbalanced (window deaf to clicks).
fn open_submenu(
    anchor: &gtk::Button,
    parent: &gtk::Popover,
    rows: &[Row],
    separators_after: &[usize],
    window: &adw::ApplicationWindow,
    state: &Rc<MenuState>,
) {
    state.cancel_close();
    if state.submenu.borrow().is_some() {
        return;
    }
    let sub_state = Rc::new(MenuState::default());
    let child = build(
        anchor.upcast_ref(),
        rows,
        separators_after,
        window,
        &sub_state,
        Some(parent),
    );
    // Submenu body: inside cancels the close, outside schedules it.
    if let Some(list) = child.child() {
        let motion = gtk::EventControllerMotion::new();
        motion.connect_enter({
            let state = state.clone();
            move |_, _, _| state.cancel_close()
        });
        motion.connect_leave({
            let state = state.clone();
            move |_| state.schedule_close()
        });
        list.add_controller(motion);
    }
    // Closing the child (outside click, ESC, ...) restores the parent.
    let parent_on_close = parent.clone();
    child.connect_closed({
        let state = state.clone();
        move |_| {
            state.close_submenu();
            parent_on_close.popdown();
        }
    });

    // Attached to the row edge on the side with room, at the top.
    let side = submenu_side(anchor.upcast_ref());
    child.set_position(side);
    let x = if matches!(side, gtk::PositionType::Left) {
        0
    } else {
        anchor.width()
    };
    child.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x, 0, 1, 1)));
    // Detach the submenu from the menu with a small margin.
    child.set_margin_start(6);
    child.set_margin_end(6);
    child.set_margin_top(6);
    child.set_margin_bottom(6);

    *state.submenu.borrow_mut() = Some(child.downgrade());
    child.popup();
}

/// Submenu side: right of the row if the screen has room,
/// otherwise left.
fn submenu_side(anchor: &gtk::Widget) -> gtk::PositionType {
    /// Comfortable width for the submenu.
    const NEEDED: f32 = 240.0;
    let Some(root) = anchor.root() else {
        return gtk::PositionType::Right;
    };
    let Some(rect) = anchor.compute_bounds(&root) else {
        return gtk::PositionType::Right;
    };
    let right = root.width() as f32 - (rect.x() + rect.width());
    if right < NEEDED && rect.x() > right {
        gtk::PositionType::Left
    } else {
        gtk::PositionType::Right
    }
}

/// Builds and opens the menu anchored at `(x, y)` on `anchor`.
fn popup(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    window: &adw::ApplicationWindow,
    rows: &[Row],
    separators_after: &[usize],
) {
    let state = Rc::new(MenuState::default());
    let popover = build(anchor, rows, separators_after, window, &state, None);
    // Leaving the menu also closes a possible submenu.
    if let Some(list) = popover.child() {
        let motion = gtk::EventControllerMotion::new();
        motion.connect_leave({
            let state = state.clone();
            move |_| state.schedule_close()
        });
        list.add_controller(motion);
    }
    popover.connect_closed({
        let state = state.clone();
        move |_| state.close_submenu()
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

/// Right-click on the background: create, paste, terminal, properties.
pub fn show_background(anchor: &gtk::Widget, x: f64, y: f64, window: &adw::ApplicationWindow) {
    popup(
        anchor,
        x,
        y,
        window,
        &BACKGROUND_ROWS,
        &BACKGROUND_SEPARATORS_AFTER,
    );
}

/// Right-click on the trash background: empty the trash.
pub fn show_trash_background(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    window: &adw::ApplicationWindow,
) {
    popup(anchor, x, y, window, &TRASH_BACKGROUND_ROWS, &[]);
}
