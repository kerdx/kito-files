//! Classic views: icons, compact, details. One row GObject + factory
//! on a shared ListStore; no widget subclassing.

use glib::subclass::prelude::ObjectSubclassIsExt as _;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;

use crate::dnd::{self, DropHandler};
use crate::preferences::model::OpenItems;
pub use crate::preferences::model::ViewMode;

#[derive(Clone, Copy)]
pub struct ViewDisplay {
    pub icon_zoom: u8,
    pub show_size: bool,
    pub show_type: bool,
    pub show_modified: bool,
}

/// Right-click on a row: object + point + anchor widget.
/// The handler selects the object and opens the menu.
pub type SecondaryHandler = Rc<dyn Fn(&FileObject, f64, f64, &gtk::Widget)>;
pub type MiddleHandler = Rc<dyn Fn(&FileObject)>;
pub type DragPrepareHandler = Rc<dyn Fn(&FileObject) -> Option<dnd::PreparedDrag>>;

pub struct ViewHandlers<'a> {
    pub on_secondary: &'a SecondaryHandler,
    pub on_middle: &'a MiddleHandler,
    pub on_drag_prepare: &'a DragPrepareHandler,
    pub on_drop: &'a DropHandler,
    pub on_external_move: &'a dnd::ExternalMoveHandler,
}

type HeaderLabels = Rc<std::cell::RefCell<Vec<(String, glib::WeakRef<gtk::Label>)>>>;

/// Binds right-click to the row for the current object. Called on every
/// bind (rows are recycled): removes the previous gesture.
fn set_secondary(row: &impl IsA<gtk::Widget>, obj: &FileObject, on_secondary: &SecondaryHandler) {
    let controllers = row.observe_controllers();
    let mut kept = 0;
    while kept < controllers.n_items() {
        let Some(controller) = controllers
            .item(kept)
            .and_downcast::<gtk::EventController>()
        else {
            kept += 1;
            continue;
        };
        if controller
            .clone()
            .downcast::<gtk::GestureClick>()
            .is_ok_and(|click| click.button() == gtk::gdk::BUTTON_SECONDARY)
        {
            row.remove_controller(&controller);
        } else {
            kept += 1;
        }
    }
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_SECONDARY)
        .build();
    let obj = obj.clone();
    let on_secondary = on_secondary.clone();
    gesture.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(source) = gesture.widget() else {
            return;
        };
        on_secondary(&obj, x, y, &source);
    });
    row.add_controller(gesture);
}

fn set_middle(row: &impl IsA<gtk::Widget>, obj: &FileObject, on_middle: &MiddleHandler) {
    let controllers = row.observe_controllers();
    let mut kept = 0;
    while kept < controllers.n_items() {
        let Some(controller) = controllers
            .item(kept)
            .and_downcast::<gtk::EventController>()
        else {
            kept += 1;
            continue;
        };
        if controller
            .clone()
            .downcast::<gtk::GestureClick>()
            .is_ok_and(|click| click.button() == gtk::gdk::BUTTON_MIDDLE)
        {
            row.remove_controller(&controller);
        } else {
            kept += 1;
        }
    }
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_MIDDLE)
        .build();
    let obj = obj.clone();
    let on_middle = on_middle.clone();
    gesture.connect_pressed(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if obj.is_dir() {
            on_middle(&obj);
        }
    });
    row.add_controller(gesture);
}

mod imp {
    use glib::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct FileObject {
        pub name: RefCell<String>,
        pub uri: RefCell<String>,
        pub is_dir: Cell<bool>,
        pub size: Cell<i64>,
        pub modified: Cell<Option<i64>>,
        pub content_type: RefCell<Option<String>>,
        pub icon: RefCell<Option<gio::Icon>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileObject {
        const NAME: &'static str = "KitoFilesFileObject";
        type Type = super::FileObject;
    }

    impl ObjectImpl for FileObject {}
}

glib::wrapper! {
    pub struct FileObject(ObjectSubclass<imp::FileObject>);
}

impl FileObject {
    pub fn new(entry: &kito_core::Entry) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        *imp.name.borrow_mut() = entry.name.clone();
        *imp.uri.borrow_mut() = entry.uri.clone();
        imp.is_dir.set(entry.is_dir);
        imp.size.set(entry.size);
        imp.modified.set(entry.modified);
        *imp.content_type.borrow_mut() = entry.content_type.clone();
        // Serialized in the worker; parsed here on the main thread.
        // Unparsable values fall back to the generic icon in the factories.
        *imp.icon.borrow_mut() = entry
            .icon
            .as_deref()
            .and_then(|s| gio::Icon::for_string(s).ok());
        obj
    }

