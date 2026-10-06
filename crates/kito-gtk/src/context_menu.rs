//! Menu contestuale premium: popover compatto con righe icona + etichetta.
//!
//! Il sottomenu ("Crea") è una seconda popover accanto alla prima, non
//! una colonna interna: così il menu principale non cambia dimensione e
//! resta fermo. Finché il sottomenu è aperto il padre ha `autohide`
//! disattivato, altrimenti GTK lo chiuderebbe appena il puntatore passa
//! sulla superficie del figlio.

use adw::prelude::*;
use gtk::gio;
use std::{cell::RefCell, rc::Rc, time::Duration};

/// Una riga del menu: icone, etichetta, azione `win.*` (vuota se è un
/// sottomenu) e flag distruttiva.
struct Row {
    icons: &'static [&'static str],
    label: &'static str,
    action: &'static str,
    danger: bool,
    /// Sottomenu: righe + indici dei divisori.
    sub: Option<(&'static [Row], &'static [usize])>,
}

const fn row(icons: &'static [&'static str], label: &'static str, action: &'static str) -> Row {
    Row {
        icons,
        label,
        action,
        danger: false,
        sub: None,
    }
}

const fn danger_row(
    icons: &'static [&'static str],
    label: &'static str,
    action: &'static str,
) -> Row {
    Row {
        icons,
        label,
        action,
        danger: true,
        sub: None,
    }
}

const fn menu(
    icons: &'static [&'static str],
    label: &'static str,
    sub: &'static [Row],
    separators: &'static [usize],
) -> Row {
    Row {
        icons,
        label,
        action: "",
        danger: false,
        sub: Some((sub, separators)),
    }
}

/// Menu su una voce selezionata (riga o cella).
const ROWS: [Row; 9] = [
    row(&["document-open"], "Open", "open"),
    row(&["bookmark-new", "list-add"], "Pin to Places", "pin"),
    row(&["edit-cut"], "Cut", "cut"),
    row(&["edit-copy"], "Copy", "copy"),
    row(&["edit-paste"], "Paste", "paste"),
    row(
        &["document-edit", "document-edit-symbolic"],
        "Rename…",
        "rename",
    ),
    row(&["user-trash"], "Move to Trash", "trash"),
    danger_row(&["edit-delete"], "Delete Permanently…", "delete"),
    row(
        &["dialog-information", "help-about"],
        "Properties",
        "properties",
    ),
];

/// Gruppi: [0..2, 2..5, 5..8, 8..9].
const SEPARATORS_AFTER: [usize; 3] = [1, 4, 7];

/// Menu su una voce dentro il cestino: ripristino e cancellazione.
const TRASH_ROWS: [Row; 5] = [
    row(&["document-revert", "edit-undo"], "Restore", "restore"),
    row(&["edit-cut"], "Cut", "cut"),
    row(&["edit-copy"], "Copy", "copy"),
    danger_row(&["edit-delete"], "Delete Permanently…", "delete"),
    row(
        &["dialog-information", "help-about"],
        "Properties",
        "properties",
    ),
];

/// Gruppi: [0..1, 1..3, 3..4, 4..5].
const TRASH_SEPARATORS_AFTER: [usize; 3] = [0, 2, 3];

/// Colonna "Crea": il tipo è solo il nome iniziale, il nome vero e
/// proprio lo chiede sempre il dialog.
const CREATE_ROWS: [Row; 6] = [
    row(
        &["folder-new", "folder-new-symbolic"],
        "New Folder",
        "new-folder",
    ),
    row(&["text-x-generic"], "New Text File", "new-text-file"),
    row(&["document-new"], "New Empty File", "new-empty-file"),
    row(
        &["x-office-document", "application-msword"],
        "Word Document",
        "new-word-doc",
    ),
    row(&["x-office-spreadsheet"], "Spreadsheet", "new-spreadsheet"),
    row(&["text-html"], "HTML Page", "new-html-page"),
];

/// Menu sullo sfondo (area vuota della cartella).
const BACKGROUND_ROWS: [Row; 5] = [
    menu(&["list-add", "folder-new"], "Create", &CREATE_ROWS, &[]),
    row(&["edit-paste"], "Paste", "paste"),
    row(
        &["utilities-terminal", "terminal"],
        "Open Terminal",
        "open-terminal",
    ),
    row(
        &["utilities-terminal", "terminal"],
        "Open Terminal as Root",
        "open-terminal-root",
    ),
    row(
        &["dialog-information", "help-about"],
        "Properties",
        "properties",
    ),
];

/// Gruppi: [0..1, 1..2, 2..4, 4..5].
const BACKGROUND_SEPARATORS_AFTER: [usize; 3] = [0, 2, 4];

/// Sfondo del cestino: l'unica azione sensata è svuotarlo.
const TRASH_BACKGROUND_ROWS: [Row; 1] = [danger_row(
    &["user-trash-full", "user-trash"],
    "Empty Trash…",
    "empty-trash",
)];

