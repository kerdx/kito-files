//! File operations: clipboard, trash, delete, rename, copy/move.
//! Copy/move/delete run in a thread; the outcome returns to the main loop
//! with toast + reload. No `%` progress for now (next step).

use crate::tabs::TabManager;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Default)]
pub struct Clipboard {
    pub uris: Vec<String>,
    pub cut: bool,
}

#[derive(Clone)]
pub struct Ctx {
    pub window: adw::ApplicationWindow,
    pub manager: Rc<TabManager>,
    pub toast: adw::ToastOverlay,
    pub clipboard: Rc<RefCell<Clipboard>>,
    /// Shows the path entry (Ctrl+L). Set by main.
    pub focus_path: Rc<dyn Fn()>,
}

impl Ctx {
    pub fn toast(&self, msg: &str) {
        self.toast.add_toast(adw::Toast::new(msg));
    }

    /// Ctrl+L: switches to path writing mode.
    pub fn focus_path(&self) {
        (self.focus_path)();
    }

    fn error_dialog(&self, heading: &str, body: String) {
        let dialog = adw::AlertDialog::builder()
            .heading(heading)
            .body(body)
            .build();
        dialog.add_response("ok", "Ok");
        dialog.present(Some(&self.window));
    }

    /// Runs `job` in a thread; `done` runs on the main loop.
    /// `done` touches GTK widgets, so it stays on the main thread: the worker
    /// sends the result via channel and a local idle delivers it.
    fn run_in_thread<F, R>(job: F, done: impl FnOnce(R) + 'static)
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
        let mut done = Some(done);
        glib::idle_add_local(move || match rx.try_recv() {
            Ok(result) => {
                if let Some(done) = done.take() {
                    done(result);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
    }

    pub fn open_selected(&self) {
        let Some(tab) = self.manager.selected() else {
            return;
        };
        let objs = tab.selected_objects();
        let Some(first) = objs.first() else {
            self.toast("Nothing selected");
            return;
        };
        tab.activate_entry(&first.uri(), first.is_dir());
    }

    /// Ctrl+C / Ctrl+X: internal state + `text/uri-list` for other apps.
    pub fn copy_selected(&self, cut: bool) {
        let uris: Vec<String> = self
            .manager
            .selected_objects()
            .iter()
            .map(|o| o.uri())
            .collect();
        if uris.is_empty() {
            self.toast("Nothing selected");
            return;
        }
        *self.clipboard.borrow_mut() = Clipboard { uris, cut };
        if let Some(display) = gdk::Display::default() {
            display
                .clipboard()
                .set_text(&self.clipboard.borrow().uris.join("\n"));
        }
        self.toast(if cut { "Cut" } else { "Copied" });
    }

    pub fn paste(&self) {
        let Some(dest) = self.manager.selected_uri() else {
            return;
        };
        let (uris, cut) = {
            let cb = self.clipboard.borrow();
            (cb.uris.clone(), cb.cut)
        };
        if uris.is_empty() {
            self.paste_from_system(dest);
            return;
        }
        if cut {
            self.clipboard.borrow_mut().uris.clear();
            self.clipboard.borrow_mut().cut = false;
        }
        let this = self.clone();
        Self::run_in_thread(
            move || {
                let mut failed = 0;
                let mut first_error = None;
                for uri in &uris {
                    let r = if cut {
                        kito_core::move_to(uri, &dest)
                    } else {
                        kito_core::copy_to(uri, &dest)
                    };
                    if let Err(e) = r {
                        failed += 1;
                        if first_error.is_none() {
                            first_error = Some(e.to_string());
                        }
                    }
                }
                (uris.len(), failed, first_error)
            },
            move |(total, failed, first_error): (usize, i32, Option<String>)| {
                this.manager.reload_selected();
                if failed == 0 {
                    this.toast(&format!("Pasted {total} item(s)"));
                } else if let Some(error) = first_error {
                    this.error_dialog("Could not paste", error);
                } else {
                    this.toast(&format!("{failed} of {total} item(s) failed"));
                }
            },
        );
    }

    /// Paste from other apps: reads `text/plain` with `file://` lines.
    /// Copy only, never move.
    fn paste_from_system(&self, dest: String) {
        let Some(display) = gdk::Display::default() else {
            self.toast("Clipboard is empty");
            return;
        };
        let this = self.clone();
        display
            .clipboard()
            .read_text_async(gio::Cancellable::NONE, move |result| match result {
                Ok(Some(text)) => {
                    let uris: Vec<String> = text
                        .lines()
                        .map(str::trim)
                        .filter(|l| l.starts_with("file://"))
                        .map(String::from)
                        .collect();
                    if uris.is_empty() {
                        this.toast("Clipboard is empty");
                        return;
                    }
                    Self::run_in_thread(
                        move || {
                            let mut failed = 0;
                            let mut first_error = None;
                            for uri in &uris {
                                if let Err(e) = kito_core::copy_to(uri, &dest) {
                                    failed += 1;
                                    if first_error.is_none() {
                                        first_error = Some(e.to_string());
                                    }
                                }
                            }
                            (uris.len(), failed, first_error)
                        },
                        move |(total, failed, first_error): (usize, i32, Option<String>)| {
                            this.manager.reload_selected();
                            if failed == 0 {
                                this.toast(&format!("Pasted {total} item(s)"));
                            } else if let Some(error) = first_error {
                                this.error_dialog("Could not paste", error);
                            } else {
                                this.toast(&format!("{failed} of {total} item(s) failed"));
                            }
                        },
                    );
                }
                _ => this.toast("Clipboard is empty"),
            });
    }

    pub fn trash_selected(&self) {
        let uris: Vec<String> = self
            .manager
            .selected_objects()
            .iter()
            .map(|o| o.uri())
            .collect();
        if uris.is_empty() {
            self.toast("Nothing selected");
            return;
        }
        let mut failed = 0;
        for uri in &uris {
            if kito_core::trash(uri).is_err() {
                failed += 1;
            }
        }
        self.manager.reload_selected();
        if failed == 0 {
            self.toast(&format!("Moved {} item(s) to trash", uris.len()));
        } else {
            self.toast(&format!("{failed} of {} item(s) failed", uris.len()));
        }
    }

    /// From the trash: restores the selected entries to the original location.
    pub fn restore_selected(&self) {
        let uris: Vec<String> = self
            .manager
            .selected_objects()
            .iter()
            .map(|o| o.uri())
            .collect();
        if uris.is_empty() {
            self.toast("Nothing selected");
            return;
        }
        let this = self.clone();
        Self::run_in_thread(
            move || {
                let mut failed = 0;
                let mut first_error = None;
                for uri in &uris {
                    if let Err(e) = kito_core::restore(uri) {
                        failed += 1;
                        if first_error.is_none() {
                            first_error = Some(e.to_string());
                        }
                    }
                }
                (uris.len(), failed, first_error)
            },
            move |(total, failed, first_error): (usize, i32, Option<String>)| {
                this.manager.reload_selected();
                if failed == 0 {
                    this.toast(&format!("Restored {total} item(s)"));
                } else if let Some(error) = first_error {
                    this.error_dialog("Could not restore", error);
                } else {
                    this.toast(&format!("{failed} of {total} item(s) not restored"));
                }
            },
        );
    }

    /// Empties the trash: confirm, then delete in background.
    pub fn empty_trash(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading("Empty the Trash?")
            .body("All items in the Trash will be permanently deleted.")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("empty", "Empty Trash");
        dialog.set_response_appearance("empty", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        let this = self.clone();
        dialog.connect_response(None, move |dlg, response| {
            dlg.close();
            if response != "empty" {
                return;
            }
            let this = this.clone();
            Self::run_in_thread(
                move || match kito_core::empty_trash() {
                    Ok((total, failed)) => (total, failed, None),
                    Err(e) => (0, 0, Some(e.to_string())),
                },
                move |(total, failed, error)| {
                    this.manager.reload_selected();
                    if let Some(error) = error {
                        this.error_dialog("Could not empty the Trash", error);
                    } else if failed == 0 {
                        this.toast(&format!("Removed {total} item(s) from the Trash"));
                    } else {
                        this.toast(&format!("{failed} of {total} item(s) not removed"));
                    }
                },
            );
        });
        dialog.present(Some(&self.window));
    }

    /// Shift+Delete: confirm and then really delete, recursively.
    pub fn delete_selected(&self) {
        let uris: Vec<String> = self
            .manager
            .selected_objects()
            .iter()
            .map(|o| o.uri())
            .collect();
        if uris.is_empty() {
            self.toast("Nothing selected");
            return;
        }
        let dialog = adw::AlertDialog::builder()
            .heading("Delete permanently?")
            .body(format!(
                "{} item(s) will be deleted. This cannot be undone.",
                uris.len()
            ))
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        let this = self.clone();
        dialog.connect_response(None, move |dlg, response| {
            dlg.close();
            if response != "delete" {
                return;
            }
            let uris = uris.clone();
            let this = this.clone();
            Self::run_in_thread(
                move || {
                    let mut failed = 0;
                    for uri in &uris {
                        if kito_core::delete_recursive(uri).is_err() {
                            failed += 1;
                        }
                    }
                    (uris.len(), failed)
                },
                move |(total, failed)| {
                    this.manager.reload_selected();
                    if failed == 0 {
                        this.toast(&format!("Deleted {total} item(s)"));
                    } else {
                        this.toast(&format!("{failed} of {total} item(s) failed"));
                    }
                },
            );
        });
        dialog.present(Some(&self.window));
    }

    /// Pin/unpin of the single selected folder (context menu).
    /// Writes to `~/.config/gtk-3.0/bookmarks`: the sidebar reloads
    /// on its own via monitor (also applies to pins made by Nautilus).
    pub fn toggle_pin(&self) {
        let objs = self.manager.selected_objects();
        if objs.len() != 1 || !objs[0].is_dir() {
            self.toast("Select a single folder to pin");
            return;
        }
        let uri = objs[0].uri();
        if kito_core::bookmarks::is_pinned(&uri) {
            match kito_core::bookmarks::unpin(&uri) {
                Ok(_) => self.toast("Unpinned from Places"),
                Err(e) => self.error_dialog("Could not unpin", e.to_string()),
            }
        } else {
            match kito_core::bookmarks::pin(&objs[0].name(), &uri) {
                Ok(_) => self.toast("Pinned to Places"),
                Err(e) => self.error_dialog("Could not pin", e.to_string()),
            }
        }
    }

    /// Dialog with a text field: `on_ok` receives the written value.
    /// The initial text stays selected, just type over it.
    fn name_dialog<F>(&self, title: &str, placeholder: &str, initial: &str, confirm: &str, on_ok: F)
    where
        F: Fn(String) + 'static,
    {
        let dialog = adw::Dialog::builder()
            .title(title)
            .content_width(360)
            .build();
        let entry = gtk::Entry::builder()
            .text(initial)
            .placeholder_text(placeholder)
            .build();
        let confirm_button = gtk::Button::builder()
            .label(confirm)
            .css_classes(["suggested-action"])
            .build();
        let cancel_button = gtk::Button::builder().label("Cancel").build();
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::End)
            .build();
        buttons.append(&cancel_button);
        buttons.append(&confirm_button);
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_start(16)
            .margin_end(16)
            .margin_top(16)
            .margin_bottom(16)
            .build();
        content.append(&entry);
        content.append(&buttons);
        dialog.set_child(Some(&content));

        let ok: Rc<dyn Fn()> = Rc::new({
            let dialog = dialog.clone();
            let entry = entry.clone();
            move || {
                let text = entry.text().to_string();
                dialog.close();
                on_ok(text);
            }
        });
        cancel_button.connect_clicked({
            let dialog = dialog.clone();
            move |_| {
                dialog.close();
            }
        });
        confirm_button.connect_clicked({
            let ok = ok.clone();
            move |_| ok()
        });
        entry.connect_activate({
            let ok = ok.clone();
            move |_| ok()
        });
        dialog.present(Some(&self.window));
        // Text selection exists only on a focused widget.
        let entry = entry.clone();
        glib::idle_add_local_once(move || {
            entry.grab_focus();
            entry.select_region(0, -1);
        });
    }

    /// F2: renames the single selected item.
    pub fn rename_selected(&self) {
        let objs = self.manager.selected_objects();
        if objs.len() != 1 {
            self.toast("Select a single item to rename");
            return;
        }
        let uri = objs[0].uri();
        let name = objs[0].name();
        let this = self.clone();
        self.name_dialog(
            "Rename",
            "File name",
            &name,
            "Rename",
            move |text| match kito_core::rename(&uri, &text) {
                Ok(_) => {
                    this.manager.reload_selected();
                    this.toast("Renamed");
                }
                Err(e) => this.error_dialog("Could not rename", e.to_string()),
            },
        );
    }

    /// New folder in the current folder (background menu, Ctrl+Shift+N).
    pub fn new_folder(&self) {
        let Some(dest) = self.manager.selected_uri() else {
            return;
        };
        let this = self.clone();
        self.name_dialog(
            "New Folder",
            "Folder name",
            "Untitled Folder",
            "Create",
            move |text| match kito_core::mkdir(&dest, &text) {
                Ok(_) => {
                    this.manager.reload_selected();
                    this.toast("Folder created");
                }
                Err(e) => this.error_dialog("Could not create folder", e.to_string()),
            },
        );
    }

    /// New file: the type chosen from the "Create" menu is only the initial
    /// suggested name, the real name is always written by the user.
    pub fn new_file(&self, template: &'static str) {
        let Some(dest) = self.manager.selected_uri() else {
            return;
        };
        if !dest.starts_with("file://") {
            self.error_dialog(
                "Cannot create files here",
                "Only local folders are supported.".to_string(),
            );
            return;
        }
        let this = self.clone();
        self.name_dialog("New File", "File name", template, "Create", move |text| {
            match kito_core::create_file(&dest, &text) {
                Ok(_) => {
                    this.manager.reload_selected();
                    this.toast(&format!("Created {text}"));
                }
                Err(e) => this.error_dialog("Could not create file", e.to_string()),
            }
        });
    }

    /// Opens the system terminal in the current folder.
    pub fn open_terminal(&self) {
        self.launch_terminal(false);
    }

    /// Opens the terminal with a root shell (`sudo -i` inside the terminal).
    pub fn open_terminal_root(&self) {
        self.launch_terminal(true);
    }

    fn launch_terminal(&self, root: bool) {
        let Some(uri) = self.manager.selected_uri() else {
            return;
        };
        // Real local path via GIO (decoded: spaces, Unicode, `%`, `#`...).
        // `None` on non-local locations: refuse with a clear message.
        let Some(path) = kito_core::uri_to_path(&uri) else {
            self.error_dialog(
                "Cannot open a terminal here",
                "Only local folders are supported.".to_string(),
            );
            return;
        };
        let heading = if root {
            "Could not open a root terminal"
        } else {
            "Could not open a terminal"
        };
        if let Err(e) = crate::terminal::open(&path, root) {
            self.error_dialog(heading, e);
        }
    }

    /// Properties: the selected entry (if just one) or the folder.
    pub fn show_properties(&self) {
        let selected = self.manager.selected_objects();
        let uri = if selected.len() == 1 {
            selected[0].uri()
        } else {
            self.manager.selected_uri().unwrap_or_default()
        };
        if uri.is_empty() {
            return;
        }
        let info = match kito_core::props(&uri) {
            Ok(info) => info,
            Err(e) => {
                self.error_dialog("Could not read properties", e.to_string());
                return;
            }
        };

        let dialog = adw::Dialog::builder()
            .title("Properties")
            .content_width(420)
            .build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_start(20)
            .margin_end(20)
            .margin_top(20)
            .margin_bottom(20)
            .build();

        // Header: big icon + name + type.
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(14)
            .build();
        let icon_name = if info.is_dir {
            "folder".to_string()
        } else {
            info.content_type
                .as_deref()
                .map(gio::content_type_get_icon)
                .and_then(|icon| icon.downcast::<gio::ThemedIcon>().ok())
                .and_then(|themed| themed.names().first().map(|n| n.to_string()))
                .unwrap_or_else(|| "text-x-generic".to_string())
        };
        header.append(&gtk::Image::from_gicon(&gio::ThemedIcon::from_names(&[
            &icon_name,
            "text-x-generic",
        ])));
        let titles = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .valign(gtk::Align::Center)
            .build();
        titles.append(
            &gtk::Label::builder()
                .label(&info.name)
                .halign(gtk::Align::Start)
                .css_classes(["title-2"])
                .build(),
        );
        titles.append(
            &gtk::Label::builder()
                .label(if info.is_dir { "Folder" } else { "File" })
                .halign(gtk::Align::Start)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        header.append(&titles);
        content.append(&header);

        // Name/value rows.
        let grid = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build();
        let type_label = match &info.content_type {
            Some(ct) if !info.is_dir => ct.clone(),
            _ => "inode/directory".to_string(),
        };
        grid.append(&prop_row("Type", &type_label));
        let size = if info.is_dir {
            // Best-effort count: big folders don't block.
            kito_core::list_dir(&uri, true)
                .map(|entries| format!("{} items", entries.len()))
                .unwrap_or_else(|_| "—".to_string())
        } else {
            crate::file_list::human_size(info.size)
        };
        grid.append(&prop_row("Size", &size));
        let location = info
            .uri
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or(&info.uri);
        grid.append(&prop_row("Location", location));
        let modified = info
            .modified
            .and_then(|t| glib::DateTime::from_unix_local(t).ok())
            .and_then(|dt| dt.format("%Y-%m-%d %H:%M").ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "—".to_string());
        grid.append(&prop_row("Modified", &modified));
        content.append(&grid);

        let close = gtk::Button::builder()
            .label("Close")
            .halign(gtk::Align::End)
            .css_classes(["suggested-action"])
            .build();
        close.connect_clicked({
            let dialog = dialog.clone();
            move |_| {
                dialog.close();
            }
        });
        content.append(&close);
        dialog.set_child(Some(&content));
        dialog.present(Some(&self.window));
    }
}

/// Label/value row for the Properties window.
fn prop_row(label: &str, value: &str) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .build();
    row.append(
        &gtk::Label::builder()
            .label(label)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .css_classes(["dim-label"])
            .build(),
    );
    row.append(
        &gtk::Label::builder()
            .label(value)
            .halign(gtk::Align::End)
            .selectable(true)
            .build(),
    );
    row
}
