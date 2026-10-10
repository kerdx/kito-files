//! Classic views: icons, compact, details. One row GObject + factory
//! on a shared ListStore; no widget subclassing.

use glib::subclass::prelude::ObjectSubclassIsExt as _;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;

use crate::dnd::{self, DropHandler};
use crate::preferences::model::OpenItems;
pub use crate::preferences::model::ViewMode;

const ICON_VIEW_BASE_SIZE: i32 = 96;
const ICON_VIEW_BASE_CELL_WIDTH: i32 = 160;
const NAME_ICON_MIN_SIZE: i32 = 26;

fn name_icon_pixel_size(base_size: i32, zoom: u8) -> i32 {
    (base_size * i32::from(zoom) / 100).max(NAME_ICON_MIN_SIZE)
}

fn icon_pixel_size(zoom: u8) -> i32 {
    (ICON_VIEW_BASE_SIZE * i32::from(zoom) / 100).max(24)
}

fn icon_cell_width(zoom: u8) -> i32 {
    (ICON_VIEW_BASE_CELL_WIDTH * i32::from(zoom) / 100).max(96)
}

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

/// Resolve file-list icons as regular artwork. GtkImage's default GIcon lookup
/// can select a symbolic companion from a GThemedIcon's name/fallback list;
/// the symbolic variants in some themes are monochrome even when the regular
/// icon is not. Keep this scoped to the name column: the grid retains GTK's
/// normal GIcon selection behavior.
fn set_file_list_icon(image: &gtk::Image, icon: &impl IsA<gio::Icon>) {
    let key = glib::Quark::from_str("kito-file-list-icon-state");
    let current_icon = unsafe { image.qdata::<Rc<std::cell::RefCell<Option<gio::Icon>>>>(key) }
        .map(|value| unsafe { value.as_ref().clone() });
    let state = current_icon.unwrap_or_else(|| {
        let state = Rc::new(std::cell::RefCell::new(None));
        unsafe {
            image.set_qdata(key, state.clone());
        }
        install_icon_refresh(image, state.clone());
        state
    });
    *state.borrow_mut() = Some(icon.as_ref().clone());
    refresh_file_list_icon(image, &state);
}

fn refresh_file_list_icon(image: &gtk::Image, state: &Rc<std::cell::RefCell<Option<gio::Icon>>>) {
    let Some(icon) = state.borrow().clone() else {
        return;
    };
    let theme = gtk::IconTheme::for_display(&image.display());
    let paintable = theme.lookup_by_gicon(
        &icon,
        image.pixel_size().max(1),
        image.scale_factor(),
        image.direction(),
        gtk::IconLookupFlags::FORCE_REGULAR,
    );
    image.set_paintable(Some(&paintable));
}

fn install_icon_refresh(image: &gtk::Image, state: Rc<std::cell::RefCell<Option<gio::Icon>>>) {
    let weak_image = image.downgrade();
    let scale_state = state.clone();
    image.connect_notify_local(Some("scale-factor"), move |_, _| {
        if let Some(image) = weak_image.upgrade() {
            refresh_file_list_icon(&image, &scale_state);
        }
    });

    let handler = Rc::new(std::cell::RefCell::new(None::<glib::SignalHandlerId>));
    let on_realize = {
        let weak_image = image.downgrade();
        let state = state.clone();
        let handler = handler.clone();
        move |_: &gtk::Image| {
            let Some(image) = weak_image.upgrade() else {
                return;
            };
            let theme = gtk::IconTheme::for_display(&image.display());
            if handler.borrow().is_none() {
                let weak_image = image.downgrade();
                let state = state.clone();
                let id = theme.connect_changed(move |_| {
                    if let Some(image) = weak_image.upgrade() {
                        refresh_file_list_icon(&image, &state);
                    }
                });
                *handler.borrow_mut() = Some(id);
            }
            refresh_file_list_icon(&image, &state);
        }
    };
    image.connect_realize(on_realize);
    let weak_image = image.downgrade();
    let unrealize_handler = handler.clone();
    image.connect_unrealize(move |_| {
        if let Some(id) = unrealize_handler.borrow_mut().take() {
            if let Some(image) = weak_image.upgrade() {
                gtk::IconTheme::for_display(&image.display()).disconnect(id);
            }
        }
    });
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
                Some(gicon) => set_file_list_icon(&icon, &gicon),
                None => set_file_list_icon(&icon, &fallback_icon(obj.is_dir())),
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
            name_icon_pixel_size(22, zoom),
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
            name_icon_pixel_size(20, display.icon_zoom),
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

    // ColumnView's own headers are the clickable, localized column titles.
    // The view-level `header_factory` is for GtkListHeader section rows, not
    // column titles; using it here passes a non-ListItem to its callbacks.
    // Give each column a sorter so GTK enables the native header click and
    // arrow indicator. The app sorts its ListStore asynchronously via
    // `on_sort`, rather than installing GTK's sorter on a SortListModel.
    let click_sorter = gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal);
    name.set_sorter(Some(&click_sorter));
    size.set_sorter(Some(&click_sorter));
    kind.set_sorter(Some(&click_sorter));
    modified.set_sorter(Some(&click_sorter));

    let initial = sort_order.get();
    let initial_column = match initial.field {
        kito_core::SortField::Name => &name,
        kito_core::SortField::Size => &size,
        kito_core::SortField::Type => &kind,
        kito_core::SortField::Modified => &modified,
    };
    view.sort_by_column(Some(initial_column), gtk_sort_type(initial.direction));

    if let Some(sorter) = view.sorter().and_downcast::<gtk::ColumnViewSorter>() {
        sorter.connect_changed({
            let sort_order = sort_order.clone();
            move |sorter, _| {
                let Some(column) = sorter.primary_sort_column() else {
                    return;
                };
                let Some(field) = column.id().as_deref().and_then(sort_field_for_column) else {
                    return;
                };
                let direction = if sorter.primary_sort_order() == gtk::SortType::Descending {
                    kito_core::SortDirection::Descending
                } else {
                    kito_core::SortDirection::Ascending
                };
                let next = kito_core::SortOrder { field, direction };
                // Let the owner update this shared state and kick off its
                // asynchronous re-sort; setting it first would make that
                // callback treat the requested order as already applied.
                if sort_order.get() != next {
                    on_sort(next);
                }
            }
        });
    }
    view
}

