//! Places sidebar: Places (XDG + bookmarks) + Devices (volumes) + Network.
//! UI labels use the app catalogs; names returned by GIO remain system data.

use adw::prelude::*;
use gtk::{gio, glib};
use std::{cell::RefCell, rc::Rc};

type LoadFn = Rc<dyn Fn(&str)>;
/// Sidebar rows for highlighting the current folder.
type Rows = Rc<RefCell<Vec<(String, gtk::Button)>>>;

/// Normalized URI: comparison without trailing slashes (`file:///` included).
fn normalize(uri: &str) -> String {
    uri.trim_end_matches('/').to_string()
}

/// Registers the row and adds it to the container.
fn add_row(container: &gtk::Box, rows: &Rows, uri: &str, button: &gtk::Button) {
    rows.borrow_mut().push((normalize(uri), button.clone()));
    container.append(button);
}

/// Clickable row with icon + label.
fn nav_row(icon: &impl IsA<gio::Icon>, label: &str, load: LoadFn, uri: String) -> gtk::Button {
    let button = gtk::Button::builder()
        .has_frame(false)
        .css_classes(["side-row"])
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
        // 22px: at this size Papirus/Breeze have color icons,
        // at 16px they are monochrome.
        let image = gtk::Image::from_gicon(icon);
        image.set_pixel_size(22);
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
    button.connect_clicked(move |_| load(&uri));
    button
}

fn section_header(title: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(title)
        .halign(gtk::Align::Start)
        .margin_start(14)
        .margin_end(14)
        .margin_top(14)
        .margin_bottom(4)
        .css_classes(["caption", "dim-label"])
        .build()
}

fn row_label(button: &gtk::Button) -> Option<gtk::Label> {
    button
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|row| row.last_child())
        .and_downcast::<gtk::Label>()
}

