//! Viste classiche: icone, compatta, dettagli. Un GObject riga + factory
//! su ListStore condiviso; niente subclassing di widget.

use glib::subclass::prelude::ObjectSubclassIsExt as _;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;

/// Click destro su una riga: oggetto + punto + widget di ancoraggio.
/// Il gestore seleziona l'oggetto e apre il menu.
pub type SecondaryHandler = Rc<dyn Fn(&FileObject, f64, f64, &gtk::Widget)>;

/// Lega il click destro alla riga per l'oggetto corrente. Chiamato a ogni
/// bind (le righe sono riciclate): rimuove il gesture precedente.
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

/// Modalità di vista della tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Icons,
    Compact,
    Details,
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
            return "Folder".to_string();
        }
        match self.imp().content_type.borrow().as_deref() {
            Some(mime) => gio::content_type_get_description(mime).to_string(),
            None => "File".to_string(),
        }
    }

    pub fn icon(&self) -> Option<gio::Icon> {
        self.imp().icon.borrow().clone()
    }
}

/// 1234567 -> "1.2 MB", -1 (cartelle) -> "—".
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

/// Riga icona + nome (usata da compatta e dettagli).
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
        // 22px: a 16px Papirus/Breeze sono monocromatiche.
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

/// Colonna di sole etichette (Size, Type nei dettagli).
fn text_factory(
    get: fn(&FileObject) -> String,
    on_secondary: &SecondaryHandler,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, list_item| {
        let label = gtk::Label::builder()
            // Riempie la cella (così il menu arriva anche cliccando lo
            // spazio a destra del testo), testo comunque a sinistra.
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
    let column = gtk::ColumnViewColumn::new(Some("Name"), Some(name_factory(22, on_secondary)));
    column.set_expand(true);
    view.append_column(&column);
    view
}

fn build_details(
    selection: gtk::SingleSelection,
    on_secondary: &SecondaryHandler,
) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(Some(selection));
    let name = gtk::ColumnViewColumn::new(Some("Name"), Some(name_factory(20, on_secondary)));
    name.set_expand(true);
    view.append_column(&name);
    let size = gtk::ColumnViewColumn::new(
        Some("Size"),
        Some(text_factory(|o| human_size(o.size()), on_secondary)),
    );
    size.set_fixed_width(110);
    view.append_column(&size);
    let kind = gtk::ColumnViewColumn::new(
        Some("Type"),
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
                // Larghezza fissa in caratteri su 3 righe: il contenitore
                // non deve mai allargarsi oltre.
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

/// Costruisce la vista per `mode` sullo store dato. Ritorna widget e selezione.
pub fn build_view(
    mode: ViewMode,
    store: &gio::ListStore,
    on_secondary: &SecondaryHandler,
) -> (gtk::Widget, gtk::SingleSelection) {
    let selection = gtk::SingleSelection::new(Some(store.clone()));
    let widget: gtk::Widget = match mode {
        ViewMode::Icons => build_icons(selection.clone(), on_secondary).upcast(),
        ViewMode::Compact => build_compact(selection.clone(), on_secondary).upcast(),
        ViewMode::Details => build_details(selection.clone(), on_secondary).upcast(),
    };
    // Doppio click / Invio gestito dalla tab (vede la selezione corrente).
    widget.set_vexpand(true);
    (widget, selection)
}

/// Vista + store vuoto; popolare con [`reload`].
pub fn build_file_view() -> (gtk::ScrolledWindow, gio::ListStore) {
    let store = gio::ListStore::new::<FileObject>();
    let scrolled = gtk::ScrolledWindow::builder().vexpand(true).build();
    (scrolled, store)
}

/// Riempie lo store con `dir_uri`. Ritorna il numero di voci.
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
