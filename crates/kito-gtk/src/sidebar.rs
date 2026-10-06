//! Places sidebar: Places (XDG + bookmarks) + Devices (volumes) + Network.
//! All strings are in English (i18n source, see agents.md).

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
            .label("Remove from Places")
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
            &display_name(&home, "Home"),
            load.clone(),
            format!("file://{}", home.display()),
        );
        add_row(parent, rows, &format!("file://{}", home.display()), &button);
    }
    const PLACES: [(&str, &str, glib::UserDirectory); 6] = [
        ("folder-desktop", "Desktop", glib::UserDirectory::Desktop),
        (
            "folder-documents",
            "Documents",
            glib::UserDirectory::Documents,
        ),
        (
            "folder-download",
            "Downloads",
            glib::UserDirectory::Downloads,
        ),
        ("folder-music", "Music", glib::UserDirectory::Music),
        ("folder-pictures", "Pictures", glib::UserDirectory::Pictures),
        ("folder-videos", "Videos", glib::UserDirectory::Videos),
    ];
    for (icon_name, label, dir) in PLACES {
        let Some(path) = glib::user_special_dir(dir) else {
            continue;
        };
        if !path.is_dir() {
            continue;
        }
        let uri = format!("file://{}", path.display());
        let button = nav_row(
            &place_icon(icon_name),
            &display_name(&path, label),
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

/// `true` if the trash contains at least one entry. If the trash backend
/// is unavailable (no gvfs) it does not block: treated as empty.
fn trash_is_full() -> bool {
    gio::File::for_uri(kito_core::TRASH_URI)
        .enumerate_children(
            "standard::name",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .map(|children| {
            children
                .next_file(gio::Cancellable::NONE)
                .ok()
                .flatten()
                .is_some()
        })
        .unwrap_or(false)
}

fn trash_row(load: LoadFn, window: &adw::ApplicationWindow) -> gtk::Button {
    let button = nav_row(
        &trash_icon(trash_is_full()),
        "Trash",
        load,
        kito_core::TRASH_URI.to_string(),
    );
    // The row updates itself: if a full trash becomes empty
    // (or vice versa) the icon changes without reloading the sidebar.
    {
        let image = button
            .child()
            .and_downcast::<gtk::Box>()
            .and_then(|b| b.first_child())
            .and_downcast::<gtk::Image>();
        if let Some(image) = image {
            let (tx, rx) = std::sync::mpsc::channel::<bool>();
            glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
                let _ = tx.send(trash_is_full());
                glib::ControlFlow::Continue
            });
            let mut last = trash_is_full();
            glib::idle_add_local(move || match rx.try_recv() {
                Ok(full) => {
                    if full != last {
                        last = full;
                        image.set_from_gicon(&trash_icon(full));
                    }
                    glib::ControlFlow::Continue
                }
                Err(_) => glib::ControlFlow::Break,
            });
        }
    }
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_SECONDARY)
        .build();
    let button_weak = button.downgrade();
    let window = window.clone();
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
        let image = gtk::Image::from_gicon(&trash_icon(trash_is_full()));
        image.set_pixel_size(18);
        row.append(&image);
        row.append(
            &gtk::Label::builder()
                .label("Empty Trash…")
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
        .heading("Operation failed")
        .body(body)
        .build();
    dialog.add_response("ok", "Ok");
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
}

impl Sidebar {
    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.widget
    }

    /// Highlights the row matching `uri`, turns the others off.
    pub fn set_active(&self, uri: &str) {
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
    outer.append(&section_header("Places"));
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

    outer.append(&section_header("Devices"));
    let devices = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    outer.append(&devices);

    outer.append(&section_header("Network"));
    let network = gio::ThemedIcon::new("network-workgroup");
    {
        let network_row = nav_row(
            &network,
            "Browse network",
            load.clone(),
            "network:///".to_string(),
        );
        add_row(&outer, &other_rows, "network:///", &network_row);
    }

    let refresh = Rc::new({
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
                    "File System",
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
    }
}