    pub fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }

    pub fn uri(&self) -> String {
        self.imp().uri.borrow().clone()
    }

    pub fn is_dir(&self) -> bool {
        self.imp().is_dir.get()
    }

    pub fn size(&self) -> i64 {
        self.imp().size.get()
    }

    pub fn modified(&self) -> Option<i64> {
        self.imp().modified.get()
    }

    pub fn type_label(&self) -> String {
        if self.is_dir() {
            return crate::l10n::tr("props-folder");
        }
        match self.imp().content_type.borrow().as_deref() {
            Some(mime) => gio::content_type_get_description(mime).to_string(),
            None => crate::l10n::tr("props-file"),
        }
    }

    pub fn icon(&self) -> Option<gio::Icon> {
        self.imp().icon.borrow().clone()
    }
}

/// 1234567 -> "1.2 MB", -1 (folders) -> "—".
pub fn human_size(bytes: i64) -> String {
    if bytes < 0 {
        return "—".to_string();
    }
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn obj_of(list_item: &gtk::ListItem) -> FileObject {
    list_item.item().and_downcast::<FileObject>().unwrap()
}

/// Content fallback only. File and folder icons in the main views keep the
/// normal GIO theme icons; do not route them through interface-control helpers.
fn fallback_icon(is_dir: bool) -> gio::ThemedIcon {
    gio::ThemedIcon::new(if is_dir { "folder" } else { "text-x-generic" })
}

/// Icon + name row (used by compact and details).
fn name_factory(
    pixel_size: i32,
    on_secondary: &SecondaryHandler,
    on_middle: &MiddleHandler,
    on_drag_prepare: &DragPrepareHandler,
    on_drop: &DropHandler,
    on_external_move: &dnd::ExternalMoveHandler,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, list_item| {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        row.add_css_class("file-item");
        // 22px: at 16px Papirus/Breeze are monochrome.
        let image = gtk::Image::new();
        image.set_pixel_size(pixel_size);
        row.append(&image);
        row.append(
            &gtk::Label::builder()
                .halign(gtk::Align::Start)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        list_item
            .downcast_ref::<gtk::ListItem>()
            .unwrap()
            .set_child(Some(&row));
    });
    factory.connect_bind({
        let on_secondary = on_secondary.clone();
        let on_middle = on_middle.clone();
        let on_drag_prepare = on_drag_prepare.clone();
        let on_drop = on_drop.clone();
        let on_external_move = on_external_move.clone();
        move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
            let obj = obj_of(list_item);
            let row = list_item.child().and_downcast::<gtk::Box>().unwrap();
            let icon = row.first_child().and_downcast::<gtk::Image>().unwrap();
            let label = row.last_child().and_downcast::<gtk::Label>().unwrap();
            label.set_text(&obj.name());
            match obj.icon() {
                Some(gicon) => icon.set_from_gicon(&gicon),
                None => icon.set_from_gicon(&fallback_icon(obj.is_dir())),
            }
            set_secondary(&row, &obj, &on_secondary);
            set_middle(&row, &obj, &on_middle);
            let dragged = obj.clone();
            let on_drag_prepare = on_drag_prepare.clone();
            dnd::attach_row_drag_drop(
                &row,
                obj.uri(),
                obj.is_dir(),
                Rc::new(move || on_drag_prepare(&dragged)),
                on_drop.clone(),
                on_external_move.clone(),
            );
        }
    });
    factory
}

