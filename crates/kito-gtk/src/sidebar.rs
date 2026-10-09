//! Sidebar: shortcuts, XDG folders and mounted volumes.
//! UI labels use the app catalogs; names returned by GIO remain system data.

use crate::{
    dnd::{self, DropHandler},
    icons,
};
use adw::prelude::*;
use gtk::{gio, glib};
use std::{cell::RefCell, rc::Rc};

type LoadFn = Rc<dyn Fn(&str)>;
/// Sidebar rows for highlighting the current folder.
type Rows = Rc<RefCell<Vec<(String, gtk::Button)>>>;

const PLACES: [(&str, &str, glib::UserDirectory); 5] = [
    (
        "folder-documents",
        "side-documents",
        glib::UserDirectory::Documents,
    ),
    ("folder-music", "side-music", glib::UserDirectory::Music),
    (
        "folder-pictures",
        "side-pictures",
        glib::UserDirectory::Pictures,
    ),
    ("folder-videos", "side-videos", glib::UserDirectory::Videos),
    (
        "folder-download",
        "side-downloads",
        glib::UserDirectory::Downloads,
    ),
];

/// Registers the row and adds it to the container.
fn add_row(container: &gtk::Box, rows: &Rows, uri: &str, button: &gtk::Button) {
    rows.borrow_mut().push((uri.to_string(), button.clone()));
    container.append(button);
}

fn update_nav_row_label(button: &gtk::Button, label: &str) {
    if let Some(label_widget) = button.child().and_then(|child| find_side_label(&child)) {
        label_widget.set_text(label);
    }
    button.set_tooltip_text(Some(label));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
}

/// Clickable row with icon + label.
fn nav_row(
    icon: &impl IsA<gio::Icon>,
    label: &str,
    load: LoadFn,
    uri: String,
    on_drop: &DropHandler,
) -> gtk::Button {
    let button = gtk::Button::builder()
        .has_frame(false)
        .css_classes(["side-row"])
        .tooltip_text(label)
        .build();
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    row.append(&{
        let image = gtk::Image::from_gicon(icon);
        image.set_pixel_size(18);
        image
    });
    row.append(
        &gtk::Label::builder()
            .label(label)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["side-label"])
            .build(),
    );
    button.set_child(Some(&row));
    let destination = uri.clone();
    dnd::attach_drop_target(
        &button,
        Rc::new(move || Some(destination.clone())),
        on_drop.clone(),
    );
    button.connect_clicked(move |_| load(&uri));
    button
}

fn section_separator() -> gtk::Separator {
    gtk::Separator::builder()
        .orientation(gtk::Orientation::Horizontal)
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(4)
        .build()
}

fn disclosure_icon(expanded: bool) -> gio::ThemedIcon {
    if expanded {
        icons::control_icon(&["pan-down-symbolic", "pan-down", "go-down"])
    } else {
        icons::control_icon(&["pan-end-symbolic", "pan-end", "go-next"])
    }
}

fn favorites_toggle_row() -> (gtk::Button, gtk::Label, gtk::Image) {
    let label_text = crate::l10n::tr("side-favorites");
    let description = crate::l10n::tr("side-favorites-toggle");
    let button = gtk::Button::builder()
        .has_frame(false)
        .css_classes(["side-row", "favorites-toggle"])
        .tooltip_text(&description)
        .build();
    button.update_property(&[
        gtk::accessible::Property::Label(&label_text),
        gtk::accessible::Property::Description(&description),
    ]);
    button.update_state(&[gtk::accessible::State::Expanded(Some(false))]);

    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    let star = gtk::Image::from_gicon(&icons::control_icon(&[
        "starred-symbolic",
        "star-symbolic",
        "starred",
        "star",
    ]));
    star.set_pixel_size(18);
    row.append(&star);

    let label = gtk::Label::builder()
        .label(&label_text)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .css_classes(["side-label"])
        .build();
    row.append(&label);

    let arrow = gtk::Image::from_gicon(&disclosure_icon(false));
    arrow.set_pixel_size(12);
    row.append(&arrow);
    button.set_child(Some(&row));
    (button, label, arrow)
}

fn set_favorites_expanded(
    button: &gtk::Button,
    contents: &gtk::Box,
    arrow: &gtk::Image,
    expanded: bool,
) {
    contents.set_visible(expanded);
    arrow.set_from_gicon(&disclosure_icon(expanded));
    button.update_state(&[gtk::accessible::State::Expanded(Some(expanded))]);
}