/// Attesa prima di chiudere il sottomenu quando il puntatore esce dalle
/// sue righe: dà tempo di arrivarci davvero.
const CLOSE_DELAY: Duration = Duration::from_millis(300);

/// Sottomenu aperto e timer che lo chiude. I riferimenti sono deboli:
/// le righe del menu stanno dentro i popover, non li tengono vivi.
#[derive(Default)]
struct MenuState {
    submenu: RefCell<Option<glib::WeakRef<gtk::Popover>>>,
    parent: RefCell<Option<glib::WeakRef<gtk::Popover>>>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl MenuState {
    /// Chiude il sottomenu e ridà al padre la chiusura automatica.
    fn close_submenu(&self) {
        self.cancel_close();
        if let Some(weak) = self.submenu.borrow_mut().take() {
            if let Some(popover) = weak.upgrade() {
                popover.popdown();
            }
        }
        if let Some(weak) = self.parent.borrow_mut().take() {
            if let Some(popover) = weak.upgrade() {
                popover.set_autohide(true);
            }
        }
    }

    /// Il puntatore è ancora dentro il menu: nessuna chiusura in arrivo.
    fn cancel_close(&self) {
        if let Some(id) = self.timer.borrow_mut().take() {
            id.remove();
        }
    }

    /// Chiude fra `CLOSE_DELAY`, se il puntatore non rientra.
    fn schedule_close(self: &Rc<Self>) {
        self.cancel_close();
        let this = self.clone();
        *self.timer.borrow_mut() = Some(glib::timeout_add_local_once(CLOSE_DELAY, move || {
            *this.timer.borrow_mut() = None;
            this.close_submenu();
        }));
    }
}

/// Contenitore verticale delle righe di un menu.
fn menu_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(6)
        .margin_end(6)
        .margin_top(6)
        .margin_bottom(6)
        .width_request(216)
        .build()
}

/// Riga del menu: bottone piatto con icona, etichetta e freccia.
fn row_button(r: &Row) -> gtk::Button {
    let button = gtk::Button::builder().has_frame(false).build();
    button.add_css_class("ctx-row");
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .margin_start(8)
        .margin_end(8)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    let image = gtk::Image::from_gicon(&gio::ThemedIcon::from_names(r.icons));
    image.set_pixel_size(18);
    content.append(&image);
    let text = gtk::Label::builder()
        .label(r.label)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    if r.danger {
        text.add_css_class("error");
    }
    content.append(&text);
    if r.sub.is_some() {
        content.append(
            &gtk::Label::builder()
                .label("›")
                .css_classes(["dim-label"])
                .build(),
        );
    }
    button.set_child(Some(&content));
    button
}

/// Costruisce il popover con le sue righe, senza aprirlo.
fn build(
    anchor: &gtk::Widget,
    rows: &[Row],
    separators_after: &[usize],
    window: &adw::ApplicationWindow,
    state: &Rc<MenuState>,
    parent: Option<&gtk::Popover>,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("ctx-menu");
    let list = menu_box();
    for (i, r) in rows.iter().enumerate() {
        let button = row_button(r);
        if let Some((sub_rows, sub_separators)) = r.sub {
            // Il sottomenu si apre sfiorando la riga; il click resta
            // un modo alternativo.
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter({
                let state = state.clone();
                let anchor = button.clone();
                let popover = popover.clone();
                let window = window.clone();
                move |_, _, _| {
                    open_submenu(&anchor, &popover, sub_rows, sub_separators, &window, &state)
                }
            });
            motion.connect_leave({
                let state = state.clone();
                move |_| state.schedule_close()
            });
            button.add_controller(motion);
            let state = state.clone();
            let anchor = button.clone();
            let popover = popover.clone();
            let window = window.clone();
            button.connect_clicked(move |_| {
                open_submenu(&anchor, &popover, sub_rows, sub_separators, &window, &state)
            });
        } else {
            // Su una riga normale il sottomeno non serve: via subito.
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter({
                let state = state.clone();
                move |_, _, _| state.close_submenu()
            });
            button.add_controller(motion);
            let window = window.clone();
            let action = r.action.to_string();
            let popover = popover.clone();
            let parent = parent.cloned();
            button.connect_clicked(move |_| {
                popover.popdown();
                if let Some(parent) = &parent {
                    parent.popdown();
                }
                let _ = gtk::prelude::WidgetExt::activate_action(
                    &window,
                    &format!("win.{action}"),
                    None,
                );
            });
        }
        list.append(&button);
        if separators_after.contains(&i) {
            list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
    }
    popover.set_child(Some(&list));
    popover.set_parent(anchor);
    popover
}

/// Apre il sottomenu accanto alla riga `anchor`, una volta sola.
/// Il padre perde `autohide` finché il figlio è aperto: è la causa dei
/// menu che spariscono appena il puntatore entra nel sottomenu.
fn open_submenu(
    anchor: &gtk::Button,
    parent: &gtk::Popover,
    rows: &[Row],
    separators_after: &[usize],
    window: &adw::ApplicationWindow,
    state: &Rc<MenuState>,
) {
    state.cancel_close();
    if state.submenu.borrow().is_some() {
        return;
    }
    let sub_state = Rc::new(MenuState::default());
    let child = build(
        anchor.upcast_ref(),
        rows,
        separators_after,
        window,
        &sub_state,
        Some(parent),
    );
    // Corpo del sottomenu: dentro annulla la chiusura, fuori la programa.
    if let Some(list) = child.child() {
        let motion = gtk::EventControllerMotion::new();
        motion.connect_enter({
            let state = state.clone();
            move |_, _, _| state.cancel_close()
        });
        motion.connect_leave({
            let state = state.clone();
            move |_| state.schedule_close()
        });
        list.add_controller(motion);
    }
    // Chiudere il figlio (click fuori, ESC, ...) ripristina il padre.
    let parent_on_close = parent.clone();
    child.connect_closed({
        let state = state.clone();
        move |_| {
            state.close_submenu();
            parent_on_close.popdown();
        }
    });

    // Attaccato al bordo della riga dal lato con spazio, in alto.
    let side = submenu_side(anchor.upcast_ref());
    child.set_position(side);
    let x = if matches!(side, gtk::PositionType::Left) {
        0
    } else {
        anchor.width()
    };
    child.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x, 0, 1, 1)));
    // Stacca il sottomenu dal menu con un piccolo margine.
    child.set_margin_start(6);
    child.set_margin_end(6);
    child.set_margin_top(6);
    child.set_margin_bottom(6);

    *state.submenu.borrow_mut() = Some(child.downgrade());
    *state.parent.borrow_mut() = Some(parent.downgrade());
    parent.set_autohide(false);
    child.popup();
}

