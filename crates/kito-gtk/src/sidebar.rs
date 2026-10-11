//! Sidebar: shortcuts, shared bookmarks and mounted volumes.
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

const PLACES: [(&str, glib::UserDirectory); 5] = [
    ("folder-documents", glib::UserDirectory::Documents),
    ("folder-music", glib::UserDirectory::Music),
    ("folder-pictures", glib::UserDirectory::Pictures),
    ("folder-videos", glib::UserDirectory::Videos),
    ("folder-download", glib::UserDirectory::Downloads),
];

const BOOKMARK_ROW_MIME: &str = "application/x-kito-bookmark-row";
const MAX_BOOKMARK_ROW_PAYLOAD_BYTES: usize = 16 * 1024;

fn bookmark_drop_after(y: f64, height: i32) -> bool {
    y >= f64::from(height) / 2.0
}

fn bookmark_row_content_formats() -> gtk::gdk::ContentFormats {
    gtk::gdk::ContentFormats::builder()
        .add_mime_type(BOOKMARK_ROW_MIME)
        .build()
}

fn bookmark_row_provider(uri: &str) -> gtk::gdk::ContentProvider {
    gtk::gdk::ContentProvider::for_bytes(
        BOOKMARK_ROW_MIME,
        &glib::Bytes::from_owned(uri.as_bytes().to_vec()),
    )
}

fn read_bookmark_row_stream(
    stream: gio::InputStream,
    mut accumulated: Vec<u8>,
    callback: Box<dyn FnOnce(Result<Vec<u8>, glib::Error>)>,
) {
    let remaining = MAX_BOOKMARK_ROW_PAYLOAD_BYTES + 1 - accumulated.len();
    let next_stream = stream.clone();
    stream.read_bytes_async(
        remaining,
        glib::Priority::DEFAULT,
        gio::Cancellable::NONE,
        move |result| match result {
            Err(error) => callback(Err(error)),
            Ok(bytes) if bytes.is_empty() => callback(Ok(accumulated)),
            Ok(bytes) => {
                accumulated.extend_from_slice(bytes.as_ref());
                if accumulated.len() > MAX_BOOKMARK_ROW_PAYLOAD_BYTES {
                    callback(Err(glib::Error::new(
                        gio::IOErrorEnum::NoSpace,
                        "bookmark row payload exceeds the maximum size",
                    )));
                } else {
                    read_bookmark_row_stream(next_stream, accumulated, callback);
                }
            }
        },
    );
}

fn clear_bookmark_drop_marker(button: &gtk::Button) {
    button.remove_css_class("bookmark-drop-before");
    button.remove_css_class("bookmark-drop-after");
}

fn set_bookmark_drop_marker(button: &gtk::Button, after: bool) {
    clear_bookmark_drop_marker(button);
    button.add_css_class(if after {
        "bookmark-drop-after"
    } else {
        "bookmark-drop-before"
    });
}

/// Registers the row and adds it to the container.
fn add_row(
    container: &gtk::Box,
    rows: &Rows,
    uri: &str,
    button: &gtk::Button,
    _preferences: &Rc<PreferenceStore>,
) {
    rows.borrow_mut().push((uri.to_string(), button.clone()));
    container.append(button);
}