/// Bookmark row with right-click to remove it. The sidebar reloads
/// on its own via monitor on the file (see `build_sidebar`).
fn bookmark_row(name: &str, uri: &str, load: LoadFn) -> gtk::Button {
    let button = nav_row(
        &gio::ThemedIcon::new("user-bookmarks"),
        name,
        load,
        uri.to_string(),
    );
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
    gio::ThemedIcon::from_names(&[primary, "folder"])
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

fn xdg_places(load: &LoadFn, parent: &gtk::Box, rows: &Rows, window: &adw::ApplicationWindow) {
    // Home is not an XDG user dir: taken separately.
    let home = glib::home_dir();
    {
        let button = nav_row(
            &place_icon("user-home"),
            &display_name(&home, &crate::l10n::tr("side-home")),
            load.clone(),
            format!("file://{}", home.display()),
        );
        add_row(parent, rows, &format!("file://{}", home.display()), &button);
    }
    const PLACES: [(&str, &str, glib::UserDirectory); 6] = [
        (
            "folder-desktop",
            "side-desktop",
            glib::UserDirectory::Desktop,
        ),
        (
            "folder-documents",
            "side-documents",
            glib::UserDirectory::Documents,
        ),
        (
            "folder-download",
            "side-downloads",
            glib::UserDirectory::Downloads,
        ),
        ("folder-music", "side-music", glib::UserDirectory::Music),
        (
            "folder-pictures",
            "side-pictures",
            glib::UserDirectory::Pictures,
        ),
        ("folder-videos", "side-videos", glib::UserDirectory::Videos),
    ];
    for (icon_name, label_id, dir) in PLACES {
        let Some(path) = glib::user_special_dir(dir) else {
            continue;
        };
        if !path.is_dir() {
            continue;
        }
        let uri = format!("file://{}", path.display());
        let button = nav_row(
            &place_icon(icon_name),
            &display_name(&path, &crate::l10n::tr(label_id)),
            load.clone(),
            uri.clone(),
        );
        add_row(parent, rows, &uri, &button);
    }
    // Trash: end of fixed places, before the user bookmarks.
    {
        let button = trash_row(load.clone(), window);
        add_row(parent, rows, kito_core::TRASH_URI, &button);
    }
    for (name, uri) in kito_core::bookmarks::read() {
        let button = bookmark_row(&name, &uri, load.clone());
        add_row(parent, rows, &uri, &button);
    }
}

/// Trash row in Places: opens `trash:///` and has the "Empty Trash…"
/// menu on right-click (activates `win.empty-trash`, which asks first).
/// Trash icon: `user-trash-full` if there is something inside, otherwise
/// the empty variant. Papirus has both.
fn trash_icon(full: bool) -> gio::ThemedIcon {
    if full {
        gio::ThemedIcon::new("user-trash-full")
    } else {
        gio::ThemedIcon::new("user-trash")
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

fn trash_row(load: LoadFn, window: &adw::ApplicationWindow) -> gtk::Button {
    let button = nav_row(
        &trash_icon(false),
        &crate::l10n::tr("side-trash"),
        load,
        kito_core::TRASH_URI.to_string(),
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

/// Volume row: if mounted it opens, otherwise it tries to mount first.
fn volume_row(volume: &gio::Volume, load: LoadFn, window: adw::ApplicationWindow) -> gtk::Button {
    let button = gtk::Button::builder()
        .has_frame(false)
        .css_classes(["side-row"])
        .build();
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    row.append(&gtk::Image::from_gicon(&volume.icon()));
    row.append(
        &gtk::Label::builder()
            .label(volume.name())
            .halign(gtk::Align::Start)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["side-label"])
            .build(),
    );
    button.set_child(Some(&row));

    if let Some(root) = volume.activation_root() {
        let uri = root.uri().to_string();
        button.connect_clicked(move |_| load(&uri));
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
    }
    button
}

/// Highlights/unhighlights the row: background and label color.
fn set_row_active(button: &gtk::Button, active: bool) {
    if active {
        button.add_css_class("active");
    } else {
        button.remove_css_class("active");
    }
    // The label is a child of the button: the class must be set by hand.
    if let Some(box_) = button.child().and_downcast::<gtk::Box>() {
        if let Some(label) = box_.last_child().and_downcast::<gtk::Label>() {
            if active {
                label.add_css_class("active");
            } else {
                label.remove_css_class("active");
            }
        }
    }
}

/// Sidebar: widget + row registry to highlight the open folder.
/// `places` is recreated on every refresh (cleared and repopulated),
/// `other` holds the static rows (network).
pub struct Sidebar {
    widget: gtk::ScrolledWindow,
    places: Rows,
    other: Rows,
    places_title: gtk::Label,
    devices_title: gtk::Label,
    network_title: gtk::Label,
    network_label: gtk::Label,
    refresh_places: Rc<dyn Fn()>,
    refresh_devices: Rc<dyn Fn()>,
    active_uri: RefCell<Option<String>>,
}

impl Sidebar {
    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.widget
    }

    /// Highlights the row matching `uri`, turns the others off.
    pub fn set_active(&self, uri: &str) {
        *self.active_uri.borrow_mut() = Some(uri.to_string());
        let target = normalize(uri);
        for (key, button) in self
            .places
            .borrow()
            .iter()
            .chain(self.other.borrow().iter())
        {
            set_row_active(button, *key == target);
        }
    }

    /// Refreshes translated labels and system rows whose fallback names
    /// are supplied by the application. Real device/bookmark names remain
    /// system data.
    pub fn retranslate(&self) {
        self.places_title.set_text(&crate::l10n::tr("side-places"));
        self.devices_title
            .set_text(&crate::l10n::tr("side-devices"));
        self.network_title
            .set_text(&crate::l10n::tr("side-network"));
        self.network_label
            .set_text(&crate::l10n::tr("side-browse-network"));
        (self.refresh_places)();
        (self.refresh_devices)();
        let active_uri = self.active_uri.borrow().clone();
        if let Some(uri) = active_uri {
            self.set_active(&uri);
        }
    }
}

/// Full sidebar in a ScrolledWindow. Updates itself on
/// mount/unmount (GVolumeMonitor) and on bookmark changes
/// (monitor on `~/.config/gtk-3.0`, so pins made by Nautilus count too).
pub fn build_sidebar(load: LoadFn, window: adw::ApplicationWindow) -> Sidebar {
    let places_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let other_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(6)
        .margin_end(6)
        .margin_top(4)
        .margin_bottom(8)
        .build();
    let places_title = section_header(&crate::l10n::tr("side-places"));
    outer.append(&places_title);
    let places = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    outer.append(&places);

    let config_dir = glib::user_config_dir().join("gtk-3.0");
    let _ = std::fs::create_dir_all(&config_dir);
    let monitor = gio::File::for_path(&config_dir)
        .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        .ok();
    let refresh_places: Rc<dyn Fn()> = Rc::new({
        let places = places.clone();
        let load = load.clone();
        let rows = places_rows.clone();
        let window = window.clone();
        // Kept on purpose: monitor -> handler -> refresh -> monitor loop,
        // living as long as the process.
        let _monitor = monitor.clone();
        move || {
            let _ = &_monitor;
            while let Some(child) = places.first_child() {
                places.remove(&child);
            }
            // Rows are recreated: registry cleared and repopulated.
            rows.borrow_mut().clear();
            xdg_places(&load, &places, &rows, &window);
        }
    });
    refresh_places();
    if let Some(monitor) = monitor {
        let refresh_places = refresh_places.clone();
        monitor.connect_changed(move |_, file, _, _| {
            if file
                .basename()
                .is_some_and(|n| n.as_os_str() == "bookmarks")
            {
                refresh_places();
            }
        });
    }

    let devices_title = section_header(&crate::l10n::tr("side-devices"));
    outer.append(&devices_title);
    let devices = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    outer.append(&devices);

    let network_title = section_header(&crate::l10n::tr("side-network"));
    outer.append(&network_title);
    let network = gio::ThemedIcon::new("network-workgroup");
    let network_row = nav_row(
        &network,
        &crate::l10n::tr("side-browse-network"),
        load.clone(),
        "network:///".to_string(),
    );
    let network_label = row_label(&network_row).expect("network row has a label");
    add_row(&outer, &other_rows, "network:///", &network_row);

    let refresh: Rc<dyn Fn()> = Rc::new({
        let devices = devices.clone();
        let load = load.clone();
        let window = window.clone();
        move || {
            while let Some(child) = devices.first_child() {
                devices.remove(&child);
            }
            for volume in gio::VolumeMonitor::get().volumes() {
                devices.append(&volume_row(&volume, load.clone(), window.clone()));
            }
            // No volumes: the root filesystem is reachable anyway.
            if devices.first_child().is_none() {
                devices.append(&nav_row(
                    &gio::ThemedIcon::from_names(&[
                        "drive-harddisk-root",
                        "drive-harddisk",
                        "folder",
                    ]),
                    &crate::l10n::tr("side-filesystem"),
                    load.clone(),
                    "file:///".to_string(),
                ));
            }
        }
    });
    for signal in ["volume-added", "volume-removed", "volume-changed"] {
        let refresh = refresh.clone();
        gio::VolumeMonitor::get().connect_closure(
            signal,
            false,
            glib::closure_local!(move |_: &gio::VolumeMonitor| {
                refresh();
            }),
        );
    }
    refresh();

    Sidebar {
        widget: gtk::ScrolledWindow::builder()
            .child(&outer)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
        places: places_rows,
        other: other_rows,
        places_title,
        devices_title,
        network_title,
        network_label,
        refresh_places,
        refresh_devices: refresh,
        active_uri: RefCell::new(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
