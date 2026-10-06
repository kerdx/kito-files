//! Classic views: icons, compact, details. One row GObject + factory
//! on a shared ListStore; no widget subclassing.

use glib::subclass::prelude::ObjectSubclassIsExt as _;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;

use crate::preferences::model::OpenItems;
pub use crate::preferences::model::ViewMode;

/// Right-click on a row: object + point + anchor widget.
/// The handler selects the object and opens the menu.
pub type SecondaryHandler = Rc<dyn Fn(&FileObject, f64, f64, &gtk::Widget)>;

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

mod imp {
    use glib::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct FileObject {
        pub name: RefCell<String>,
        pub uri: RefCell<String>,
        pub is_dir: Cell<bool>,
        pub size: Cell<i64>,
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
        *imp.content_type.borrow_mut() = entry.content_type.clone();
        *imp.icon.borrow_mut() = entry.icon.clone();
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

fn fallback_icon(is_dir: bool) -> gio::ThemedIcon {
    gio::ThemedIcon::new(if is_dir { "folder" } else { "text-x-generic" })
}

/// Icon + name row (used by compact and details).
fn name_factory(pixel_size: i32, on_secondary: &SecondaryHandler) -> gtk::SignalListItemFactory {
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
        }
    });
    factory
}

/// Label-only column (Size, Type in details).
fn text_factory(
    get: fn(&FileObject) -> String,
    on_secondary: &SecondaryHandler,
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
        list_item
            .downcast_ref::<gtk::ListItem>()
            .unwrap()
            .set_child(Some(&label));
    });
    let on_secondary = on_secondary.clone();
    factory.connect_bind(move |_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = list_item.child().and_downcast::<gtk::Label>().unwrap();
        let obj = obj_of(list_item);
        label.set_text(&get(&obj));
        set_secondary(&label, &obj, &on_secondary);
    });
    factory
}

fn build_compact(
    selection: gtk::SingleSelection,
    on_secondary: &SecondaryHandler,
) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(Some(selection));
    let column = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("column-name")),
        Some(name_factory(22, on_secondary)),
    );
    column.set_expand(true);
    view.append_column(&column);
    view
}

fn build_details(
    selection: gtk::SingleSelection,
    on_secondary: &SecondaryHandler,
) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(Some(selection));
    let name = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("column-name")),
        Some(name_factory(20, on_secondary)),
    );
    name.set_expand(true);
    view.append_column(&name);
    let size = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("props-size")),
        Some(text_factory(|o| human_size(o.size()), on_secondary)),
    );
    size.set_fixed_width(110);
    view.append_column(&size);
    let kind = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("props-type")),
        Some(text_factory(|o| o.type_label(), on_secondary)),
    );
    kind.set_fixed_width(200);
    view.append_column(&kind);
    view
}

fn build_icons(selection: gtk::SingleSelection, on_secondary: &SecondaryHandler) -> gtk::GridView {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, list_item| {
        let cell = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_start(6)
            .margin_end(6)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        let image = gtk::Image::new();
        image.set_pixel_size(48);
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
    let on_secondary = on_secondary.clone();
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
    });
    gtk::GridView::new(Some(selection), Some(factory))
}

/// Builds the view for `mode` on the given store. Returns widget and selection.
pub fn build_view(
    mode: ViewMode,
    store: &gio::ListStore,
    on_secondary: &SecondaryHandler,
    open_items: OpenItems,
) -> (gtk::Widget, gtk::SingleSelection) {
    let selection = gtk::SingleSelection::new(Some(store.clone()));
    let widget: gtk::Widget = match mode {
        ViewMode::Icons => build_icons(selection.clone(), on_secondary).upcast(),
        ViewMode::Compact => build_compact(selection.clone(), on_secondary).upcast(),
        ViewMode::Details => build_details(selection.clone(), on_secondary).upcast(),
    };
    // Activation (pointer or keyboard) has one shared handler in FileTab.
    set_open_items(&widget, open_items);
    widget.set_vexpand(true);
    (widget, selection)
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
}

/// View + empty store; populate with [`reload`].
pub fn build_file_view() -> (gtk::ScrolledWindow, gio::ListStore) {
    let store = gio::ListStore::new::<FileObject>();
    let scrolled = gtk::ScrolledWindow::builder().vexpand(true).build();
    (scrolled, store)
}

/// Fills the store with `dir_uri`. Returns the entry count.
pub fn reload(
    store: &gio::ListStore,
    dir_uri: &str,
    show_hidden: bool,
) -> Result<usize, glib::Error> {
    let entries = kito_core::list_dir(dir_uri, show_hidden)?;
    store.remove_all();
    for entry in &entries {
        store.append(&FileObject::new(entry));
    }
    Ok(entries.len())
}