/// Bookmark row with right-click to remove it. The sidebar reloads
/// on its own via monitor on the file (see `build_sidebar`).
fn bookmark_row(name: &str, uri: &str, load: LoadFn, on_drop: &DropHandler) -> gtk::Button {
    let button = nav_row(&bookmark_icon(uri), name, load, uri.to_string(), on_drop);
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_SECONDARY)
        .build();
    let button_weak = button.downgrade();
    let uri = uri.to_string();
    gesture.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(button) = button_weak.upgrade() else {
            return;
        };
        let popover = gtk::Popover::new();
        let remove = gtk::Button::builder()
            .label(crate::l10n::tr("side-remove"))
            .has_frame(false)
            .build();
        let uri = uri.clone();
        remove.connect_clicked({
            let popover = popover.clone();
            move |_| {
                popover.popdown();
                let _ = kito_core::bookmarks::unpin(&uri);
            }
        });
        popover.set_child(Some(&remove));
        popover.set_parent(&button);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    button.add_controller(gesture);
    button
}

fn place_icon(primary: &str) -> gio::ThemedIcon {
    let symbolic = format!("{primary}-symbolic");
    icons::control_icon(&[&symbolic, "folder-symbolic", primary, "folder"])
}

fn bookmark_icon(uri: &str) -> gio::ThemedIcon {
    let generic_folder = || icons::control_icon(&["folder-symbolic", "folder"]);
    let Some(path) = gio::File::for_uri(uri).path() else {
        return generic_folder();
    };
    if path == glib::home_dir() {
        return place_icon("user-home");
    }
    if glib::user_special_dir(glib::UserDirectory::Desktop)
        .is_some_and(|desktop| desktop.as_path() == path.as_path())
    {
        return place_icon("user-desktop");
    }
    for (icon_name, _, directory) in PLACES {
        if glib::user_special_dir(directory)
            .is_some_and(|special| special.as_path() == path.as_path())
        {
            return place_icon(icon_name);
        }
    }
    generic_folder()
}

