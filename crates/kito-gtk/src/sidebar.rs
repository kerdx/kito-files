//! Sidebar: shortcuts, XDG folders and mounted volumes.
//! UI labels use the app catalogs; names returned by GIO remain system data.

use crate::preferences::PreferenceStore;
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
fn add_row(
    container: &gtk::Box,
    rows: &Rows,
    uri: &str,
    button: &gtk::Button,
    preferences: &Rc<PreferenceStore>,
) {
    let id = sidebar_id(uri);
    add_row_with_id(container, rows, uri, button, preferences, &id);
}

fn add_row_with_id(
    container: &gtk::Box,
    rows: &Rows,
    uri: &str,
    button: &gtk::Button,
    preferences: &Rc<PreferenceStore>,
    id: &str,
) {
    let hidden = is_sidebar_hidden(&preferences.snapshot(), id);
    if hidden {
        button.set_visible(false);
    }
    if id != "builtin:trash" && !button.has_css_class("freedesktop-bookmark") {
        attach_hide_menu(button, id, preferences);
    }
    rows.borrow_mut().push((uri.to_string(), button.clone()));
    container.append(button);
}

fn bookmark_sidebar_id(uri: &str) -> String {
    format!("bookmark:{uri}")
}

fn custom_sidebar_id(uri: &str) -> String {
    format!("custom:{uri}")
}

fn sidebar_id(uri: &str) -> String {
    match uri {
        "network:///" => "builtin:network".into(),
        kito_core::TRASH_URI => "builtin:trash".into(),
        "file:///" => "builtin:filesystem-root".into(),
        _ if gio::File::for_path(glib::home_dir()).uri().as_str() == uri => "builtin:home".into(),
        _ => uri.to_string(),
    }
}

fn volume_sidebar_id(
    uuid: Option<&str>,
    device_id: Option<&str>,
    drive_id: Option<&str>,
    fallback_name: &str,
    fallback_drive: &str,
) -> String {
    uuid.map(|value| format!("volume:uuid:{value}"))
        .or_else(|| device_id.map(|value| format!("volume:device:{value}")))
        .or_else(|| drive_id.map(|value| format!("volume:drive:{value}:{fallback_name}")))
        // Some backends expose no stable ID. Use volume/drive labels, which
        // remain stable across mount cycles, rather than the activation URI.
        // If two unidentifiable volumes expose identical metadata, GIO gives
        // us no persistent way to distinguish them.
        .unwrap_or_else(|| {
            format!(
                "volume:fallback:{}:{fallback_name}:{}:{fallback_drive}",
                fallback_name.len(),
                fallback_drive.len()
            )
        })
}

fn attach_hide_menu(button: &gtk::Button, id: &str, preferences: &Rc<PreferenceStore>) {
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_SECONDARY)
        .build();
    let weak = button.downgrade();
    let id = id.to_string();
    let preferences = Rc::downgrade(preferences);
    gesture.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let (Some(button), Some(preferences)) = (weak.upgrade(), preferences.upgrade()) else {
            return;
        };
        let popover = gtk::Popover::new();
        let hide = gtk::Button::builder()
            .label(crate::l10n::tr("side-hide-in-kito"))
            .has_frame(false)
            .build();
        let id = id.clone();
        let popover_weak = popover.downgrade();
        let button_weak = button.downgrade();
        hide.connect_clicked(move |_| {
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
            if let Err(error) = preferences.set_sidebar_hidden(&id, true) {
                eprintln!("{}", localized_error("side-settings-error", &error));
                if let Some(button) = button_weak.upgrade() {
                    if let Some(window) = button.root().and_downcast::<adw::ApplicationWindow>() {
                        error_dialog(&window, localized_error("side-settings-error", &error));
                    }
                }
            }
        });
        popover.set_child(Some(&hide));
        popover.set_parent(&button);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    button.add_controller(gesture);
}

