//! Menu contestuale premium: popover compatto con righe icona + etichetta.
//! Niente scrollbar, larghezza fissa contenuta. Le righe attivano le
//! azioni `win.*` registrate sulla finestra.

use adw::prelude::*;
use gtk::gio;

/// (icone, etichetta, azione win.*, distruttiva).
type Row = (&'static [&'static str], &'static str, &'static str, bool);

/// Menu su una voce selezionata (riga o cella).
const ROWS: [Row; 8] = [
    (&["document-open"], "Open", "open", false),
    (&["bookmark-new", "list-add"], "Pin to Places", "pin", false),
    (&["edit-cut"], "Cut", "cut", false),
    (&["edit-copy"], "Copy", "copy", false),
    (&["edit-paste"], "Paste", "paste", false),
    (
        &["document-edit", "document-edit-symbolic"],
        "Rename…",
        "rename",
        false,
    ),
    (&["user-trash"], "Move to Trash", "trash", false),
    (&["edit-delete"], "Delete Permanently…", "delete", true),
];

/// Gruppi separati da divisori, come indici in ROWS: [0..2, 2..5, 5..8].
const SEPARATORS_AFTER: [usize; 2] = [1, 4];

/// Menu su una voce dentro il cestino: ripristino e cancellazione.
const TRASH_ROWS: [Row; 4] = [
    (
        &["document-revert", "edit-undo"],
        "Restore",
        "restore",
        false,
    ),
    (&["edit-cut"], "Cut", "cut", false),
    (&["edit-copy"], "Copy", "copy", false),
    (&["edit-delete"], "Delete Permanently…", "delete", true),
];

/// Gruppi: [0..1, 1..3, 3..4].
const TRASH_SEPARATORS_AFTER: [usize; 2] = [0, 2];

/// Menu sullo sfondo (area vuota della cartella).
const BACKGROUND_ROWS: [Row; 2] = [
    (
        &["folder-new", "folder-new-symbolic"],
        "New Folder…",
        "new-folder",
        false,
    ),
    (&["edit-paste"], "Paste", "paste", false),
];

/// Sfondo del cestino: l'unica azione sensata è svuotarlo.
const TRASH_BACKGROUND_ROWS: [Row; 1] = [(
    &["user-trash", "user-trash-full"],
    "Empty Trash…",
    "empty-trash",
    true,
)];

/// Costruisce il popover e lo apre ancorato a `(x, y)` su `anchor`.
fn popup(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    window: &adw::ApplicationWindow,
    rows: &[Row],
    separators_after: &[usize],
) {
    let popover = gtk::Popover::new();
    popover.add_css_class("ctx-menu");
    let list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(6)
        .margin_end(6)
        .margin_top(6)
        .margin_bottom(6)
        .width_request(216)
        .build();
    for (i, (icons, label, action, danger)) in rows.iter().enumerate() {
        let button = gtk::Button::builder().has_frame(false).build();
        button.add_css_class("ctx-row");
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .margin_start(8)
            .margin_end(8)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        let image = gtk::Image::from_gicon(&gio::ThemedIcon::from_names(icons));
        image.set_pixel_size(18);
        row.append(&image);
        let text = gtk::Label::builder()
            .label(*label)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        if *danger {
            text.add_css_class("error");
        }
        row.append(&text);
        button.set_child(Some(&row));
        let window = window.clone();
        let action = action.to_string();
        let popover_weak = popover.downgrade();
        button.connect_clicked(move |_| {
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
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
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.popup();
}

/// Click destro su una voce: menu completo, la voce è già selezionata.
pub fn show(anchor: &gtk::Widget, x: f64, y: f64, window: &adw::ApplicationWindow) {
    popup(anchor, x, y, window, &ROWS, &SEPARATORS_AFTER);
}

/// Click destro su una voce del cestino: ripristina, copia, elimina.
pub fn show_trash(anchor: &gtk::Widget, x: f64, y: f64, window: &adw::ApplicationWindow) {
    popup(anchor, x, y, window, &TRASH_ROWS, &TRASH_SEPARATORS_AFTER);
}

/// Click destro sullo sfondo: nuove cartelle e appunti.
pub fn show_background(anchor: &gtk::Widget, x: f64, y: f64, window: &adw::ApplicationWindow) {
    popup(anchor, x, y, window, &BACKGROUND_ROWS, &[]);
}

/// Click destro sullo sfondo del cestino: svuota il cestino.
pub fn show_trash_background(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    window: &adw::ApplicationWindow,
) {
    popup(anchor, x, y, window, &TRASH_BACKGROUND_ROWS, &[]);
}