/// Label-only column (Size, Type in details).
fn text_factory(
    get: fn(&FileObject) -> String,
    on_secondary: &SecondaryHandler,
    on_middle: &MiddleHandler,
    on_drag_prepare: &DragPrepareHandler,
    on_drop: &DropHandler,
    on_external_move: &dnd::ExternalMoveHandler,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, list_item| {
        let label = gtk::Label::builder()
            // Fills the cell (so the menu also arrives by clicking the
            // space right of the text), text still on the left.
            .xalign(0.0)
            .margin_start(8)
            .margin_end(8)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        label.add_css_class("file-item");
        list_item
            .downcast_ref::<gtk::ListItem>()
            .unwrap()
            .set_child(Some(&label));
    });
    let on_secondary = on_secondary.clone();
    let on_middle = on_middle.clone();
    let on_drag_prepare = on_drag_prepare.clone();
    let on_drop = on_drop.clone();
    let on_external_move = on_external_move.clone();
    factory.connect_bind(move |_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = list_item.child().and_downcast::<gtk::Label>().unwrap();
        let obj = obj_of(list_item);
        label.set_text(&get(&obj));
        set_secondary(&label, &obj, &on_secondary);
        set_middle(&label, &obj, &on_middle);
        let dragged = obj.clone();
        let on_drag_prepare = on_drag_prepare.clone();
        dnd::attach_row_drag_drop(
            &label,
            obj.uri(),
            obj.is_dir(),
            Rc::new(move || on_drag_prepare(&dragged)),
            on_drop.clone(),
            on_external_move.clone(),
        );
    });
    factory
}

fn build_compact(
    selection: gtk::MultiSelection,
    handlers: &ViewHandlers<'_>,
    zoom: u8,
) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(Some(selection));
    let column = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("column-name")),
        Some(name_factory(
            (22 * i32::from(zoom) / 100).max(12),
            handlers.on_secondary,
            handlers.on_middle,
            handlers.on_drag_prepare,
            handlers.on_drop,
            handlers.on_external_move,
        )),
    );
    column.set_expand(true);
    view.append_column(&column);
    view
}

fn build_details(
    selection: gtk::MultiSelection,
    handlers: &ViewHandlers<'_>,
    display: ViewDisplay,
    sort_order: Rc<std::cell::Cell<kito_core::SortOrder>>,
    on_sort: Rc<dyn Fn(kito_core::SortOrder)>,
) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(Some(selection));
    let name = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("column-name")),
        Some(name_factory(
            (20 * i32::from(display.icon_zoom) / 100).max(12),
            handlers.on_secondary,
            handlers.on_middle,
            handlers.on_drag_prepare,
            handlers.on_drop,
            handlers.on_external_move,
        )),
    );
    name.set_id(Some("name"));
    name.set_expand(true);
    view.append_column(&name);
    let size = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("props-size")),
        Some(text_factory(
            |o| human_size(o.size()),
            handlers.on_secondary,
            handlers.on_middle,
            handlers.on_drag_prepare,
            handlers.on_drop,
            handlers.on_external_move,
        )),
    );
    size.set_id(Some("size"));
    size.set_fixed_width(110);
    size.set_visible(display.show_size);
    view.append_column(&size);
    let kind = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("props-type")),
        Some(text_factory(
            |o| o.type_label(),
            handlers.on_secondary,
            handlers.on_middle,
            handlers.on_drag_prepare,
            handlers.on_drop,
            handlers.on_external_move,
        )),
    );
    kind.set_id(Some("type"));
    kind.set_fixed_width(200);
    kind.set_visible(display.show_type);
    view.append_column(&kind);
    let modified = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("props-modified")),
        Some(text_factory(
            |o| modified_label(o.modified()),
            handlers.on_secondary,
            handlers.on_middle,
            handlers.on_drag_prepare,
            handlers.on_drop,
            handlers.on_external_move,
        )),
    );
    modified.set_id(Some("modified"));
    modified.set_fixed_width(170);
    modified.set_visible(display.show_modified);
    view.append_column(&modified);

    let headers: HeaderLabels = Rc::new(std::cell::RefCell::new(Vec::new()));
    let header_factory = gtk::SignalListItemFactory::new();
    header_factory.connect_setup({
        let headers = headers.clone();
        let sort_order = sort_order.clone();
        let on_sort = on_sort.clone();
        move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap().clone();
            let button = gtk::Button::builder()
                .has_frame(false)
                .halign(gtk::Align::Start)
                .build();
            let label = gtk::Label::builder()
                .halign(gtk::Align::Start)
                .margin_start(8)
                .margin_end(8)
                .build();
            button.set_child(Some(&label));
            let weak_item = list_item.downgrade();
            let headers = headers.clone();
            let sort_order = sort_order.clone();
            let on_sort = on_sort.clone();
            button.connect_clicked(move |_| {
                let Some(column) = weak_item
                    .upgrade()
                    .and_then(|item| item.item())
                    .and_downcast::<gtk::ColumnViewColumn>()
                else {
                    return;
                };
                let Some(field) = column.id().as_deref().and_then(sort_field_for_column) else {
                    return;
                };
                let current = sort_order.get();
                let direction = if current.field == field {
                    match current.direction {
                        kito_core::SortDirection::Ascending => kito_core::SortDirection::Descending,
                        kito_core::SortDirection::Descending => kito_core::SortDirection::Ascending,
                    }
                } else {
                    kito_core::SortDirection::Ascending
                };
                let next = kito_core::SortOrder { field, direction };
                sort_order.set(next);
                on_sort(next);
                for (id, weak_label) in headers.borrow().iter() {
                    if let Some(label) = weak_label.upgrade() {
                        let title = label.tooltip_text().unwrap_or_default();
                        label.set_text(&sort_header_text(&title, sort_field_for_column(id), next));
                    }
                }
            });
            list_item.set_child(Some(&button));
        }
    });
    header_factory.connect_bind({
        let headers = headers.clone();
        let sort_order = sort_order.clone();
        move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(column) = list_item.item().and_downcast::<gtk::ColumnViewColumn>() else {
                return;
            };
            let Some(button) = list_item.child().and_downcast::<gtk::Button>() else {
                return;
            };
            let Some(label) = button.child().and_downcast::<gtk::Label>() else {
                return;
            };
            let title = column.title().unwrap_or_default().to_string();
            label.set_tooltip_text(Some(&title));
            label.set_text(&sort_header_text(
                &title,
                column.id().as_deref().and_then(sort_field_for_column),
                sort_order.get(),
            ));
            let weak = glib::WeakRef::new();
            weak.set(Some(&label));
            let id = column.id().map(|id| id.to_string()).unwrap_or_default();
            if !headers.borrow().iter().any(|(known, _)| known == &id) {
                headers.borrow_mut().push((id, weak));
            }
        }
    });
    view.set_header_factory(Some(&header_factory));
    view
}

