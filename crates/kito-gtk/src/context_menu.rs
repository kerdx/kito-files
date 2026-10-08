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
// Background menu: native `gio::Menu` in a `gtk::PopoverMenu`.
//
// The menu refers to the tab folder captured at open time (`dest`), never
// to the live selection: folder properties ignore selected files, and every
// action carries its own destination. Availability reflects the context
// (clipboard formats, locality, terminal setup, writability) without
// touching the shared `win.*` actions used by shortcuts and file menus.
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

/// One native menu item: translated label + symbolic icon.
fn bg_item(label_key: &str, detailed_action: &str, icons: &[&str]) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(&crate::l10n::tr(label_key)), Some(detailed_action));
    item.set_icon(&gio::ThemedIcon::from_names(icons));
    item
}

/// Background model for `dest`. Sections give the separators (none after
/// the last one); the terminal section exists only for local paths.
/// Word/Spreadsheet are intentionally absent: only placeholders would be
/// created, not valid documents.
pub(crate) fn bg_menu_model(include_terminal: bool) -> gio::Menu {
    let menu = gio::Menu::new();

    let create = gio::Menu::new();
    create.append_item(&bg_item(
        "bg-new-folder",
        "bg.new-folder",
        &["folder-new", "folder-new-symbolic"],
    ));
    let new_file = gio::Menu::new();
    new_file.append_item(&bg_item(
        "menu-new-empty-file",
        "bg.new-empty-file",
        &["document-new"],
    ));
    new_file.append_item(&bg_item(
        "menu-new-text-file",
        "bg.new-text-file",
        &["text-x-generic"],
    ));
    new_file.append_item(&bg_item(
        "menu-new-html",
        "bg.new-html-page",
        &["text-html"],
    ));
    let new_file_item = gio::MenuItem::new(Some(&crate::l10n::tr("bg-new-file")), None);
    new_file_item.set_submenu(Some(&new_file));
    new_file_item.set_icon(&gio::ThemedIcon::from_names(&[
        "document-new",
        "document-new-symbolic",
    ]));
    create.append_item(&new_file_item);
    menu.append_section(None, &create);

    let edit = gio::Menu::new();
    edit.append_item(&bg_item("menu-paste", "bg.paste", &["edit-paste"]));
    menu.append_section(None, &edit);

    if include_terminal {
        let terminal = gio::Menu::new();
        terminal.append_item(&bg_item(
            "menu-open-terminal",
            "bg.open-terminal",
            &["utilities-terminal", "terminal"],
        ));
        terminal.append_item(&bg_item(
            "menu-open-terminal-root",
            "bg.open-terminal-root",
            &["utilities-terminal", "terminal"],
        ));
        menu.append_section(None, &terminal);
    }

    let props = gio::Menu::new();
    props.append_item(&bg_item(
        "bg-folder-properties",
        "bg.folder-properties",
        &["dialog-information", "help-about"],
    ));
    menu.append_section(None, &props);

    menu
}