/// Lato del sottomenu: a destra della riga se lo schermo ha spazio,
/// altrimenti a sinistra.
fn submenu_side(anchor: &gtk::Widget) -> gtk::PositionType {
    /// Larghezza comoda per il sottomenu.
    const NEEDED: f32 = 240.0;
    let Some(root) = anchor.root() else {
        return gtk::PositionType::Right;
    };
    let Some(rect) = anchor.compute_bounds(&root) else {
        return gtk::PositionType::Right;
    };
    let right = root.width() as f32 - (rect.x() + rect.width());
    if right < NEEDED && rect.x() > right {
        gtk::PositionType::Left
    } else {
        gtk::PositionType::Right
    }
}

/// Costruisce e apre il menu ancorato a `(x, y)` su `anchor`.
fn popup(
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
    window: &adw::ApplicationWindow,
    rows: &[Row],
    separators_after: &[usize],
) {
    let state = Rc::new(MenuState::default());
    let popover = build(anchor, rows, separators_after, window, &state, None);
    // Uscendo dal menu si chiude anche un eventuale sottomenu.
    if let Some(list) = popover.child() {
        let motion = gtk::EventControllerMotion::new();
        motion.connect_leave({
            let state = state.clone();
            move |_| state.schedule_close()
        });
        list.add_controller(motion);
    }
    popover.connect_closed({
        let state = state.clone();
        move |_| state.close_submenu()
    });
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.set_position(side_for(anchor, x));
    popover.popup();
}

/// A destra del click se lo schermo ha spazio, altrimenti a sinistra.
fn side_for(anchor: &gtk::Widget, x: f64) -> gtk::PositionType {
    /// Larghezza del menu principale.
    const NEEDED: f32 = 240.0;
    let Some(root) = anchor.root() else {
        return gtk::PositionType::Right;
    };
    let x_root = anchor
        .compute_bounds(&root)
        .map(|rect| rect.x() + x as f32)
        .unwrap_or(x as f32);
    let free = root.width() as f32 - x_root;
    if free < NEEDED && x_root > free {
        gtk::PositionType::Left
    } else {
        gtk::PositionType::Right
    }
}

/// Click destro su una voce: menu completo, la voce è già selezionata.
pub fn show(anchor: &gtk::Widget, x: f64, y: f64, window: &adw::ApplicationWindow) {
    popup(anchor, x, y, window, &ROWS, &SEPARATORS_AFTER);
}

/// Click destro su una voce del cestino: ripristina, copia, elimina.
pub fn show_trash(anchor: &gtk::Widget, x: f64, y: f64, window: &adw::ApplicationWindow) {
    popup(anchor, x, y, window, &TRASH_ROWS, &TRASH_SEPARATORS_AFTER);
}

/// Click destro sullo sfondo: crea, incolla, terminale, proprietà.
pub fn show_background(anchor: &gtk::Widget, x: f64, y: f64, window: &adw::ApplicationWindow) {
    popup(
        anchor,
        x,
        y,
        window,
        &BACKGROUND_ROWS,
        &BACKGROUND_SEPARATORS_AFTER,
    );
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