fn has_visible_child(container: &gtk::Box) -> bool {
    let mut child = container.first_child();
    while let Some(widget) = child {
        if widget.is_visible() {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn is_sidebar_hidden(preferences: &crate::preferences::model::Preferences, id: &str) -> bool {
    preferences
        .hidden_sidebar_ids
        .iter()
        .any(|hidden| hidden == id)
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
    let target = uri.clone();
    let button = nav_row_with_action(icon, label, move || load(&target));
    let destination = uri;
    dnd::attach_drop_target(
        &button,
        Rc::new(move || Some(destination.clone())),
        on_drop.clone(),
    );
    button
}

fn nav_row_with_action(
    icon: &impl IsA<gio::Icon>,
    label: &str,
    activate: impl Fn() + 'static,
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
    button.connect_clicked(move |_| activate());
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

fn section_heading(label: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(label)
        .halign(gtk::Align::Start)
        .margin_start(12)
        .margin_end(8)
        .margin_top(6)
        .margin_bottom(2)
        .css_classes(["dim-label", "caption"])
        .build()
}

fn favorites_toggle_row(
    load: LoadFn,
    on_drop: &DropHandler,
) -> (gtk::Button, gtk::Label, gtk::Image) {
    let label_text = crate::l10n::tr("side-favorites");
    let description = label_text.clone();
    let button = gtk::Button::builder()
        .has_frame(false)
        .css_classes(["side-row", "favorites-toggle"])
        .tooltip_text(&description)
        .build();
    button.update_property(&[
        gtk::accessible::Property::Label(&label_text),
        gtk::accessible::Property::Description(&description),
    ]);

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

    button.set_child(Some(&row));
    let uri = kito_core::FAVORITES_URI.to_string();
    dnd::attach_drop_target_with_actions(
        &button,
        Rc::new({
            let uri = uri.clone();
            move || Some(uri.clone())
        }),
        on_drop.clone(),
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::LINK,
    );
    button.connect_clicked(move |_| load(&uri));
    let arrow = gtk::Image::new();
    (button, label, arrow)
}

fn bookmarks_group_row(contents: &gtk::Box) -> (gtk::Button, gtk::Label) {
    let label_text = crate::l10n::tr("side-bookmarks");
    let button = gtk::Button::builder()
        .has_frame(false)
        .css_classes(["side-row", "bookmarks-toggle"])
        .tooltip_text(&label_text)
        .build();
    button.update_property(&[
        gtk::accessible::Property::Label(&label_text),
        gtk::accessible::Property::Description(&label_text),
    ]);
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    let icon = gtk::Image::from_gicon(&icons::control_icon(&[
        "folder-bookmark-symbolic",
        "bookmarks-symbolic",
        "bookmark-new-symbolic",
        "folder-symbolic",
    ]));
    icon.set_pixel_size(18);
    row.append(&icon);
    let label = gtk::Label::builder()
        .label(&label_text)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .css_classes(["side-label"])
        .build();
    row.append(&label);
    let arrow = gtk::Image::from_gicon(&icons::control_icon(&["pan-down-symbolic", "pan-down"]));
    arrow.set_pixel_size(12);
    row.append(&arrow);
    button.set_child(Some(&row));
    contents.set_visible(false);
    let contents = contents.clone();
    button.connect_clicked(move |button| {
        let expanded = !contents.is_visible();
        contents.set_visible(expanded);
        arrow.set_from_gicon(&icons::control_icon(if expanded {
            &["pan-down-symbolic", "pan-down"]
        } else {
            &["pan-end-symbolic", "pan-end"]
        }));
        button.update_state(&[gtk::accessible::State::Expanded(Some(expanded))]);
    });
    (button, label)
}

fn bookmark_row(
    name: &str,
    uri: &str,
    load: LoadFn,
    on_drop: &DropHandler,
    window: &adw::ApplicationWindow,
) -> gtk::Button {
    let target_uri = uri.to_string();
    let window_weak = window.downgrade();
    let activate_uri = target_uri.clone();
    let activate = move || {
        let file = gio::File::for_uri(&activate_uri);
        let target = activate_uri.clone();
        let load = load.clone();
        let window = window_weak.clone();
        file.query_info_async(
            "standard::type",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(info) if bookmark_file_type_is_directory(info.file_type()) => load(&target),
                Ok(_) => launch_bookmark_file(&target, &window),
                Err(error) => present_bookmark_open_error(&window, &error),
            },
        );
    };
    let button = nav_row_with_action(&bookmark_icon(uri), name, activate);
    let remove_window_weak = window.downgrade();
    let weak_button = button.downgrade();
    let destination = target_uri.clone();
    let on_drop = on_drop.clone();
    gio::File::for_uri(uri).query_info_async(
        "standard::type",
        gio::FileQueryInfoFlags::NONE,
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        move |result| {
            if result.is_ok_and(|info| bookmark_file_type_is_directory(info.file_type())) {
                if let Some(button) = weak_button.upgrade() {
                    dnd::attach_drop_target(
                        &button,
                        Rc::new(move || Some(destination.clone())),
                        on_drop,
                    );
                }
            }
        },
    );
    update_bookmark_icon(&button, &target_uri);
    button.add_css_class("freedesktop-bookmark");
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_SECONDARY)
        .build();
    let uri = uri.to_string();
    gesture.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(button) = gesture.widget().and_downcast::<gtk::Button>() else {
            return;
        };
        let popover = gtk::Popover::new();
        let remove = gtk::Button::builder()
            .label(crate::l10n::tr("side-remove-bookmark"))
            .has_frame(false)
            .build();
        remove.connect_clicked({
            let popover = popover.clone();
            let uri = uri.clone();
            let window = remove_window_weak.clone();
            move |_| {
                popover.popdown();
                if let Err(error) = kito_core::bookmarks::unpin(&uri) {
                    eprintln!("unpin bookmark: {error}");
                    if let Some(window) = window.upgrade() {
                        error_dialog(&window, error.to_string());
                    }
                }
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

/// Observe compatible drag hover over the sidebar, but accept drops only on
/// the transient button itself. The observer stays under the pointer while it
/// moves from the empty Places area to the newly revealed button, and never
/// negotiates or consumes a drop, so folder-row MOVE targets remain independent.
fn install_add_bookmark_drop_target(
    hover_area: &gtk::Box,
    places: &gtk::Box,
    button: &gtk::Button,
    on_drop: &DropHandler,
) {
    button.set_visible(false);
    let button_weak = button.downgrade();
    let places_weak = places.downgrade();
    let drag_motion = gtk::DropControllerMotion::new();
    drag_motion.connect_enter({
        let button = button_weak.clone();
        let places = places_weak.clone();
        move |controller, _, _| {
            let accepts_bookmark_drag = controller.drop().is_some_and(|drop| {
                let formats = drop.formats();
                (formats.contains_type(gtk::gdk::FileList::static_type())
                    || formats.contain_mime_type("text/uri-list"))
                    && drop
                        .actions()
                        .intersects(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::LINK)
            });
            if accepts_bookmark_drag {
                if let Some(button) = button.upgrade() {
                    button.set_visible(true);
                }
                if let Some(places) = places.upgrade() {
                    places.set_visible(true);
                }
            }
        }
    });
    drag_motion.connect_motion({
        let button = button_weak.clone();
        move |controller, _, _| {
            let accepts_bookmark_drag = controller.drop().is_some_and(|drop| {
                let formats = drop.formats();
                (formats.contains_type(gtk::gdk::FileList::static_type())
                    || formats.contain_mime_type("text/uri-list"))
                    && drop
                        .actions()
                        .intersects(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::LINK)
            });
            if accepts_bookmark_drag {
                if let Some(button) = button.upgrade() {
                    button.set_visible(true);
                }
            }
        }
    });
    drag_motion.connect_leave({
        let button = button_weak.clone();
        let places = places_weak;
        move |_| {
            if let Some(button) = button.upgrade() {
                button.set_visible(false);
            }
            if let Some(places) = places.upgrade() {
                places.set_visible(has_visible_child(&places));
            }
        }
    });
    hover_area.add_controller(drag_motion);

    let drop_handler = on_drop.clone();
    let drop_button = button.downgrade();
    let bookmark_drop: DropHandler = Rc::new(move |uris, destination, action, internal, drop| {
        if let Some(button) = drop_button.upgrade() {
            button.set_visible(false);
        }
        drop_handler(uris, destination, action, internal, drop);
    });
    dnd::attach_drop_target_with_actions_and_hover(
        button,
        Rc::new(|| Some(kito_core::ADD_BOOKMARK_URI.to_string())),
        bookmark_drop,
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::LINK,
        None,
    );
}

fn bookmark_file_type_is_directory(file_type: gio::FileType) -> bool {
    file_type == gio::FileType::Directory
}

fn launch_bookmark_file(uri: &str, window: &glib::WeakRef<adw::ApplicationWindow>) {
    let uri = uri.to_string();
    let window = window.clone();
    gio::AppInfo::launch_default_for_uri_async(
        &uri,
        None::<&gio::AppLaunchContext>,
        None::<&gio::Cancellable>,
        move |result| {
            if let Err(error) = result {
                present_bookmark_open_error(&window, &error);
            }
        },
    );
}

fn present_bookmark_open_error(
    window: &glib::WeakRef<adw::ApplicationWindow>,
    error: &glib::Error,
) {
    if let Some(window) = window.upgrade() {
        let dialog = adw::AlertDialog::builder()
            .heading(crate::l10n::tr("error-open-file"))
            .body(crate::l10n::tr_with_one(
                "error-open-file-detail",
                "error",
                &error.to_string(),
            ))
            .build();
        dialog.add_response("ok", &crate::l10n::tr("dialog-ok"));
        dialog.present(Some(&window));
    } else {
        eprintln!("open bookmark target: {error}");
    }
}

fn update_bookmark_icon(button: &gtk::Button, uri: &str) {
    let Some(image) = button
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|row| row.first_child())
        .and_downcast::<gtk::Image>()
    else {
        return;
    };
    let image = image.downgrade();
    gio::File::for_uri(uri).query_info_async(
        "standard::icon",
        gio::FileQueryInfoFlags::NONE,
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        move |result| {
            if let (Ok(info), Some(image)) = (result, image.upgrade()) {
                if let Some(icon) = info.icon() {
                    image.set_from_gicon(&icon);
                    image.set_pixel_size(18);
                }
            }
        },
    );
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

fn sidebar_place_uris() -> Vec<String> {
    let mut places = vec![gio::File::for_path(glib::home_dir()).uri().to_string()];
    for (_, _, directory) in PLACES {
        if let Some(path) = glib::user_special_dir(directory).filter(|path| path.is_dir()) {
            places.push(gio::File::for_path(&path).uri().to_string());
        }
    }
    places
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

#[allow(clippy::too_many_arguments)]
fn shortcut_places(
    load: &LoadFn,
    parent: &gtk::Box,
    rows: &Rows,
    favorites_toggle: &gtk::Button,
    _favorite_rows: &gtk::Box,
    window: &adw::ApplicationWindow,
    on_drop: &DropHandler,
    preferences: &Rc<PreferenceStore>,
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
    add_row(parent, rows, &home_uri, &home_button, preferences);

    add_row_with_id(
        parent,
        rows,
        kito_core::FAVORITES_URI,
        favorites_toggle,
        preferences,
        "builtin:favorites",
    );

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
    add_row(parent, rows, "network:///", &network_button, preferences);

    let trash_button = trash_row(load.clone(), window, on_drop, preferences);
    add_row(
        parent,
        rows,
        kito_core::TRASH_URI,
        &trash_button,
        preferences,
    );
    [home_button, network_button, trash_button]
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

fn xdg_places(
    load: &LoadFn,
    parent: &gtk::Box,
    rows: &Rows,
    on_drop: &DropHandler,
    preferences: &Rc<PreferenceStore>,
) {
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
        add_row(parent, rows, &uri, &button, preferences);
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

fn trash_row(
    load: LoadFn,
    window: &adw::ApplicationWindow,
    on_drop: &DropHandler,
    preferences: &Rc<PreferenceStore>,
) -> gtk::Button {
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
    let preferences = preferences.clone();
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
        let hide = gtk::Button::builder()
            .label(crate::l10n::tr("side-hide-in-kito"))
            .has_frame(false)
            .build();
        let preferences = preferences.clone();
        let popover_weak = popover.downgrade();
        let button_weak = button.downgrade();
        hide.connect_clicked(move |_| {
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
            if let Err(error) = preferences.set_sidebar_hidden("builtin:trash", true) {
                eprintln!("{}", localized_error("side-settings-error", &error));
                if let Some(button) = button_weak.upgrade() {
                    if let Some(window) = button.root().and_downcast::<adw::ApplicationWindow>() {
                        error_dialog(&window, localized_error("side-settings-error", &error));
                    }
                }
            }
        });
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
        menu.append(&empty);
        menu.append(&hide);
        popover.set_child(Some(&menu));
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

fn localized_error(message: &str, error: &impl std::fmt::Display) -> String {
    crate::l10n::tr(message).replace("{ $error }", &error.to_string())
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
    preferences: &Rc<PreferenceStore>,
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
    // Prefer stable backend identifiers; exclude activation URIs because they
    // can change between mount and unmount.
    let fallback_name = volume.name();
    let device_id = volume.identifier("unix-device");
    let drive = volume.drive();
    let drive_id = drive
        .as_ref()
        .and_then(|drive| drive.identifier("unix-device"));
    let fallback_drive = drive
        .map(|drive| drive.name().to_string())
        .unwrap_or_default();
    let stable_id = volume_sidebar_id(
        volume.uuid().as_deref(),
        device_id.as_deref(),
        drive_id.as_deref(),
        &fallback_name,
        &fallback_drive,
    );
    if preferences
        .snapshot()
        .hidden_sidebar_ids
        .contains(&stable_id)
    {
        button.set_visible(false);
    }
    attach_hide_menu(&button, &stable_id, preferences);

    let line = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(2)
        .build();
    line.append(&button);
    line.set_visible(button.is_visible());

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

fn active_row_index_visible(destinations: &[(String, bool)], current_uri: &str) -> Option<usize> {
    let visible = destinations
        .iter()
        .filter(|(_, is_visible)| *is_visible)
        .map(|(uri, _)| uri.clone())
        .collect::<Vec<_>>();
    active_row_index(&visible, current_uri)
}

fn apply_active_row(rows: &[Rows], active_uri: Option<&str>) -> Option<usize> {
    let mut entries = Vec::new();
    let mut destinations = Vec::new();
    for group in rows {
        for (uri, button) in group.borrow().iter() {
            let visible = button.is_visible();
            destinations.push((uri.clone(), visible));
            if visible {
                entries.push((uri.clone(), button.clone()));
            } else {
                set_row_active(button, false);
            }
        }
    }
    let active_index = active_uri.and_then(|uri| active_row_index_visible(&destinations, uri));
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
    custom_heading: gtk::Label,
    refresh_shortcuts: Rc<dyn Fn()>,
    refresh_places: Rc<dyn Fn()>,
    refresh_devices: Rc<dyn Fn()>,
    refresh_custom: Rc<dyn Fn()>,
    active_uri: Rc<RefCell<Option<String>>>,
}

impl Sidebar {
    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.widget
    }

    /// Highlights the nearest matching destination, including its descendants.
    pub fn set_active(&self, uri: &str) {
        *self.active_uri.borrow_mut() = Some(uri.to_string());
        let _ = apply_active_row(&self.rows, Some(uri));
    }

    pub fn refresh_preferences(&self) {
        (self.refresh_shortcuts)();
        (self.refresh_places)();
        (self.refresh_devices)();
        (self.refresh_custom)();
    }

    /// Refreshes translated labels and system rows whose fallback names
    /// are supplied by the application. Real device/bookmark names remain
    /// system data.
    pub fn retranslate(&self) {
        self.custom_heading
            .set_text(&crate::l10n::tr("side-custom"));
        for (button, message_id) in
            self.quick_places
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
    preferences: Rc<PreferenceStore>,
) -> Sidebar {
    let shortcut_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let bookmark_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let place_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let custom_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let device_rows: Rows = Rc::new(RefCell::new(Vec::new()));
    let rows = vec![
        shortcut_rows.clone(),
        bookmark_rows.clone(),
        place_rows.clone(),
        custom_rows.clone(),
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

    let (favorites_toggle, favorites_label, _favorites_arrow) =
        favorites_toggle_row(load.clone(), &on_drop);
    attach_hide_menu(&favorites_toggle, "builtin:favorites", &preferences);
    favorites_toggle.set_visible(
        !preferences
            .snapshot()
            .hidden_sidebar_ids
            .iter()
            .any(|id| id == "builtin:favorites"),
    );
    let favorites_contents = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let quick_places = shortcut_places(
        &load,
        &shortcuts,
        &shortcut_rows,
        &favorites_toggle,
        &favorites_contents,
        &window,
        &on_drop,
        &preferences,
    );
    let bookmark_contents = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(12)
        .build();
    let (bookmark_group, bookmark_group_label) = bookmarks_group_row(&bookmark_contents);
    shortcuts.insert_child_after(&bookmark_group, Some(&favorites_toggle));
    shortcuts.insert_child_after(&bookmark_contents, Some(&bookmark_group));

    let places_separator_before = section_separator();
    outer.append(&places_separator_before);
    let places = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    outer.append(&places);
    let places_separator_after = section_separator();
    outer.append(&places_separator_after);
    places_separator_after.set_visible(true);

    let custom_heading = section_heading(&crate::l10n::tr("side-custom"));
    outer.append(&custom_heading);
    let custom_locations = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    outer.append(&custom_locations);

    let custom_separator = section_separator();
    outer.append(&custom_separator);
    let devices = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    outer.append(&devices);

    let add_bookmark = gtk::Button::with_label(&crate::l10n::tr("side-add-bookmark"));
    add_bookmark.add_css_class("side-row");
    add_bookmark.update_property(&[gtk::accessible::Property::Label(&crate::l10n::tr(
        "side-add-bookmark",
    ))]);
    install_add_bookmark_drop_target(&outer, &places, &add_bookmark, &on_drop);

    let config_dir = glib::user_config_dir().join("gtk-3.0");
    let _ = std::fs::create_dir_all(&config_dir);
    let bookmark_monitor = gio::File::for_path(&config_dir)
        .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        .ok();
    let refresh_shortcuts: Rc<dyn Fn()> = Rc::new({
        let rows = bookmark_rows.clone();
        let quick_places = quick_places.clone();
        let bookmark_contents = bookmark_contents.clone();
        let bookmark_group_label = bookmark_group_label.clone();
        let bookmark_group = bookmark_group.clone();
        let load = load.clone();
        let bookmark_window = window.clone();
        let on_drop = on_drop.clone();
        let favorites_label = favorites_label.clone();
        let favorites_toggle = favorites_toggle.clone();
        let all_rows = rows_for_active.clone();
        let active_uri = active_uri.clone();
        let preferences = preferences.clone();
        let _monitor = bookmark_monitor.clone();
        move || {
            let _ = &_monitor;
            let snapshot = preferences.snapshot();
            for (button, id) in
                quick_places
                    .iter()
                    .zip(["builtin:home", "builtin:network", "builtin:trash"])
            {
                button.set_visible(!is_sidebar_hidden(&snapshot, id));
            }
            let label = crate::l10n::tr("side-favorites");
            favorites_label.set_text(&label);
            favorites_toggle.set_tooltip_text(Some(&label));
            favorites_toggle.update_property(&[gtk::accessible::Property::Label(&label)]);
            favorites_toggle.set_visible(!is_sidebar_hidden(&snapshot, "builtin:favorites"));
            let bookmark_label = crate::l10n::tr("side-bookmarks");
            bookmark_group_label.set_text(&bookmark_label);
            bookmark_group.set_tooltip_text(Some(&bookmark_label));
            bookmark_group.update_property(&[
                gtk::accessible::Property::Label(&bookmark_label),
                gtk::accessible::Property::Description(&bookmark_label),
            ]);
            let bookmarks_expanded = bookmark_contents.is_visible();
            while let Some(child) = bookmark_contents.first_child() {
                bookmark_contents.remove(&child);
            }
            rows.borrow_mut().clear();
            let bookmarks = unique_bookmarks(kito_core::bookmarks::read(), &sidebar_place_uris());
            for (name, uri) in bookmarks {
                let button = bookmark_row(&name, &uri, load.clone(), &on_drop, &bookmark_window);
                button.set_visible(!is_sidebar_hidden(&snapshot, &bookmark_sidebar_id(&uri)));
                rows.borrow_mut().push((uri, button.clone()));
                bookmark_contents.append(&button);
            }
            bookmark_contents.set_visible(bookmarks_expanded);
            let _ = apply_active_row(&all_rows, active_uri.borrow().as_deref());
        }
    });

    if let Some(monitor) = bookmark_monitor {
        let refresh_shortcuts = refresh_shortcuts.clone();
        let preferences = preferences.clone();
        monitor.connect_changed(move |_, file, other, _| {
            let is_bookmarks = file
                .basename()
                .is_some_and(|name| name.as_os_str() == "bookmarks")
                || other.is_some_and(|other| {
                    other
                        .basename()
                        .is_some_and(|name| name.as_os_str() == "bookmarks")
                });
            if is_bookmarks {
                refresh_shortcuts();
                preferences.notify_bookmarks_changed();
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
        let add_bookmark = add_bookmark.clone();
        let all_rows = rows_for_active.clone();
        let active_uri = active_uri.clone();
        let preferences = preferences.clone();
        move || {
            while let Some(child) = places.first_child() {
                places.remove(&child);
            }
            rows.borrow_mut().clear();
            xdg_places(&load, &places, &rows, &on_drop, &preferences);
            let add_bookmark_label = crate::l10n::tr("side-add-bookmark");
            add_bookmark.set_label(&add_bookmark_label);
            add_bookmark.update_property(&[gtk::accessible::Property::Label(&add_bookmark_label)]);
            places.append(&add_bookmark);
            let has_places = has_visible_child(&places);
            places.set_visible(has_places);
            before_separator.set_visible(has_places);
            after_separator.set_visible(true);
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
        let preferences = preferences.clone();
        move || {
            while let Some(child) = devices.first_child() {
                devices.remove(&child);
            }
            rows.borrow_mut().clear();
            for volume in gio::VolumeMonitor::get().volumes() {
                let (row, active) = volume_row(
                    &volume,
                    load.clone(),
                    window.clone(),
                    &on_drop,
                    &preferences,
                );
                devices.append(&row);
                if let Some(active) = active {
                    rows.borrow_mut().push(active);
                }
            }
            // Keep the root filesystem reachable when no removable volumes exist.
            if !has_visible_child(&devices) {
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
                add_row(&devices, &rows, "file:///", &root, &preferences);
            }
            let _ = apply_active_row(&all_rows, active_uri.borrow().as_deref());
        }
    });

    let refresh_custom: Rc<dyn Fn()> = Rc::new({
        let box_widget = custom_locations.clone();
        let rows = custom_rows.clone();
        let heading = custom_heading.clone();
        let separator = custom_separator.clone();
        let load = load.clone();
        let drop_handler = on_drop.clone();
        let prefs = preferences.clone();
        let all_rows = rows_for_active.clone();
        let active_uri = active_uri.clone();
        move || {
            while let Some(child) = box_widget.first_child() {
                box_widget.remove(&child);
            }
            rows.borrow_mut().clear();
            for uri in prefs.snapshot().custom_sidebar_uris {
                let file = gio::File::for_uri(&uri);
                let name = file
                    .basename()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| uri.clone());
                let row = nav_row(
                    &icons::control_icon(&["folder-symbolic", "folder"]),
                    &name,
                    load.clone(),
                    uri.clone(),
                    &drop_handler,
                );
                let hidden = prefs
                    .snapshot()
                    .hidden_sidebar_ids
                    .iter()
                    .any(|id| id == &custom_sidebar_id(&uri));
                row.set_visible(!hidden);
                attach_hide_menu(&row, &custom_sidebar_id(&uri), &prefs);
                rows.borrow_mut().push((uri.clone(), row.clone()));
                box_widget.append(&row);
            }
            let visible = has_visible_child(&box_widget);
            box_widget.set_visible(visible);
            heading.set_visible(visible);
            separator.set_visible(visible);
            let _ = apply_active_row(&all_rows, active_uri.borrow().as_deref());
        }
    });

    // Make the groups before the first navigation callback can update active state.
    refresh_shortcuts();
    refresh_places();
    refresh_devices();
    refresh_custom();

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
        custom_heading,
        refresh_shortcuts,
        refresh_places,
        refresh_devices,
        refresh_custom,
        active_uri,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_click_classification_uses_gio_type_not_filename() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("looks-like-a-file.txt");
        let file = temp.path().join("looks-like-a-folder.dir");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(&file, b"file").unwrap();
        let directory_type = gio::File::for_path(&directory)
            .query_info(
                "standard::type",
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .unwrap()
            .file_type();
        let file_type = gio::File::for_path(&file)
            .query_info(
                "standard::type",
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .unwrap()
            .file_type();
        assert!(bookmark_file_type_is_directory(directory_type));
        assert!(!bookmark_file_type_is_directory(file_type));
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn add_bookmark_hover_observer_and_link_target_are_on_distinct_widgets() {
        gtk::test_synced(|| {
            let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let places = gtk::Box::new(gtk::Orientation::Vertical, 0);
            places.set_visible(false); // no XDG places are present
            let add_bookmark = gtk::Button::with_label("Add bookmark");
            places.append(&add_bookmark);
            sidebar.append(&places);
            let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            let handler: DropHandler = Rc::new(|_, _, _, _, drop| {
                drop.finish(gtk::gdk::DragAction::empty());
            });

            install_add_bookmark_drop_target(&sidebar, &places, &add_bookmark, &handler);

            assert!(!add_bookmark.is_visible());
            let sidebar_controllers = sidebar.observe_controllers();
            assert!((0..sidebar_controllers.n_items()).any(|index| {
                sidebar_controllers
                    .item(index)
                    .and_downcast::<gtk::DropControllerMotion>()
                    .is_some()
            }));
            assert!((0..sidebar_controllers.n_items()).all(|index| {
                sidebar_controllers
                    .item(index)
                    .and_downcast::<gtk::DropTargetAsync>()
                    .is_none()
            }));
            let button_controllers = add_bookmark.observe_controllers();
            let link_target = (0..button_controllers.n_items())
                .filter_map(|index| {
                    button_controllers
                        .item(index)
                        .and_downcast::<gtk::DropTargetAsync>()
                })
                .next()
                .expect("only the visible add-bookmark row receives the drop target");
            assert_eq!(
                link_target.actions(),
                gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::LINK
            );
            let separator_controllers = separator.observe_controllers();
            assert!((0..separator_controllers.n_items()).all(|index| {
                separator_controllers
                    .item(index)
                    .and_downcast::<gtk::DropTargetAsync>()
                    .is_none()
            }));
        });
    }

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
    fn sidebar_ids_are_stable_and_volume_fallbacks_are_not_shared() {
        assert_eq!(sidebar_id("network:///"), "builtin:network");
        assert_eq!(sidebar_id(kito_core::TRASH_URI), "builtin:trash");
        assert_eq!(sidebar_id("file:///"), "builtin:filesystem-root");
        assert_eq!(
            volume_sidebar_id(
                Some("uuid-1"),
                Some("/dev/sda1"),
                Some("/dev/sda"),
                "Disk",
                "External drive"
            ),
            "volume:uuid:uuid-1"
        );
        assert_eq!(
            volume_sidebar_id(
                None,
                Some("/dev/sdb1"),
                Some("/dev/sdb"),
                "Share",
                "Network drive"
            ),
            "volume:device:/dev/sdb1"
        );
        assert_eq!(
            volume_sidebar_id(None, None, Some("/dev/sdc"), "Backup", "External drive"),
            "volume:drive:/dev/sdc:Backup"
        );
        let fallback = volume_sidebar_id(None, None, None, "Backup", "External drive");
        assert_ne!(
            fallback,
            volume_sidebar_id(None, None, None, "Media", "External drive")
        );
        let hidden = crate::preferences::model::Preferences {
            hidden_sidebar_ids: vec![
                "builtin:home".into(),
                "builtin:favorites".into(),
                "builtin:network".into(),
                "builtin:trash".into(),
                "file:///places".into(),
                "volume:uuid:uuid-1".into(),
                "builtin:filesystem-root".into(),
            ],
            ..Default::default()
        };
        for id in [
            "builtin:home",
            "builtin:favorites",
            "builtin:network",
            "builtin:trash",
            "file:///places",
            "volume:uuid:uuid-1",
            "builtin:filesystem-root",
        ] {
            assert!(is_sidebar_hidden(&hidden, id), "{id}");
        }
        assert!(!is_sidebar_hidden(&hidden, "file:///other"));
    }

    #[test]
    fn bookmark_and_custom_ids_are_namespaced_for_the_same_uri() {
        let uri = "smb://host/share/item";
        let bookmark = bookmark_sidebar_id(uri);
        let custom = custom_sidebar_id(uri);
        assert_eq!(bookmark, "bookmark:smb://host/share/item");
        assert_eq!(custom, "custom:smb://host/share/item");
        assert_ne!(bookmark, custom);
        assert_eq!(uri, "smb://host/share/item");
    }

    #[test]
    fn hidden_rows_do_not_steal_the_active_sidebar_highlight() {
        let destinations = [
            ("file:///home/kerd".to_string(), true),
            ("file:///home/kerd/Documenti".to_string(), false),
        ];
        assert_eq!(
            active_row_index_visible(&destinations, "file:///home/kerd/Documenti/Progetto"),
            Some(0)
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
            [(
                "Project".to_string(),
                "file:///home/kerd/Projects".to_string()
            )]
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