fn has_visible_child(container: &gtk::Box) -> bool {
    let mut child = container.first_child();
    while let Some(widget) = child {
        // `WidgetExt::is_visible()` also depends on the ancestors, so it stays
        // false when this container is hidden. Inspect the child's own GTK
        // `visible` flag so a populated group can reveal its parent again.
        if widget.property::<bool>("visible") {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn append_bookmarks_group(places: &gtk::Box, contents: &gtk::Box) {
    places.append(contents);
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

fn bookmarks_group_row(contents: &gtk::Box) {
    contents.set_visible(false);
}

fn bookmark_row(
    name: &str,
    uri: &str,
    load: LoadFn,
    on_drop: &DropHandler,
    window: &adw::ApplicationWindow,
    preferences: &Rc<PreferenceStore>,
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
    let bookmark_name = name.to_string();
    let preferences = Rc::downgrade(preferences);
    gesture.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(button) = gesture.widget().and_downcast::<gtk::Button>() else {
            return;
        };
        let popover = gtk::Popover::new();
        let rename = gtk::Button::builder()
            .label(crate::l10n::tr("side-rename-bookmark"))
            .has_frame(false)
            .build();
        let rename_uri = uri.clone();
        let rename_name = bookmark_name.clone();
        let rename_window = remove_window_weak.clone();
        let rename_preferences = preferences.clone();
        let rename_button = button.downgrade();
        let rename_popover = popover.downgrade();
        rename.connect_clicked(move |_| {
            if let Some(popover) = rename_popover.upgrade() {
                popover.popdown();
            }
            if let (Some(button), Some(window)) = (rename_button.upgrade(), rename_window.upgrade())
            {
                show_bookmark_rename(
                    &button,
                    &window,
                    &rename_uri,
                    &rename_name,
                    rename_preferences.clone(),
                );
            }
        });
        let remove = gtk::Button::builder()
            .label(crate::l10n::tr("side-remove-bookmark"))
            .has_frame(false)
            .build();
        remove.connect_clicked({
            let popover = popover.clone();
            let uri = uri.clone();
            let window = remove_window_weak.clone();
            let preferences = preferences.clone();
            move |_| {
                popover.popdown();
                if let Err(error) = kito_core::bookmarks::unpin(&uri) {
                    eprintln!("unpin bookmark: {error}");
                    if let Some(window) = window.upgrade() {
                        error_dialog(&window, error.to_string());
                    }
                } else if let Some(preferences) = preferences.upgrade() {
                    preferences.notify_bookmarks_changed();
                }
            }
        });
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
        menu.append(&rename);
        menu.append(&remove);
        popover.set_child(Some(&menu));
        popover.set_parent(&button);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    button.add_controller(gesture);
    button
}

fn show_bookmark_rename(
    _button: &gtk::Button,
    window: &adw::ApplicationWindow,
    uri: &str,
    current_name: &str,
    preferences: std::rc::Weak<PreferenceStore>,
) {
    let dialog = adw::Dialog::builder()
        .title(crate::l10n::tr("side-rename-bookmark"))
        .content_width(360)
        .build();
    let entry = gtk::Entry::builder()
        .text(current_name)
        .placeholder_text(crate::l10n::tr("side-bookmark-name"))
        .build();
    let error_label = gtk::Label::builder()
        .visible(false)
        .css_classes(["error"])
        .wrap(true)
        .build();
    let save = gtk::Button::builder()
        .label(crate::l10n::tr("side-bookmark-save"))
        .css_classes(["suggested-action"])
        .build();
    let cancel = gtk::Button::builder()
        .label(crate::l10n::tr("dialog-cancel"))
        .build();
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&save);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.append(&entry);
    content.append(&error_label);
    content.append(&buttons);
    dialog.set_child(Some(&content));
    cancel.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            dialog.close();
        }
    });
    let uri = uri.to_string();
    save.connect_clicked({
        let dialog = dialog.clone();
        let entry = entry.clone();
        let error_label = error_label.clone();
        let preferences = preferences.clone();
        move |_| {
            let name = entry.text().trim().to_string();
            if name.is_empty() {
                error_label.set_text(&crate::l10n::tr("side-bookmark-name-empty"));
                error_label.set_visible(true);
                return;
            }
            if kito_core::bookmarks::read()
                .iter()
                .any(|(other_name, other_uri)| other_uri != &uri && other_name == &name)
            {
                error_label.set_text(&crate::l10n::tr("side-bookmark-name-duplicate"));
                error_label.set_visible(true);
                return;
            }
            match kito_core::bookmarks::rename(&uri, &name) {
                Ok(_) => {
                    dialog.close();
                    if let Some(preferences) = preferences.upgrade() {
                        preferences.notify_bookmarks_changed();
                    }
                }
                Err(error) => {
                    error_label.set_text(&crate::l10n::tr_with_one(
                        "side-bookmark-rename-error",
                        "error",
                        &error.to_string(),
                    ));
                    error_label.set_visible(true);
                }
            }
        }
    });
    dialog.present(Some(window));
    glib::idle_add_local_once({
        let entry = entry.clone();
        move || {
            entry.grab_focus();
            entry.select_region(0, -1);
        }
    });
}