fn sort_field_for_column(id: &str) -> Option<kito_core::SortField> {
    match id {
        "name" => Some(kito_core::SortField::Name),
        "size" => Some(kito_core::SortField::Size),
        "type" => Some(kito_core::SortField::Type),
        "modified" => Some(kito_core::SortField::Modified),
        _ => None,
    }
}

fn sort_header_text(
    title: &str,
    field: Option<kito_core::SortField>,
    order: kito_core::SortOrder,
) -> String {
    if field == Some(order.field) {
        format!(
            "{title} {}",
            if order.direction == kito_core::SortDirection::Ascending {
                "↑"
            } else {
                "↓"
            }
        )
    } else {
        title.to_string()
    }
}

fn modified_label(timestamp: Option<i64>) -> String {
    timestamp
        .and_then(|timestamp| glib::DateTime::from_unix_local(timestamp).ok())
        .and_then(|date| date.format("%x %H:%M").ok())
        .map(|text| text.to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn build_icons(
    selection: gtk::MultiSelection,
    handlers: &ViewHandlers<'_>,
    zoom: u8,
) -> gtk::GridView {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, list_item| {
        let cell = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_start(6)
            .margin_end(6)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        cell.add_css_class("file-item");
        let image = gtk::Image::new();
        image.set_pixel_size((48 * i32::from(zoom) / 100).max(24));
        cell.append(&image);
        cell.append(
            &gtk::Label::builder()
                .halign(gtk::Align::Center)
                .justify(gtk::Justification::Center)
                // Fixed width in characters over 3 lines: the container
                // must never grow wider.
                .width_chars(12)
                .max_width_chars(12)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(3)
                .single_line_mode(false)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        list_item
            .downcast_ref::<gtk::ListItem>()
            .unwrap()
            .set_child(Some(&cell));
    });
    let on_secondary = handlers.on_secondary.clone();
    let on_middle = handlers.on_middle.clone();
    let on_drag_prepare = handlers.on_drag_prepare.clone();
    let on_drop = handlers.on_drop.clone();
    let on_external_move = handlers.on_external_move.clone();
    factory.connect_bind(move |_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let obj = obj_of(list_item);
        let cell = list_item.child().and_downcast::<gtk::Box>().unwrap();
        let image = cell.first_child().and_downcast::<gtk::Image>().unwrap();
        let label = cell.last_child().and_downcast::<gtk::Label>().unwrap();
        label.set_text(&obj.name());
        match obj.icon() {
            Some(gicon) => image.set_from_gicon(&gicon),
            None => image.set_from_gicon(&fallback_icon(obj.is_dir())),
        }
        set_secondary(&cell, &obj, &on_secondary);
        set_middle(&cell, &obj, &on_middle);
        let dragged = obj.clone();
        let on_drag_prepare = on_drag_prepare.clone();
        dnd::attach_row_drag_drop(
            &cell,
            obj.uri(),
            obj.is_dir(),
            Rc::new(move || on_drag_prepare(&dragged)),
            on_drop.clone(),
            on_external_move.clone(),
        );
    });
    gtk::GridView::new(Some(selection), Some(factory))
}

/// Builds the view for `mode` on the given store. Returns widget and selection.
pub fn build_view(
    mode: ViewMode,
    store: &gio::ListStore,
    handlers: &ViewHandlers<'_>,
    open_items: OpenItems,
    display: ViewDisplay,
    sort_order: Rc<std::cell::Cell<kito_core::SortOrder>>,
    on_sort: Rc<dyn Fn(kito_core::SortOrder)>,
) -> (gtk::Widget, gtk::MultiSelection) {
    let selection = gtk::MultiSelection::new(Some(store.clone()));
    let widget: gtk::Widget = match mode {
        ViewMode::Icons => build_icons(selection.clone(), handlers, display.icon_zoom).upcast(),
        ViewMode::Compact => build_compact(selection.clone(), handlers, display.icon_zoom).upcast(),
        ViewMode::Details => {
            build_details(selection.clone(), handlers, display, sort_order, on_sort).upcast()
        }
    };
    // Activation (pointer or keyboard) has one shared handler in FileTab.
    set_open_items(&widget, open_items);
    widget.set_vexpand(true);
    (widget, selection)
}

/// Attaches file-view-local shortcuts so text entries and dialogs keep their
/// normal keyboard behavior.
pub fn add_file_shortcuts(widget: &gtk::Widget) {
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Local);
    for (accelerator, name) in crate::shortcuts::FILE_VIEW_SHORTCUTS {
        if let (Some(trigger), Some(action)) = (
            gtk::ShortcutTrigger::parse_string(accelerator),
            gtk::ShortcutAction::parse_string(name),
        ) {
            controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
        }
    }
    widget.add_controller(controller);
}