/// Localized folder name from GIO (e.g. "Documents" on an Italian
/// system). It is system data, not a UI string to translate.
fn display_name(path: &std::path::Path, fallback: &str) -> String {
    gio::File::for_path(path)
        .query_info(
            "standard::display-name",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .map(|info| info.display_name().to_string())
        .unwrap_or_else(|_| fallback.to_string())
}

fn shortcut_places(
    load: &LoadFn,
    parent: &gtk::Box,
    rows: &Rows,
    favorites_toggle: &gtk::Button,
    favorite_rows: &gtk::Box,
    window: &adw::ApplicationWindow,
    on_drop: &DropHandler,
) -> [gtk::Button; 3] {
    // Home is the first quick-access row; user folders have their own group.
    let home = glib::home_dir();
    let home_uri = gio::File::for_path(&home).uri().to_string();
    let home_button = nav_row(
        &place_icon("user-home"),
        &crate::l10n::tr("side-home"),
        load.clone(),
        home_uri.clone(),
        on_drop,
    );
    add_row(parent, rows, &home_uri, &home_button);

    parent.append(favorites_toggle);
    parent.append(favorite_rows);

    let network = icons::control_icon(&[
        "network-computer-symbolic",
        "network-workgroup-symbolic",
        "network-computer",
        "network-workgroup",
    ]);
    let network_button = nav_row(
        &network,
        &crate::l10n::tr("side-network"),
        load.clone(),
        "network:///".to_string(),
        on_drop,
    );
    add_row(parent, rows, "network:///", &network_button);

    let trash_button = trash_row(load.clone(), window, on_drop);
    add_row(parent, rows, kito_core::TRASH_URI, &trash_button);
    [home_button, network_button, trash_button]
}

fn sidebar_place_uris() -> Vec<String> {
    let mut places = vec![gio::File::for_path(glib::home_dir()).uri().to_string()];
    for (_, _, directory) in PLACES {
        if let Some(path) = glib::user_special_dir(directory).filter(|path| path.is_dir()) {
            places.push(gio::File::for_path(&path).uri().to_string());
        }
    }
    places
}

fn unique_bookmarks(
    bookmarks: Vec<(String, String)>,
    standard_locations: &[String],
) -> Vec<(String, String)> {
    let mut known = standard_locations
        .iter()
        .map(|uri| gio::File::for_uri(uri))
        .collect::<Vec<_>>();
    bookmarks
        .into_iter()
        .filter_map(|(name, uri)| {
            let target = gio::File::for_uri(&uri);
            if known.iter().any(|place| target.equal(place)) {
                None
            } else {
                known.push(target);
                Some((name, uri))
            }
        })
        .collect()
}

fn xdg_places(load: &LoadFn, parent: &gtk::Box, rows: &Rows, on_drop: &DropHandler) {
    for (icon_name, label_id, dir) in PLACES {
        let Some(path) = glib::user_special_dir(dir) else {
            continue;
        };
        if !path.is_dir() {
            continue;
        }
        let uri = gio::File::for_path(&path).uri().to_string();
        let button = nav_row(
            &place_icon(icon_name),
            &display_name(&path, &crate::l10n::tr(label_id)),
            load.clone(),
            uri.clone(),
            on_drop,
        );
        add_row(parent, rows, &uri, &button);
    }
}

/// Trash row in Places: opens `trash:///` and has the "Empty Trash…"
/// menu on right-click (activates `win.empty-trash`, which asks first).
/// Symbolic trash icon, with the current theme resolving fallbacks.
fn trash_icon(full: bool) -> gio::ThemedIcon {
    if full {
        icons::control_icon(&[
            "user-trash-full-symbolic",
            "user-trash-symbolic",
            "user-trash-full",
            "user-trash",
        ])
    } else {
        icons::control_icon(&[
            "user-trash-symbolic",
            "user-trash-full-symbolic",
            "user-trash",
            "user-trash-full",
        ])
    }
}

/// Trash icon updater state: pure generation-guarded logic, no widgets,
/// no I/O. Async probes report back with their generation; superseded
/// results are ignored, so overlapping queries and stale answers can
/// never flip the icon wrongly. Starts empty until the first result.
#[derive(Default)]
struct TrashIconState {
    applied: bool,
    next_generation: u64,
    pending: u64,
}

impl TrashIconState {
    /// Currently shown state.
    fn current(&self) -> bool {
        self.applied
    }

    /// Starts a probe, returning its generation.
    fn begin_query(&mut self) -> u64 {
        self.next_generation += 1;
        self.pending = self.next_generation;
        self.pending
    }

    /// Applies a finished probe. Returns the new state iff the icon
    /// must change; stale generations return `None`.
    fn apply_result(&mut self, generation: u64, full: bool) -> Option<bool> {
        if generation != self.pending {
            return None;
        }
        self.pending = 0;
        let changed = full != self.applied;
        self.applied = full;
        changed.then_some(full)
    }
}

/// Applies a finished probe to the image if still alive: the icon
/// changes only when the guarded state actually flips.
fn apply_trash_result(
    image: &glib::WeakRef<gtk::Image>,
    state: &Rc<RefCell<TrashIconState>>,
    generation: u64,
    full: bool,
) {
    let Some(image) = image.upgrade() else {
        return;
    };
    if let Some(full) = state.borrow_mut().apply_result(generation, full) {
        image.set_from_gicon(&trash_icon(full));
    }
}

/// One async fullness probe: enumerate (async) + first entry (async),
/// then apply through the generation guard. Never blocks the UI thread;
/// backend errors count as empty without crashing.
fn poll_trash_once(image: &glib::WeakRef<gtk::Image>, state: &Rc<RefCell<TrashIconState>>) {
    let generation = state.borrow_mut().begin_query();
    let image_weak = image.clone();
    let state = state.clone();
    gio::File::for_uri(kito_core::TRASH_URI).enumerate_children_async(
        "standard::name",
        gio::FileQueryInfoFlags::NONE,
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        move |listed| {
            let image_weak = image_weak.clone();
            let state = state.clone();
            match listed {
                Ok(children) => children.next_files_async(
                    1,
                    glib::Priority::DEFAULT,
                    None::<&gio::Cancellable>,
                    move |first| {
                        let full = first.map(|infos| !infos.is_empty()).unwrap_or(false);
                        apply_trash_result(&image_weak, &state, generation, full);
                    },
                ),
                Err(_) => apply_trash_result(&image_weak, &state, generation, false),
            }
        },
    );
}

fn trash_row(load: LoadFn, window: &adw::ApplicationWindow, on_drop: &DropHandler) -> gtk::Button {
    let button = nav_row(
        &trash_icon(false),
        &crate::l10n::tr("side-trash"),
        load,
        kito_core::TRASH_URI.to_string(),
        on_drop,
    );
    // The icon follows the trash asynchronously: an initial probe plus
    // a bounded 3s poll, both async so the UI thread never blocks. The
    // generation guard drops superseded answers; weak references and an
    // explicit teardown on destroy keep nothing alive past the row.
    let state: Rc<RefCell<TrashIconState>> = Rc::default();
    let timer_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    if let Some(image) = button
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|b| b.first_child())
        .and_downcast::<gtk::Image>()
    {
        let query: Rc<dyn Fn()> = Rc::new({
            let image_weak = image.downgrade();
            let state = state.clone();
            move || poll_trash_once(&image_weak, &state)
        });
        query();
        let timer_query = query.clone();
        let timer_weak = image.downgrade();
        let timer_slot = timer_id.clone();
        *timer_id.borrow_mut() = Some(glib::timeout_add_local(
            std::time::Duration::from_secs(3),
            move || {
                if timer_weak.upgrade().is_none() {
                    *timer_slot.borrow_mut() = None;
                    return glib::ControlFlow::Break;
                }
                timer_query();
                glib::ControlFlow::Continue
            },
        ));
        let timer_slot = timer_id.clone();
        button.connect_destroy(move |_| {
            if let Some(id) = timer_slot.borrow_mut().take() {
                id.remove();
            }
        });
    }
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_SECONDARY)
        .build();
    let button_weak = button.downgrade();
    let window = window.clone();
    // Menu icon from the last applied probe: no sync query on click.
    let menu_state = state.clone();
    gesture.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(button) = button_weak.upgrade() else {
            return;
        };
        let popover = gtk::Popover::new();
        popover.add_css_class("ctx-menu");
        let empty = gtk::Button::builder()
            .has_frame(false)
            .css_classes(["ctx-row"])
            .build();
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .margin_start(8)
            .margin_end(8)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        let image = gtk::Image::from_gicon(&trash_icon(menu_state.borrow().current()));
        image.set_pixel_size(18);
        row.append(&image);
        row.append(
            &gtk::Label::builder()
                .label(crate::l10n::tr("menu-empty-trash"))
                .halign(gtk::Align::Start)
                .hexpand(true)
                .css_classes(["error"])
                .build(),
        );
        empty.set_child(Some(&row));
        let popover_weak = popover.downgrade();
        let window = window.clone();
        empty.connect_clicked(move |_| {
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
            let _ = gtk::prelude::WidgetExt::activate_action(&window, "win.empty-trash", None);
        });
        popover.set_child(Some(&empty));
        popover.set_parent(&button);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    button.add_controller(gesture);
    button
}