fn attach_bookmark_reorder(
    button: &gtk::Button,
    uri: &str,
    window: &adw::ApplicationWindow,
    preferences: &Rc<PreferenceStore>,
) {
    let source = gtk::DragSource::new();
    source.set_actions(gtk::gdk::DragAction::MOVE);
    let uri = uri.to_string();
    let target_uri = uri.clone();
    source.connect_prepare(move |_, _, _| Some(bookmark_row_provider(&uri)));
    let weak_button = button.downgrade();
    source.connect_drag_begin(move |source, _| {
        if let Some(button) = weak_button.upgrade() {
            let paintable = gtk::WidgetPaintable::new(Some(&button));
            source.set_icon(Some(&paintable), 16, 16);
        }
    });
    button.add_controller(source);

    // This private MIME is intentionally disjoint from file-list/URI targets;
    // reordering a bookmark must never be interpreted as a filesystem drop.
    let target = gtk::DropTargetAsync::new(
        Some(bookmark_row_content_formats()),
        gtk::gdk::DragAction::MOVE,
    );
    target.connect_accept(|_, drop| {
        drop.formats().contain_mime_type(BOOKMARK_ROW_MIME)
            && drop.actions().contains(gtk::gdk::DragAction::MOVE)
    });
    let weak_window = window.downgrade();
    let preferences = Rc::downgrade(preferences);
    let weak_button = button.downgrade();
    target.connect_drag_enter({
        let button = weak_button.clone();
        move |_, _, _, y| {
            if let Some(button) = button.upgrade() {
                set_bookmark_drop_marker(&button, bookmark_drop_after(y, button.height()));
            }
            gtk::gdk::DragAction::MOVE
        }
    });
    target.connect_drag_motion({
        let button = weak_button.clone();
        move |_, _, _, y| {
            if let Some(button) = button.upgrade() {
                set_bookmark_drop_marker(&button, bookmark_drop_after(y, button.height()));
            }
            gtk::gdk::DragAction::MOVE
        }
    });
    target.connect_drag_leave(move |_, _| {
        if let Some(button) = weak_button.upgrade() {
            clear_bookmark_drop_marker(&button);
        }
    });
    let weak_button = button.downgrade();
    target.connect_drop(move |_, drop, _, y| {
        if let Some(button) = weak_button.upgrade() {
            clear_bookmark_drop_marker(&button);
        }
        let after = weak_button
            .upgrade()
            .is_some_and(|button| bookmark_drop_after(y, button.height()));
        let finish_drop = drop.clone();
        let target_uri = target_uri.clone();
        let preferences = preferences.clone();
        let weak_window = weak_window.clone();
        drop.read_async(
            &[BOOKMARK_ROW_MIME],
            glib::Priority::DEFAULT,
            gio::Cancellable::NONE,
            move |result| {
                let (stream, _) = match result {
                    Ok(stream) => stream,
                    Err(error) => {
                        if let Some(window) = weak_window.upgrade() {
                            error_dialog(&window, error.to_string());
                        }
                        finish_drop.finish(gtk::gdk::DragAction::empty());
                        return;
                    }
                };
                let weak_window = weak_window.clone();
                read_bookmark_row_stream(
                    stream,
                    Vec::new(),
                    Box::new(move |result| {
                        let bytes = match result {
                            Ok(bytes) => bytes,
                            Err(error) => {
                                if let Some(window) = weak_window.upgrade() {
                                    error_dialog(&window, error.to_string());
                                }
                                finish_drop.finish(gtk::gdk::DragAction::empty());
                                return;
                            }
                        };
                        let Ok(source_uri) = std::str::from_utf8(bytes.as_ref()) else {
                            finish_drop.finish(gtk::gdk::DragAction::empty());
                            return;
                        };
                        match kito_core::bookmarks::reorder_relative_to(
                            source_uri,
                            &target_uri,
                            after,
                        ) {
                            Ok(true) => {
                                if let Some(preferences) = preferences.upgrade() {
                                    preferences.notify_bookmarks_changed();
                                }
                                finish_drop.finish(gtk::gdk::DragAction::MOVE);
                            }
                            Ok(false) => finish_drop.finish(gtk::gdk::DragAction::empty()),
                            Err(error) => {
                                if let Some(window) = weak_window.upgrade() {
                                    error_dialog(&window, error.to_string());
                                }
                                finish_drop.finish(gtk::gdk::DragAction::empty());
                            }
                        }
                    }),
                );
            },
        );
        true
    });
    button.add_controller(target);
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
            if let Some(image) = image.upgrade() {
                if let Ok(info) = result {
                    if let Some(icon) = info.icon() {
                        image.set_from_gicon(&icon);
                        image.set_pixel_size(18);
                    }
                } else {
                    image.set_from_gicon(&icons::control_icon(&[
                        "dialog-warning-symbolic",
                        "dialog-warning",
                        "folder-symbolic",
                        "folder",
                    ]));
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
    for (icon_name, directory) in PLACES {
        if glib::user_special_dir(directory)
            .is_some_and(|special| special.as_path() == path.as_path())
        {
            return place_icon(icon_name);
        }
    }
    generic_folder()
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

    add_row(
        parent,
        rows,
        kito_core::FAVORITES_URI,
        favorites_toggle,
        preferences,
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

fn unique_bookmarks(bookmarks: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut known = Vec::new();
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
    _preferences: &Rc<PreferenceStore>,
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
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
        menu.append(&empty);
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
    _preferences: &Rc<PreferenceStore>,
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
        place_rows.clone(),
        bookmark_rows.clone(),
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
        .build();
    bookmarks_group_row(&bookmark_contents);

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
        let bookmark_contents = bookmark_contents.clone();
        let before_separator = places_separator_before.clone();
        let places = places.clone();
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
            let label = crate::l10n::tr("side-favorites");
            favorites_label.set_text(&label);
            favorites_toggle.set_tooltip_text(Some(&label));
            favorites_toggle.update_property(&[gtk::accessible::Property::Label(&label)]);
            while let Some(child) = bookmark_contents.first_child() {
                bookmark_contents.remove(&child);
            }
            rows.borrow_mut().clear();
            let bookmarks = unique_bookmarks(kito_core::bookmarks::read());
            for (name, uri) in bookmarks {
                let button = bookmark_row(
                    &name,
                    &uri,
                    load.clone(),
                    &on_drop,
                    &bookmark_window,
                    &preferences,
                );
                // Freedesktop bookmarks are never hidden by Kito sidebar preferences.
                button.set_visible(true);
                attach_bookmark_reorder(&button, &uri, &bookmark_window, &preferences);
                rows.borrow_mut().push((uri, button.clone()));
                bookmark_contents.append(&button);
            }
            bookmark_contents.set_visible(!rows.borrow().is_empty());
            let has_places = has_visible_child(&places);
            places.set_visible(has_places);
            before_separator.set_visible(has_places);
            let _ = apply_active_row(&all_rows, active_uri.borrow().as_deref());
        }
    });

    if let Some(monitor) = bookmark_monitor {
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
                preferences.notify_bookmarks_changed();
            }
        });
    }
    let refresh_weak = Rc::downgrade(&refresh_shortcuts);
    preferences.subscribe_bookmarks(Rc::new(move || {
        if let Some(refresh) = refresh_weak.upgrade() {
            refresh();
        }
    }));

    let refresh_places: Rc<dyn Fn()> = Rc::new({
        let places = places.clone();
        let rows = place_rows.clone();
        let before_separator = places_separator_before.clone();
        let after_separator = places_separator_after.clone();
        let add_bookmark = add_bookmark.clone();
        let bookmark_contents = bookmark_contents.clone();
        let all_rows = rows_for_active.clone();
        let active_uri = active_uri.clone();
        move || {
            while let Some(child) = places.first_child() {
                places.remove(&child);
            }
            rows.borrow_mut().clear();
            let add_bookmark_label = crate::l10n::tr("side-add-bookmark");
            add_bookmark.set_label(&add_bookmark_label);
            add_bookmark.update_property(&[gtk::accessible::Property::Label(&add_bookmark_label)]);
            places.append(&add_bookmark);
            // Keep the transient drop target before the shared bookmark list.
            append_bookmarks_group(&places, &bookmark_contents);
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
        let window = window.downgrade();
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
                row.set_visible(true);
                // Fallback rows remain reachable when automatic migration of
                // old Kito locations failed. Removing one only edits Kito's
                // legacy settings; it never unpins a Freedesktop bookmark.
                let gesture = gtk::GestureClick::builder()
                    .button(gtk::gdk::BUTTON_SECONDARY)
                    .build();
                let row_weak = row.downgrade();
                let window_weak = window.clone();
                let prefs = prefs.clone();
                let legacy_uri = uri.clone();
                gesture.connect_pressed(move |gesture, _, x, y| {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    let Some(row) = row_weak.upgrade() else {
                        return;
                    };
                    let popover = gtk::Popover::new();
                    popover.add_css_class("ctx-menu");
                    let remove = gtk::Button::builder()
                        .label(crate::l10n::tr("side-remove-legacy-location"))
                        .has_frame(false)
                        .build();
                    let popover_weak = popover.downgrade();
                    let prefs = prefs.clone();
                    let uri = legacy_uri.clone();
                    let window = window_weak.clone();
                    remove.connect_clicked(move |_| {
                        if let Some(popover) = popover_weak.upgrade() {
                            popover.popdown();
                        }
                        if let Err(error) = prefs.remove_sidebar_uri(&uri) {
                            if let Some(window) = window.upgrade() {
                                error_dialog(&window, error.to_string());
                            }
                        }
                    });
                    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    menu.append(&remove);
                    popover.set_child(Some(&menu));
                    popover.set_parent(&row);
                    popover
                        .set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                    popover.popup();
                });
                row.add_controller(gesture);
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
    fn bookmark_reorder_format_is_private_and_position_uses_row_halves() {
        let provider = bookmark_row_provider("file:///tmp/bookmark");
        let formats = provider.formats();
        assert!(formats.contain_mime_type(BOOKMARK_ROW_MIME));
        assert!(!formats.contain_mime_type("text/plain"));
        assert!(!formats.contain_mime_type("text/uri-list"));
        assert!(!formats.contains_type(gtk::gdk::FileList::static_type()));

        let target_formats = bookmark_row_content_formats();
        assert!(target_formats.contain_mime_type(BOOKMARK_ROW_MIME));
        assert!(!target_formats.contain_mime_type("text/uri-list"));
        assert!(!target_formats.contains_type(gtk::gdk::FileList::static_type()));
        assert!(!bookmark_drop_after(49.9, 100));
        assert!(bookmark_drop_after(50.0, 100));
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn bookmark_reorder_drop_target_is_separate_from_directory_file_target() {
        gtk::test_synced(|| {
            let app = adw::Application::new(
                Some("org.example.KitoSidebarDndProbe"),
                gio::ApplicationFlags::NON_UNIQUE,
            );
            app.register(None::<&gio::Cancellable>).unwrap();
            let window = adw::ApplicationWindow::new(&app);
            let temporary = tempfile::tempdir().unwrap();
            let folder = temporary.path().join("bookmark-folder");
            std::fs::create_dir(&folder).unwrap();
            let uri = gio::File::for_path(folder).uri().to_string();
            let preferences = PreferenceStore::at_path(temporary.path().join("settings.conf"));
            let on_drop: DropHandler = Rc::new(|_, _, _, _, drop| {
                drop.finish(gtk::gdk::DragAction::empty());
            });
            let row = bookmark_row(
                "Bookmark",
                &uri,
                Rc::new(|_| {}),
                &on_drop,
                &window,
                &preferences,
            );
            attach_bookmark_reorder(&row, &uri, &window, &preferences);
            dnd::attach_drop_target(&row, Rc::new(move || Some(uri.clone())), on_drop);

            let controllers = row.observe_controllers();
            let targets = (0..controllers.n_items())
                .filter_map(|index| {
                    controllers
                        .item(index)
                        .and_downcast::<gtk::DropTargetAsync>()
                })
                .collect::<Vec<_>>();
            assert_eq!(targets.len(), 2);
            assert!(targets.iter().any(|target| {
                target
                    .formats()
                    .is_some_and(|formats| formats.contain_mime_type(BOOKMARK_ROW_MIME))
                    && target.actions() == gtk::gdk::DragAction::MOVE
            }));
            assert!(targets.iter().any(|target| {
                target
                    .formats()
                    .is_some_and(|formats| formats.contains_type(gtk::gdk::FileList::static_type()))
                    && target.actions().contains(gtk::gdk::DragAction::COPY)
            }));
            assert!((0..controllers.n_items()).all(|index| {
                controllers
                    .item(index)
                    .and_downcast::<gtk::DropTarget>()
                    .is_none()
            }));
        });
    }

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
    fn bookmarks_follow_transient_cta_without_a_section_heading() {
        gtk::test_synced(|| {
            let places = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let add_bookmark = gtk::Button::with_label("Add bookmark");
            let contents = gtk::Box::new(gtk::Orientation::Vertical, 0);
            bookmarks_group_row(&contents);
            places.append(&add_bookmark);
            append_bookmarks_group(&places, &contents);

            assert!(contents.parent().is_some());
            assert!(has_visible_child(&places));
            assert_eq!(places.first_child().unwrap(), add_bookmark);
            assert_eq!(add_bookmark.next_sibling().unwrap(), contents);
            places.set_visible(has_visible_child(&places));
            assert!(places.is_visible());
            assert!(!contents.is_visible());

            contents.set_visible(true);
            contents.append(&gtk::Button::with_label("Folder"));
            let has_places = has_visible_child(&places);
            places.set_visible(has_places);
            assert!(has_places);
            assert!(places.is_visible());
        });
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
    fn freedesktop_bookmarks_keep_xdg_locations_and_deduplicate_records() {
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
        assert_eq!(
            unique_bookmarks(bookmarks),
            [
                (
                    "Documents URI".to_string(),
                    "file:///home/kerd/Documents".to_string()
                ),
                (
                    "Project".to_string(),
                    "file:///home/kerd/Projects".to_string()
                )
            ]
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