/// True while a Ctrl/Shift pointer activation should only change selection.
pub fn add_modified_click_guard(widget: &gtk::Widget) -> Rc<std::cell::Cell<bool>> {
    let suppress = Rc::new(std::cell::Cell::new(false));
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_PRIMARY)
        .build();
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state = suppress.clone();
    gesture.connect_pressed(move |gesture, _, _, _| {
        let modifiers = gesture.current_event_state();
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        {
            state.set(true);
            let state = state.clone();
            glib::idle_add_local_once(move || state.set(false));
        }
    });
    widget.add_controller(gesture);
    suppress
}

/// Configure GTK's built-in pointer activation while leaving Enter and the
/// view's single `activate` signal handler untouched.
pub fn set_open_items(widget: &gtk::Widget, behavior: OpenItems) {
    let single = single_click_activation(behavior);
    if let Ok(view) = widget.clone().downcast::<gtk::ColumnView>() {
        view.set_single_click_activate(single);
    } else if let Ok(view) = widget.clone().downcast::<gtk::GridView>() {
        view.set_single_click_activate(single);
    }
}

fn single_click_activation(behavior: OpenItems) -> bool {
    behavior == OpenItems::SingleClick
}

/// View + empty store; populate with [`reload`].
pub fn build_file_view() -> (gtk::ScrolledWindow, gio::ListStore) {
    let store = gio::ListStore::new::<FileObject>();
    let scrolled = gtk::ScrolledWindow::builder().vexpand(true).build();
    (scrolled, store)
}

/// Rows created and inserted per main-loop turn during asynchronous loads.
/// Keeps each pause short while staying near single-splice total time.
pub const LOAD_CHUNK: usize = 500;