fn error_dialog(window: &adw::ApplicationWindow, body: String) {
    let dialog = adw::AlertDialog::builder()
        .heading(crate::l10n::tr("side-op-failed"))
        .body(body)
        .build();
    dialog.add_response("ok", &crate::l10n::tr("dialog-ok"));
    dialog.present(Some(window));
}

/// Finishes a mount operation without losing backend errors.
fn report_volume_operation(window: adw::ApplicationWindow) -> impl FnOnce(Result<(), glib::Error>) {
    move |result| {
        if let Err(error) = result {
            error_dialog(&window, error.to_string());
        }
    }
}

/// Accessible eject/unmount action for a mounted volume, when supported.
fn volume_action_button(
    volume: &gio::Volume,
    mount: Option<gio::Mount>,
    window: &adw::ApplicationWindow,
) -> Option<gtk::Button> {
    let eject_volume = volume.can_eject();
    let eject_mount = mount.as_ref().is_some_and(|mount| mount.can_eject());
    let unmount = mount.as_ref().is_some_and(|mount| mount.can_unmount());
    if !eject_volume && !eject_mount && !unmount {
        return None;
    }

    let label = crate::l10n::tr(if eject_volume || eject_mount {
        "side-eject"
    } else {
        "side-unmount"
    });
    let button = gtk::Button::builder()
        .has_frame(false)
        .css_classes(["flat", "side-device-action"])
        .tooltip_text(&label)
        .build();
    let icon = gtk::Image::from_gicon(&icons::control_icon(&[
        "media-eject-symbolic",
        "drive-eject-symbolic",
        "media-eject",
        "drive-eject",
    ]));
    icon.set_pixel_size(16);
    button.set_child(Some(&icon));
    button.update_property(&[gtk::accessible::Property::Label(&label)]);

    let volume = volume.clone();
    let window = window.clone();
    button.connect_clicked(move |_| {
        if eject_volume {
            volume.eject_with_operation(
                gio::MountUnmountFlags::NONE,
                None::<&gio::MountOperation>,
                gio::Cancellable::NONE,
                report_volume_operation(window.clone()),
            );
        } else if let Some(mount) = mount.as_ref() {
            if eject_mount {
                mount.eject_with_operation(
                    gio::MountUnmountFlags::NONE,
                    None::<&gio::MountOperation>,
                    gio::Cancellable::NONE,
                    report_volume_operation(window.clone()),
                );
            } else if unmount {
                mount.unmount_with_operation(
                    gio::MountUnmountFlags::NONE,
                    None::<&gio::MountOperation>,
                    gio::Cancellable::NONE,
                    report_volume_operation(window.clone()),
                );
            }
        }
    });
    Some(button)
}