fn gtk_sort_type(direction: kito_core::SortDirection) -> gtk::SortType {
    match direction {
        kito_core::SortDirection::Ascending => gtk::SortType::Ascending,
        kito_core::SortDirection::Descending => gtk::SortType::Descending,
    }
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
        cell.set_size_request(icon_cell_width(zoom), -1);
        cell.add_css_class("file-item");
        let image = gtk::Image::new();
        image.set_pixel_size(icon_pixel_size(zoom));
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

    #[test]
    fn icon_zoom_matches_the_larger_grid_reference() {
        assert_eq!(icon_pixel_size(60), 57);
        assert_eq!(icon_pixel_size(100), 96);
        assert_eq!(icon_pixel_size(180), 172);
        assert_eq!(icon_cell_width(60), 96);
        assert_eq!(icon_cell_width(100), 160);
        assert_eq!(icon_cell_width(180), 288);
    }

    #[test]
    fn name_view_icons_keep_theme_asset_size_floor_and_follow_zoom() {
        for base_size in [22, 20] {
            // Compact and Details
            assert_eq!(name_icon_pixel_size(base_size, 100), 26);
            assert!(name_icon_pixel_size(base_size, 150) > 26);
            assert!(name_icon_pixel_size(base_size, 200) > name_icon_pixel_size(base_size, 150));
        }
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn file_list_icon_lookup_uses_regular_theme_paintable() {
        gtk::test_synced(|| {
            let image = gtk::Image::new();
            image.set_pixel_size(name_icon_pixel_size(20, 100));
            let icon = gio::ThemedIcon::from_names(&["folder-documents", "folder"]);
            set_file_list_icon(&image, &icon);

            let paintable = image
                .paintable()
                .and_downcast::<gtk::IconPaintable>()
                .expect("theme lookup should install an icon paintable");
            let resolved_file = paintable
                .file()
                .and_then(|file| file.path())
                .expect("theme icon should resolve to an asset");
            assert!(resolved_file.exists(), "resolved asset: {resolved_file:?}");
            assert!(
                !resolved_file
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.ends_with("-symbolic")),
                "file-list icons must resolve to regular theme assets: {resolved_file:?}"
            );
        });
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

    fn widget_contains_label(widget: &gtk::Widget, expected: &str) -> bool {
        if widget
            .clone()
            .downcast::<gtk::Label>()
            .is_ok_and(|label| label.text() == expected)
        {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if widget_contains_label(&current, expected) {
                return true;
            }
            child = current.next_sibling();
        }
        false
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn details_cells_bind_and_native_column_headers_sort_both_directions() {
        gtk::test_synced(|| {
            let store = gio::ListStore::new::<FileObject>();
            replace_all(&store, &fixture_entries(3));
            let on_secondary: SecondaryHandler = Rc::new(|_, _, _, _| {});
            let on_middle: MiddleHandler = Rc::new(|_| {});
            let on_drag_prepare: DragPrepareHandler = Rc::new(|_| None);
            let on_drop: DropHandler = Rc::new(|_, _, _, _, _| {});
            let on_external_move: dnd::ExternalMoveHandler = Rc::new(|_| {});
            let handlers = ViewHandlers {
                on_secondary: &on_secondary,
                on_middle: &on_middle,
                on_drag_prepare: &on_drag_prepare,
                on_drop: &on_drop,
                on_external_move: &on_external_move,
            };
            let display = ViewDisplay {
                icon_zoom: 100,
                show_size: false,
                show_type: true,
                show_modified: false,
            };
            let sort_order = Rc::new(std::cell::Cell::new(kito_core::SortOrder::default()));
            let observed_orders = Rc::new(std::cell::RefCell::new(Vec::new()));
            let on_sort: Rc<dyn Fn(kito_core::SortOrder)> = Rc::new({
                let sort_order = sort_order.clone();
                let observed_orders = observed_orders.clone();
                move |order| {
                    sort_order.set(order);
                    observed_orders.borrow_mut().push(order);
                }
            });

            let (widget, selection) = build_view(
                ViewMode::Details,
                &store,
                &handlers,
                OpenItems::DoubleClick,
                display,
                sort_order,
                on_sort,
            );
            let view = widget.clone().downcast::<gtk::ColumnView>().unwrap();
            // ColumnView.header_factory configures section headers, not the
            // column titles. GTK's real ColumnViewCells still run the row factory.
            assert!(view.header_factory().is_none());

            let columns = view.columns();
            let column = |id: &str| {
                (0..columns.n_items())
                    .filter_map(|index| columns.item(index).and_downcast::<gtk::ColumnViewColumn>())
                    .find(|column| column.id().as_deref() == Some(id))
                    .unwrap()
            };
            let name = column("name");
            let size = column("size");
            let kind = column("type");
            let modified = column("modified");
            assert!(name.sorter().is_some());
            assert!(size.sorter().is_some());
            assert!(kind.sorter().is_some());
            assert!(modified.sorter().is_some());
            assert!(name.is_visible());
            assert!(!size.is_visible());
            assert!(kind.is_visible());
            assert!(!modified.is_visible());
            let localized_name = crate::l10n::tr("column-name");
            assert_eq!(name.title().as_deref(), Some(localized_name.as_str()));

            // Map the real ColumnView so GTK invokes the row factory's setup and
            // bind callbacks on its ColumnViewCell objects.
            let window = gtk::Window::new();
            window.set_default_size(640, 240);
            window.set_child(Some(&widget));
            window.present();
            let context = glib::MainContext::default();
            for _ in 0..10 {
                while context.pending() {
                    context.iteration(false);
                }
            }
            assert!(widget_contains_label(&widget, "file-00000"));

            selection.select_item(1, false);
            let view_sorter = view
                .sorter()
                .and_downcast::<gtk::ColumnViewSorter>()
                .unwrap();
            assert_eq!(
                view_sorter
                    .primary_sort_column()
                    .and_then(|column| column.id())
                    .as_deref(),
                Some("name")
            );
            assert_eq!(view_sorter.primary_sort_order(), gtk::SortType::Ascending);

            // `sort_by_column` is the same GTK sorter path used by clicking the
            // native column header. It verifies first-click ascending, repeated
            // clicks toggling direction, and the app's sort callback.
            view.sort_by_column(Some(&name), gtk::SortType::Descending);
            view.sort_by_column(Some(&name), gtk::SortType::Ascending);
            view.sort_by_column(Some(&size), gtk::SortType::Ascending);
            view.sort_by_column(Some(&size), gtk::SortType::Descending);
            assert_eq!(
                *observed_orders.borrow(),
                vec![
                    kito_core::SortOrder {
                        field: kito_core::SortField::Name,
                        direction: kito_core::SortDirection::Descending,
                    },
                    kito_core::SortOrder {
                        field: kito_core::SortField::Name,
                        direction: kito_core::SortDirection::Ascending,
                    },
                    kito_core::SortOrder {
                        field: kito_core::SortField::Size,
                        direction: kito_core::SortDirection::Ascending,
                    },
                    kito_core::SortOrder {
                        field: kito_core::SortField::Size,
                        direction: kito_core::SortDirection::Descending,
                    },
                ]
            );
            assert!(selection.is_selected(1));
            assert_eq!(view_sorter.primary_sort_order(), gtk::SortType::Descending);
            window.close();
        });
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