/// Replaces the whole store content with `entries` in a single `splice`:
/// one model notification instead of one per row.
#[cfg(test)]
pub fn replace_all(store: &gio::ListStore, entries: &[kito_core::Entry]) {
    let objs: Vec<FileObject> = entries.iter().map(FileObject::new).collect();
    store.splice(0, store.n_items(), &objs);
}

/// Appends one chunk of entries with a single `splice` (one notification).
/// The caller clears the store first for a fresh load.
pub fn append_chunk(store: &gio::ListStore, chunk: &[kito_core::Entry]) {
    if chunk.is_empty() {
        return;
    }
    let objs: Vec<FileObject> = chunk.iter().map(FileObject::new).collect();
    store.splice(store.n_items(), 0, &objs);
}

/// Fills the store with `dir_uri`. Returns the entry count.
/// Test helper: the UI loads asynchronously (see `tabs`), sharing the same
/// `list_dir` ordering and single-splice application.
#[cfg(test)]
pub fn reload(
    store: &gio::ListStore,
    dir_uri: &str,
    show_hidden: bool,
) -> Result<usize, glib::Error> {
    let entries = kito_core::list_dir(dir_uri, show_hidden)?;
    replace_all(store, &entries);
    Ok(entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_setting_only_switches_gtk_pointer_activation() {
        assert!(!single_click_activation(OpenItems::DoubleClick));
        assert!(single_click_activation(OpenItems::SingleClick));
        // wire_activate remains the only activation-signal hookup for both
        // view types; changing this flag never attaches another handler.
    }

    fn listed_uris(store: &gio::ListStore) -> Vec<String> {
        (0..store.n_items())
            .filter_map(|i| {
                store
                    .item(i)
                    .and_downcast::<FileObject>()
                    .map(|obj| obj.uri())
            })
            .collect()
    }

    fn fixture_entries(n: usize) -> Vec<kito_core::Entry> {
        (0..n)
            .map(|i| kito_core::Entry {
                name: format!("file-{i:05}"),
                uri: format!("file:///tmp/file-{i:05}"),
                is_dir: i % 7 == 0,
                size: i as i64,
                modified: Some(i as i64),
                content_type: None,
                icon: None,
            })
            .collect()
    }

    #[test]
    fn replace_all_fills_in_order_with_one_notification() {
        let store = gio::ListStore::new::<FileObject>();
        let emissions = std::rc::Rc::new(std::cell::Cell::new(0u32));
        store.connect_items_changed({
            let emissions = emissions.clone();
            move |_, _, _, _| emissions.set(emissions.get() + 1)
        });
        let entries = fixture_entries(1000);
        replace_all(&store, &entries);
        assert_eq!(store.n_items(), 1000);
        assert_eq!(listed_uris(&store).len(), 1000);
        assert_eq!(listed_uris(&store)[7], "file:///tmp/file-00007");
        // Bulk: a single model notification, not one per row.
        assert_eq!(emissions.get(), 1);

        // Refill replaces without growing: same count, new content, still one shot.
        emissions.set(0);
        let mut again = fixture_entries(1000);
        again[0].uri = "file:///tmp/changed".to_string();
        replace_all(&store, &again);
        assert_eq!(store.n_items(), 1000);
        assert_eq!(listed_uris(&store)[0], "file:///tmp/changed");
        assert_eq!(emissions.get(), 1);
    }

    #[test]
    fn chunked_append_matches_replace_all() {
        let entries = fixture_entries(1234);
        let full = gio::ListStore::new::<FileObject>();
        replace_all(&full, &entries);

        let chunked = gio::ListStore::new::<FileObject>();
        chunked.remove_all();
        for chunk in entries.chunks(LOAD_CHUNK) {
            append_chunk(&chunked, chunk);
        }
        assert_eq!(chunked.n_items(), full.n_items());
        assert_eq!(listed_uris(&chunked), listed_uris(&full));
    }

    #[test]
    fn append_chunk_empty_is_noop() {
        let store = gio::ListStore::new::<FileObject>();
        let emissions = std::rc::Rc::new(std::cell::Cell::new(0u32));
        store.connect_items_changed({
            let emissions = emissions.clone();
            move |_, _, _, _| emissions.set(emissions.get() + 1)
        });
        append_chunk(&store, &[]);
        assert_eq!(store.n_items(), 0);
        assert_eq!(emissions.get(), 0);
    }
}