/// Volume row: if mounted it opens, otherwise it tries to mount first.
fn volume_row(
    volume: &gio::Volume,
    load: LoadFn,
    window: adw::ApplicationWindow,
    on_drop: &DropHandler,
) -> (gtk::Box, Option<(String, gtk::Button)>) {
    let name = volume.name();
    let button = gtk::Button::builder()
        .has_frame(false)
        .css_classes(["side-row"])
        .hexpand(true)
        .tooltip_text(name.as_str())
        .build();
    button.update_property(&[gtk::accessible::Property::Label(name.as_str())]);
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    let volume_icon = icons::device_icon(&volume.icon());
    let volume_image = gtk::Image::from_gicon(&volume_icon);
    volume_image.set_pixel_size(18);
    row.append(&volume_image);
    row.append(
        &gtk::Label::builder()
            .label(name.as_str())
            .halign(gtk::Align::Start)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["side-label"])
            .build(),
    );
    button.set_child(Some(&row));

    let line = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(2)
        .build();
    line.append(&button);

    if let Some(root) = volume.activation_root() {
        let uri = root.uri().to_string();
        let destination = uri.clone();
        dnd::attach_drop_target(
            &button,
            Rc::new(move || Some(destination.clone())),
            on_drop.clone(),
        );
        button.connect_clicked(move |_| load(&uri));
        if let Some(action) = volume_action_button(volume, volume.get_mount(), &window) {
            line.append(&action);
        }
        (line, Some((root.uri().to_string(), button)))
    } else {
        let volume = volume.clone();
        button.connect_clicked(move |_| {
            let for_mount = volume.clone();
            let for_cb = volume.clone();
            let load = load.clone();
            let window = window.clone();
            for_mount.mount(
                gio::MountMountFlags::NONE,
                None::<&gio::MountOperation>,
                gio::Cancellable::NONE,
                move |result| match result {
                    Ok(()) => {
                        if let Some(root) = for_cb.activation_root() {
                            load(&root.uri());
                        }
                    }
                    Err(e) => error_dialog(&window, e.to_string()),
                },
            );
        });
        (line, None)
    }
}

/// Highlights/unhighlights the row: background and label color.
fn set_row_active(button: &gtk::Button, active: bool) {
    if active {
        button.add_css_class("active");
    } else {
        button.remove_css_class("active");
    }
    if let Some(label) = button.child().and_then(|child| find_side_label(&child)) {
        if active {
            label.add_css_class("active");
        } else {
            label.remove_css_class("active");
        }
    }
}

fn find_side_label(widget: &gtk::Widget) -> Option<gtk::Label> {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        if label.has_css_class("side-label") {
            return Some(label.clone());
        }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(label) = find_side_label(&current) {
            return Some(label);
        }
        child = current.next_sibling();
    }
    None
}

fn uri_is_same_or_parent(parent_uri: &str, current_uri: &str) -> bool {
    let parent = gio::File::for_uri(parent_uri);
    let current = gio::File::for_uri(current_uri);
    current.equal(&parent) || current.has_prefix(&parent)
}

fn active_row_index(destinations: &[String], current_uri: &str) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    for (index, destination) in destinations.iter().enumerate() {
        if uri_is_same_or_parent(destination, current_uri)
            && best.is_none_or(|(_, length)| destination.len() > length)
        {
            best = Some((index, destination.len()));
        }
    }
    best.map(|(index, _)| index)
}

fn apply_active_row(rows: &[Rows], active_uri: Option<&str>) -> Option<usize> {
    let mut entries = Vec::new();
    for group in rows {
        entries.extend(group.borrow().iter().cloned());
    }
    let destinations = entries
        .iter()
        .map(|(uri, _)| uri.clone())
        .collect::<Vec<_>>();
    let active_index = active_uri.and_then(|uri| active_row_index(&destinations, uri));
    for (index, (_, button)) in entries.iter().enumerate() {
        set_row_active(button, Some(index) == active_index);
    }
    active_index
}

/// Sidebar widget, dynamic row registries and refreshers.
pub struct Sidebar {
    widget: gtk::ScrolledWindow,
    rows: Vec<Rows>,
    quick_places: [gtk::Button; 3],
    favorites_toggle: gtk::Button,
    favorites_contents: gtk::Box,
    favorites_arrow: gtk::Image,
    refresh_shortcuts: Rc<dyn Fn()>,
    refresh_places: Rc<dyn Fn()>,
    refresh_devices: Rc<dyn Fn()>,
    active_uri: Rc<RefCell<Option<String>>>,
}