/// Trash background model: the dedicated single destructive action.
pub(crate) fn trash_bg_menu_model() -> gio::Menu {
    let menu = gio::Menu::new();
    let section = gio::Menu::new();
    section.append_item(&bg_item(
        "menu-empty-trash",
        "win.empty-trash",
        &["user-trash-full", "user-trash"],
    ));
    menu.append_section(None, &section);
    menu
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
    popover: &gtk::PopoverMenu,
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

/// Opens a native background menu for `dest` and tracks it for
/// tab-switch/navigation invalidation. The popover unparents itself on
/// close; repeated openings accumulate nothing.
///
/// Presentation is deferred by one idle (like `GtkMenuButton` does): the
/// menu tracker's build idles run first, so measuring and popup use final
/// content instead of growing after mapping, which would force a
/// scrollbar. No nested loop pumping inside the input handler.
fn popup_native(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    model: &gio::Menu,
    group: Option<(&str, &gio::SimpleActionGroup)>,
    manager: &Rc<TabManager>,
    dest: &str,
) -> gtk::PopoverMenu {
    // NESTED (not the from_model SLIDING default): traditional side
    // submenus with the `>` indicator. GTK builds nested submenu popovers
    // arrowless itself (`gtk_popover_set_has_arrow (submenu, FALSE)` in
    // gtkmenusectionbox.c), so one call here covers every level.
    let popover = gtk::PopoverMenu::from_model_full(model, gtk::PopoverMenuFlags::NESTED);
    popover.set_has_arrow(false);
    if let Some((name, group)) = group {
        popover.insert_action_group(name, Some(group));
    }
    let anchor_weak = anchor.downgrade();
    let manager_weak = Rc::downgrade(manager);
    let dest = dest.to_string();
    let popup = popover.clone();
    glib::idle_add_local_once(move || {
        let (Some(anchor), Some(manager)) = (anchor_weak.upgrade(), manager_weak.upgrade()) else {
            return;
        };
        // Stale press (navigated or switched tabs meanwhile): the tracked
        // close on navigation covers the rest.
        let fresh = manager
            .selected_uri()
            .as_deref()
            .is_some_and(|current| current == dest);
        if !fresh {
            return;
        }
        let popover = popup;
        popover.set_parent(&anchor);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        let (_, natural_h, _, _) = popover.measure(gtk::Orientation::Vertical, -1);
        popover.set_position(vertical_side(&anchor, y, natural_h));
        manager.track_bg_menu(&popover);
        let tracked = popover.clone();
        let manager_weak = Rc::downgrade(&manager);
        popover.connect_closed(move |popover| {
            popover.unparent();
            if let Some(manager) = manager_weak.upgrade() {
                manager.forget_bg_menu(&tracked);
            }
        });
        popover.popup();
    });
    popover
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
) -> (gtk::PopoverMenu, gio::SimpleActionGroup) {
    let local = kito_core::uri_to_path(dest).is_some();
    let model = bg_menu_model(local);
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
    let popover = popup_native(anchor, x, y, &model, Some(("bg", &group)), manager, dest);
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
    dest: &str,
    manager: &Rc<TabManager>,
) {
    let model = trash_bg_menu_model();
    popup_native(anchor, x, y, &model, None, manager, dest);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Actions of a menu level's items, in order (submenu parents carry
    /// none, which surfaces as an empty string).
    fn level_actions(menu: &impl IsA<gio::MenuModel>, n: i32) -> Vec<String> {
        (0..n)
            .map(|i| {
                let attrs = menu.iterate_item_attributes(i);
                loop {
                    match attrs.next() {
                        Some((name, value)) if name == "action" => {
                            break value.get::<String>().unwrap_or_default()
                        }
                        Some(_) => continue,
                        None => break String::new(),
                    }
                }
            })
            .collect()
    }

    /// The linked sub-model of a level item (`section` or `submenu`).
    fn level_link(menu: &impl IsA<gio::MenuModel>, index: i32, link: &str) -> gio::MenuModel {
        let links = menu.iterate_item_links(index);
        loop {
            match links.next() {
                Some((name, model)) if name == link => return model,
                Some(_) => continue,
                None => panic!("missing {link} link at index {index}"),
            }
        }
    }

    #[test]
    fn background_structure_matches_the_spec() {
        let menu = bg_menu_model(true);
        // Four sections: create, paste, terminal, properties.
        assert_eq!(menu.n_items(), 4);
        // Sections only separate: no trailing separator is expressible.
        let create = level_link(&menu, 0, "section");
        assert_eq!(create.n_items(), 2);
        let actions = level_actions(&create, create.n_items());
        assert_eq!(actions, vec!["bg.new-folder".to_string(), String::new()]);
    }

    #[test]
    fn background_hides_terminal_off_local_paths() {
        assert_eq!(bg_menu_model(true).n_items(), 4);
        // Remote: create, paste, properties only.
        assert_eq!(bg_menu_model(false).n_items(), 3);
    }

    #[test]
    fn background_submenu_holds_only_valid_creations() {
        let menu = bg_menu_model(true);
        let create = level_link(&menu, 0, "section");
        // Second entry is the New File submenu (no action of its own).
        let submenu = level_link(&create, 1, "submenu");
        let actions = level_actions(&submenu, submenu.n_items());
        // Empty, text, HTML — no Word/Spreadsheet placeholders.
        assert_eq!(
            actions,
            vec![
                "bg.new-empty-file".to_string(),
                "bg.new-text-file".to_string(),
                "bg.new-html-page".to_string(),
            ]
        );
    }

    #[test]
    fn trash_background_holds_only_empty_trash() {
        let menu = trash_bg_menu_model();
        assert_eq!(menu.n_items(), 1);
        let section = level_link(&menu, 0, "section");
        assert_eq!(section.n_items(), 1);
        assert_eq!(
            level_actions(&section, 1),
            vec!["win.empty-trash".to_string()]
        );
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

    /// The `bg` action group behind the native menu resolves: creation and
    /// folder-properties actions exist and start enabled on a writable
    /// local folder. Needs a display; skipped headless.
    #[test]
    fn bg_menu_actions_resolve_on_a_writable_folder() {
        let _ = gtk::init();
        let _display = gdk::Display::default()
            .or_else(|| gdk::Display::open(std::env::var("WAYLAND_DISPLAY").ok().as_deref()));
        let Some(_) = gdk::Display::default() else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let dest = format!("file://{}", tmp.path().display());
        let app = adw::Application::builder()
            .application_id("it.kito.BgMenuTest")
            .build();
        let holder: Rc<std::cell::RefCell<Option<(gtk::PopoverMenu, gio::SimpleActionGroup)>>> =
            Rc::new(std::cell::RefCell::new(None));
        let holder_in = holder.clone();
        let dest_in = dest.clone();
        app.connect_activate(move |app| {
            let window: adw::ApplicationWindow =
                adw::ApplicationWindow::builder().application(app).build();
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
            let opened =
                show_background_for(window.upcast_ref(), 10.0, 10.0, &ctx, &dest_in, &manager);
            // Assert in a later idle: the tracker builds nested content
            // asynchronously.
            let holder_in = holder_in.clone();
            let quit = app.clone();
            glib::idle_add_local_once(move || {
                *holder_in.borrow_mut() = Some(opened);
                quit.quit();
            });
        });
        app.run_with_args::<&str>(&[]);
        let Some((popover, group)) = holder.borrow().clone() else {
            panic!("background menu did not open");
        };
        assert!(!popover.has_arrow());
        for name in ["new-folder", "new-empty-file", "folder-properties"] {
            let action = group
                .lookup_action(name)
                .unwrap_or_else(|| panic!("missing bg action {name}"))
                .downcast::<gio::SimpleAction>()
                .expect("bg actions are simple actions");
            assert!(action.is_enabled(), "bg action {name} starts enabled");
        }
        popover.popdown();
        popover.unparent();
    }
}