impl Sidebar {
    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.widget
    }

    /// Highlights the nearest matching destination, including its descendants.
    pub fn set_active(&self, uri: &str) {
        *self.active_uri.borrow_mut() = Some(uri.to_string());
        let active_index = apply_active_row(&self.rows, Some(uri));
        let shortcut_count = self.rows[0].borrow().len();
        let bookmark_count = self.rows[1].borrow().len();
        let bookmark_range = shortcut_count..shortcut_count + bookmark_count;
        if active_index.is_some_and(|index| bookmark_range.contains(&index)) {
            set_favorites_expanded(
                &self.favorites_toggle,
                &self.favorites_contents,
                &self.favorites_arrow,
                true,
            );
        }
    }

    /// Refreshes translated labels and system rows whose fallback names
    /// are supplied by the application. Real device/bookmark names remain
    /// system data.
    pub fn retranslate(&self) {
        for (button, message_id) in self
            .quick_places
            .iter()
            .zip(["side-home", "side-network", "side-trash"])
        {
            update_nav_row_label(button, &crate::l10n::tr(message_id));
        }
        (self.refresh_shortcuts)();
        (self.refresh_places)();
        (self.refresh_devices)();
    }
}

/// Full sidebar in a ScrolledWindow. Updates itself on
/// mount/unmount (GVolumeMonitor) and on bookmark changes
/// (monitor on `~/.config/gtk-3.0`, so pins made by Nautilus count too).
pub fn build_sidebar(
    load: LoadFn,
    window: adw::ApplicationWindow,
    on_drop: DropHandler,
) -> Sidebar {
    let shortcut_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let bookmark_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let place_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let device_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let rows = vec![
        shortcut_rows.clone(),
        bookmark_rows.clone(),
        place_rows.clone(),
        device_rows.clone(),
    ];
    let rows_for_active = rows.clone();
    let active_uri: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(6)
        .margin_end(6)
        .margin_top(4)
        .margin_bottom(8)
        .build();
    let shortcuts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    outer.append(&shortcuts);

    let (favorites_toggle, favorites_label, favorites_arrow) = favorites_toggle_row();
    let favorites_contents = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(12)
        .build();
    favorites_contents.set_visible(false);
    let favorites_empty = gtk::Label::builder()
        .label(crate::l10n::tr("side-favorites-empty"))
        .halign(gtk::Align::Start)
        .margin_start(36)
        .margin_top(4)
        .margin_bottom(4)
        .css_classes(["dim-label", "caption"])
        .build();
    let quick_places = shortcut_places(
        &load,
        &shortcuts,
        &shortcut_rows,
        &favorites_toggle,
        &favorites_contents,
        &window,
        &on_drop,
    );
    favorites_toggle.connect_clicked({
        let favorites_contents = favorites_contents.clone();
        let favorites_arrow = favorites_arrow.clone();
        move |button| {
            let expanded = !favorites_contents.is_visible();
            set_favorites_expanded(button, &favorites_contents, &favorites_arrow, expanded);
        }
    });

    let places_separator_before = section_separator();
    outer.append(&places_separator_before);
    let places = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    outer.append(&places);
    let places_separator_after = section_separator();
    outer.append(&places_separator_after);

    let devices = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    outer.append(&devices);

    let config_dir = glib::user_config_dir().join("gtk-3.0");
    let _ = std::fs::create_dir_all(&config_dir);
    let bookmark_monitor = gio::File::for_path(&config_dir)
        .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        .ok();
    let refresh_shortcuts: Rc<dyn Fn()> = Rc::new({
        let load = load.clone();
        let rows = bookmark_rows.clone();
        let favorites_contents = favorites_contents.clone();
        let favorites_label = favorites_label.clone();
        let favorites_toggle = favorites_toggle.clone();
        let favorites_empty = favorites_empty.clone();
        let on_drop = on_drop.clone();
        let all_rows = rows_for_active.clone();
        let active_uri = active_uri.clone();
        let _monitor = bookmark_monitor.clone();
        move || {
            let _ = &_monitor;
            let label = crate::l10n::tr("side-favorites");
            let description = crate::l10n::tr("side-favorites-toggle");
            favorites_label.set_text(&label);
            favorites_empty.set_text(&crate::l10n::tr("side-favorites-empty"));
            favorites_toggle.set_tooltip_text(Some(&description));
            favorites_toggle.update_property(&[
                gtk::accessible::Property::Label(&label),
                gtk::accessible::Property::Description(&description),
            ]);
            while let Some(child) = favorites_contents.first_child() {
                favorites_contents.remove(&child);
            }
            rows.borrow_mut().clear();
            let bookmarks = unique_bookmarks(kito_core::bookmarks::read(), &sidebar_place_uris());
            for (name, uri) in bookmarks {
                let button = bookmark_row(&name, &uri, load.clone(), &on_drop);
                add_row(&favorites_contents, &rows, &uri, &button);
            }
            if rows.borrow().is_empty() {
                favorites_contents.append(&favorites_empty);
            }
            let _ = apply_active_row(&all_rows, active_uri.borrow().as_deref());
        }
    });

    if let Some(monitor) = bookmark_monitor {
        let refresh_shortcuts = refresh_shortcuts.clone();
        monitor.connect_changed(move |_, file, _, _| {
            if file
                .basename()
                .is_some_and(|name| name.as_os_str() == "bookmarks")
            {
                refresh_shortcuts();
            }
        });
    }

    let refresh_places: Rc<dyn Fn()> = Rc::new({
        let places = places.clone();
        let load = load.clone();
        let rows = place_rows.clone();
        let on_drop = on_drop.clone();
        let before_separator = places_separator_before.clone();
        let after_separator = places_separator_after.clone();
        let all_rows = rows_for_active.clone();
        let active_uri = active_uri.clone();
        move || {
            while let Some(child) = places.first_child() {
                places.remove(&child);
            }
            rows.borrow_mut().clear();
            xdg_places(&load, &places, &rows, &on_drop);
            let has_places = places.first_child().is_some();
            places.set_visible(has_places);
            before_separator.set_visible(has_places);
            after_separator.set_visible(has_places);
            let _ = apply_active_row(&all_rows, active_uri.borrow().as_deref());
        }
    });

    let refresh_devices: Rc<dyn Fn()> = Rc::new({
        let devices = devices.clone();
        let load = load.clone();
        let window = window.clone();
        let on_drop = on_drop.clone();
        let rows = device_rows.clone();
        let all_rows = rows_for_active.clone();
        let active_uri = active_uri.clone();
        move || {
            while let Some(child) = devices.first_child() {
                devices.remove(&child);
            }
            rows.borrow_mut().clear();
            for volume in gio::VolumeMonitor::get().volumes() {
                let (row, active) = volume_row(&volume, load.clone(), window.clone(), &on_drop);
                devices.append(&row);
                if let Some(active) = active {
                    rows.borrow_mut().push(active);
                }
            }
            // Keep the root filesystem reachable when no removable volumes exist.
            if devices.first_child().is_none() {
                let root = nav_row(
                    &icons::control_icon(&[
                        "drive-harddisk-root-symbolic",
                        "drive-harddisk-symbolic",
                        "drive-harddisk-root",
                        "drive-harddisk",
                        "folder-symbolic",
                        "folder",
                    ]),
                    &crate::l10n::tr("side-filesystem"),
                    load.clone(),
                    "file:///".to_string(),
                    &on_drop,
                );
                add_row(&devices, &rows, "file:///", &root);
            }
            let _ = apply_active_row(&all_rows, active_uri.borrow().as_deref());
        }
    });

    // Make the groups before the first navigation callback can update active state.
    refresh_shortcuts();
    refresh_places();
    refresh_devices();

    let volume_monitor = gio::VolumeMonitor::get();
    for signal in ["volume-added", "volume-removed", "volume-changed"] {
        let refresh = refresh_devices.clone();
        volume_monitor.connect_closure(
            signal,
            false,
            glib::closure_local!(move |_: &gio::VolumeMonitor| {
                refresh();
            }),
        );
    }

    Sidebar {
        widget: gtk::ScrolledWindow::builder()
            .child(&outer)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
        rows,
        quick_places,
        favorites_toggle,
        favorites_contents,
        favorites_arrow,
        refresh_shortcuts,
        refresh_places,
        refresh_devices,
        active_uri,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon_names(icon: gio::ThemedIcon) -> Vec<String> {
        icon.names()
            .into_iter()
            .map(|name| name.to_string())
            .collect()
    }

    #[test]
    fn place_icons_prefer_specific_symbols_then_folder_fallbacks() {
        assert_eq!(
            icon_names(place_icon("folder-download")),
            [
                "folder-download-symbolic",
                "folder-symbolic",
                "folder-download",
                "folder"
            ]
            .map(str::to_string)
        );
        assert_eq!(
            icon_names(place_icon("user-home")),
            [
                "user-home-symbolic",
                "folder-symbolic",
                "user-home",
                "folder"
            ]
            .map(str::to_string)
        );
    }

    #[test]
    fn trash_icons_keep_empty_and_full_states_symbolic() {
        assert_eq!(
            icon_names(trash_icon(false)),
            [
                "user-trash-symbolic",
                "user-trash-full-symbolic",
                "user-trash",
                "user-trash-full"
            ]
            .map(str::to_string)
        );
        assert_eq!(
            icon_names(trash_icon(true)),
            [
                "user-trash-full-symbolic",
                "user-trash-symbolic",
                "user-trash-full",
                "user-trash"
            ]
            .map(str::to_string)
        );
    }

    #[test]
    fn unknown_bookmarks_use_a_symbolic_folder_icon() {
        assert_eq!(
            icon_names(bookmark_icon("smb://example.invalid/share")),
            ["folder-symbolic", "folder"].map(str::to_string)
        );
    }

    #[test]
    fn favorites_hide_standard_place_and_duplicate_bookmarks() {
        let bookmarks = vec![
            (
                "Documents URI".to_string(),
                "file:///home/kerd/Documents".to_string(),
            ),
            (
                "Project".to_string(),
                "file:///home/kerd/Projects".to_string(),
            ),
            (
                "Projects duplicate".to_string(),
                "file:///home/kerd/Projects/".to_string(),
            ),
        ];
        let known = [
            "file:///home/kerd".to_string(),
            "file:///home/kerd/Documents/".to_string(),
        ];

        assert_eq!(
            unique_bookmarks(bookmarks, &known),
            [("Project".to_string(), "file:///home/kerd/Projects".to_string())]
        );
    }

    #[test]
    fn active_row_uses_the_nearest_ancestor_without_matching_siblings() {
        let destinations = [
            "file:///".to_string(),
            "file:///home/kerd".to_string(),
            "file:///home/kerd/Documents".to_string(),
            "file:///home/kerd/Documents/Project%20Files".to_string(),
        ];

        assert_eq!(
            active_row_index(
                &destinations,
                "file:///home/kerd/Documents/Project%20Files/src"
            ),
            Some(3)
        );
        assert_eq!(
            active_row_index(&destinations, "file:///home/kerd/Pictures"),
            Some(1)
        );
        assert_eq!(active_row_index(&destinations, "file:///tmp"), Some(0));
        assert_eq!(active_row_index(&destinations, "trash:///"), None);
    }

    #[test]
    fn active_row_matches_remote_uris_and_preserves_filesystem_root() {
        let destinations = [
            "smb://server/share".to_string(),
            "smb://server/share/projects".to_string(),
            "file:///".to_string(),
        ];

        assert_eq!(
            active_row_index(&destinations, "smb://server/share/projects/2026"),
            Some(1)
        );
        assert_eq!(active_row_index(&destinations, "file:///etc"), Some(2));
        assert_eq!(active_row_index(&destinations, "smb://server/share2"), None);
    }

    #[test]
    fn initial_empty_and_full() {
        let mut state = TrashIconState::default();
        assert!(!state.current());
        let query = state.begin_query();
        assert_eq!(state.apply_result(query, true), Some(true));
        assert!(state.current());

        let mut fresh = TrashIconState::default();
        let query = fresh.begin_query();
        assert_eq!(fresh.apply_result(query, false), None);
        assert!(!fresh.current());
    }

    #[test]
    fn empty_to_full_and_back() {
        let mut state = TrashIconState::default();
        let query = state.begin_query();
        assert_eq!(state.apply_result(query, true), Some(true));
        let query = state.begin_query();
        assert_eq!(state.apply_result(query, false), Some(false));
        assert!(!state.current());
    }

    #[test]
    fn superseded_results_are_ignored() {
        let mut state = TrashIconState::default();
        let old = state.begin_query();
        let live = state.begin_query();
        // Late answer from the superseded query: ignored, stays active.
        assert_eq!(state.apply_result(old, true), None);
        assert!(!state.current());
        assert_eq!(state.apply_result(live, true), Some(true));
        assert!(state.current());
    }

    #[test]
    fn backend_errors_keep_polling_without_wedging() {
        let mut state = TrashIconState::default();
        for _ in 0..5 {
            let query = state.begin_query();
            assert_eq!(state.apply_result(query, false), None);
        }
        assert!(!state.current());
        // Still accepts a real change afterwards.
        let query = state.begin_query();
        assert_eq!(state.apply_result(query, true), Some(true));
    }

    #[test]
    fn only_flips_reach_the_widget() {
        let mut state = TrashIconState::default();
        let mut shown = Vec::new();
        let stale = state.begin_query();
        let live = state.begin_query();
        // Superseded: no widget access.
        if let Some(full) = state.apply_result(stale, true) {
            shown.push(full);
        }
        // Unchanged: no widget access.
        if let Some(full) = state.apply_result(live, false) {
            shown.push(full);
        }
        // Flip: single update.
        let query = state.begin_query();
        if let Some(full) = state.apply_result(query, true) {
            shown.push(full);
        }
        assert_eq!(shown, vec![true]);
    }
}
