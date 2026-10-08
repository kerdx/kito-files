//! File operations: clipboard, trash, delete, rename, copy/move.
//! Copy/move/delete run in a thread; the outcome returns to the main loop
//! with toast + reload. No `%` progress for now (next step).

use crate::preferences::PreferenceStore;
use crate::tabs::TabManager;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Default)]
pub struct Clipboard {
    pub uris: Vec<String>,
    pub cut: bool,
}

/// Outcome of applying a finished cut: what the completion may do
/// with the system clipboard (only when it still belongs to the op).
#[derive(Debug, PartialEq)]
enum CutFinish {
    /// Everything moved: state consumed, caller clears the system
    /// clipboard so the moved-away URIs cannot be pasted again.
    Consumed,
    /// Some entries failed: only they are kept for retry.
    Partial,
    /// Clipboard moved on meanwhile: leave the new state alone.
    Untouched,
}

/// What to paste for freshly-read system clipboard text.
#[derive(Debug, PartialEq)]
enum PasteDecision {
    /// Clipboard changed again while reading: drop the result.
    Stale,
    /// System text still matches what Kito published: use the internal
    /// entries, preserving the copy/cut intent.
    Internal { uris: Vec<String>, cut: bool },
    /// Foreign content with usable files: copy them, never move.
    External { uris: Vec<String> },
    /// No usable files: explain, never fall back to old URIs.
    Unsupported,
}

/// Clipboard tracker: internal entries stay valid only while they match
/// what Kito published. Ownership changes arrive via the GDK `changed`
/// notification (not text comparison); paste-time text matching is only
/// a safety net for missed notifications. GDK-free: fully testable.
#[derive(Default)]
pub(crate) struct ClipTracker {
    entries: Option<Clipboard>,
    published_text: Option<String>,
    generation: u64,
    own_change_pending: bool,
    /// Provider we published with: pointer identity (not text) proves
    /// a `changed` notification is ours.
    /// Provider we published with: pointer identity (not text) proves
    /// a `changed` notification is ours.
    our_provider: Option<gdk::ContentProvider>,
    /// In-flight cut move, bound to its dispatch generation: repeated
    /// pastes of the same cut refuse to start a concurrent move.
    cut_in_flight: Option<u64>,
}

impl ClipTracker {
    /// Internal copy/cut: store the entries, arm the own-change flag and
    /// return the text to publish. Bumps the generation so in-flight
    /// async reads go stale.
    fn publish(&mut self, entries: Clipboard) -> String {
        let text = entries.uris.join("\n");
        self.entries = Some(entries);
        self.published_text = Some(text.clone());
        self.own_change_pending = true;
        self.generation += 1;
        text
    }

    /// Notification for a change Kito itself published: keep the state.
    /// Idempotent: late duplicates are harmless.
    fn on_own_changed(&mut self) {
        self.own_change_pending = false;
    }

    /// Notification for a foreign change (plain text and unsupported
    /// content included): drop the internal state so nothing stale can
    /// be pasted. Bumps the generation to retire in-flight reads.
    fn on_external_changed(&mut self) {
        self.entries = None;
        self.published_text = None;
        self.own_change_pending = false;
        self.generation += 1;
    }

    /// Starts a cut move: marks this generation in flight so repeated
    /// pastes cannot duplicate it. Returns the operation generation, or
    /// `None` when a move for the current state is already running.
    fn begin_cut(&mut self) -> Option<u64> {
        if self.cut_in_flight == Some(self.generation) {
            return None;
        }
        self.cut_in_flight = Some(self.generation);
        Some(self.generation)
    }

    /// Applies a finished cut move. Always retires a matching in-flight
    /// mark; touches entries only when the clipboard still belongs to
    /// this operation (same generation): full success consumes the cut,
    /// partial success keeps just the failed entries, and anything else
    /// leaves a newer state alone. Never converts to copy.
    fn finish_cut(&mut self, op_generation: u64, failed_uris: &[String]) -> CutFinish {
        if self.cut_in_flight == Some(op_generation) {
            self.cut_in_flight = None;
        }
        if self.generation != op_generation {
            return CutFinish::Untouched;
        }
        if failed_uris.is_empty() {
            self.entries = None;
            return CutFinish::Consumed;
        }
        if let Some(entries) = self.entries.as_mut() {
            entries.uris.retain(|u| failed_uris.contains(u));
        }
        CutFinish::Partial
    }

    fn resolve(&self, text: Option<&str>, read_generation: u64) -> PasteDecision {
        if read_generation != self.generation {
            return PasteDecision::Stale;
        }
        match (&self.entries, &self.published_text, text) {
            (Some(entries), Some(published), Some(current)) if current == published => {
                PasteDecision::Internal {
                    uris: entries.uris.clone(),
                    cut: entries.cut,
                }
            }
            (_, _, Some(current)) => {
                let uris = parse_external_uris(current);
                if uris.is_empty() {
                    PasteDecision::Unsupported
                } else {
                    PasteDecision::External { uris }
                }
            }
            _ => PasteDecision::Unsupported,
        }
    }
}

/// `file://` lines from foreign clipboard text (interop with other
/// apps). Anything else is ignored by the caller.
fn parse_external_uris(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("file://"))
        .map(String::from)
        .collect()
}

#[derive(Clone)]
pub struct Ctx {
    pub window: adw::ApplicationWindow,
    pub manager: Rc<TabManager>,
    pub toast: adw::ToastOverlay,
    pub(crate) clipboard: Rc<RefCell<ClipTracker>>,
    pub preferences: Rc<PreferenceStore>,
    /// Shows the path entry (Ctrl+L). Set by main.
    pub focus_path: Rc<dyn Fn()>,
}

impl Ctx {
    /// GDK `changed` notification: our own publish keeps the state,
    /// anything else invalidates it. Ownership is the content provider
    /// identity (not text equality).
    pub(crate) fn on_clipboard_changed(&self) {
        let ours = (|| {
            let current = gdk::Display::default()?.clipboard().content()?;
            let owned = self.clipboard.borrow().our_provider.clone()?;
            Some(current.as_ptr() == owned.as_ptr())
        })()
        .unwrap_or(false);
        let mut tracker = self.clipboard.borrow_mut();
        if ours {
            tracker.on_own_changed();
        } else {
            tracker.on_external_changed();
        }
    }

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
        dialog.add_response("ok", &crate::l10n::tr("dialog-ok"));
        dialog.present(Some(&self.window));
    }

    /// Runs `job` in a thread; `done` runs on the main loop.
    /// `done` touches GTK widgets, so it stays on the main thread: the worker
    /// sends the result through a bounded async channel and a main-loop future
    /// delivers it. The main loop sleeps until the result arrives (no polling,
    /// no sleeps on the UI thread). Dropping without delivery (worker panic)
    /// simply runs nothing, as before.
    fn run_in_thread<F, R>(job: F, done: impl FnOnce(R) + 'static)
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = async_channel::bounded::<R>(1);
        std::thread::spawn(move || {
            let result = job();
            let _ = tx.send_blocking(result);
        });
        glib::spawn_future_local(async move {
            if let Ok(result) = rx.recv().await {
                done(result);
            }
        });
    }

    pub fn open_selected(&self) {
        let Some(tab) = self.manager.selected() else {
            return;
        };
        let objs = tab.selected_objects();
        let Some(first) = objs.first() else {
            self.toast(&crate::l10n::tr("toast-nothing"));
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
            self.toast(&crate::l10n::tr("toast-nothing"));
            return;
        }
        // Publish internally first, then to the system clipboard (plain
        // text stays readable by other apps, as before).
        let text = self.clipboard.borrow_mut().publish(Clipboard { uris, cut });
        if let Some(display) = gdk::Display::default() {
            let clipboard = display.clipboard();
            clipboard.set_text(&text);
            self.clipboard.borrow_mut().our_provider = clipboard.content();
        } else {
            // Nothing published to the system: no owned provider.
            self.clipboard.borrow_mut().our_provider = None;
        }
        self.toast(&crate::l10n::tr(if cut {
            "toast-cut"
        } else {
            "toast-copied"
        }));
    }

    pub fn paste(&self) {
        let Some(dest) = self.manager.selected_uri() else {
            return;
        };
        let Some(display) = gdk::Display::default() else {
            self.toast(&crate::l10n::tr("clip-empty"));
            return;
        };
        // The system clipboard decides: read it first, then resolve
        // against the internal state captured now. A change in between
        // retires this read instead of pasting stale entries.
        let generation = self.clipboard.borrow().generation;
        let this = self.clone();
        display
            .clipboard()
            .read_text_async(gio::Cancellable::NONE, move |result| {
                let text = result.ok().flatten().map(|s| s.to_string());
                match this.clipboard.borrow().resolve(text.as_deref(), generation) {
                    PasteDecision::Stale => this.toast(&crate::l10n::tr("clip-changed")),
                    PasteDecision::Internal { uris, cut } => {
                        // Cuts move asynchronously: keep the state until
                        // completion, and refuse concurrent duplicate moves.
                        let cut_op = if cut {
                            match this.clipboard.borrow_mut().begin_cut() {
                                Some(op_generation) => Some(op_generation),
                                None => {
                                    this.toast(&crate::l10n::tr("clip-moving"));
                                    return;
                                }
                            }
                        } else {
                            None
                        };
                        this.paste_uris(uris, cut, dest, cut_op);
                    }
                    PasteDecision::External { uris } => {
                        this.paste_uris(uris, false, dest, None);
                    }
                    PasteDecision::Unsupported => {
                        if text.as_ref().is_none_or(|t| t.trim().is_empty()) {
                            this.toast(&crate::l10n::tr("clip-empty"));
                        } else {
                            this.toast(&crate::l10n::tr("clip-nofiles"));
                        }
                    }
                }
            });
    }

    /// Copies (or moves, for a cut) `uris` into `dest`. `cut_op` is the
    /// dispatch generation of a cut move, applied at completion.
    fn paste_uris(&self, uris: Vec<String>, cut: bool, dest: String, cut_op: Option<u64>) {
        if uris.is_empty() {
            self.toast(&crate::l10n::tr("clip-empty"));
            return;
        }
        let this = self.clone();
        Self::run_in_thread(
            move || {
                let mut failed = 0;
                let mut failed_uris = Vec::new();
                let mut first_error = None;
                for uri in &uris {
                    let r = if cut {
                        kito_core::move_to(uri, &dest)
                    } else {
                        kito_core::copy_to(uri, &dest)
                    };
                    if let Err(e) = r {
                        failed += 1;
                        if cut {
                            failed_uris.push(uri.clone());
                        }
                        if first_error.is_none() {
                            first_error = Some(e.to_string());
                        }
                    }
                }
                (uris.len(), failed, failed_uris, first_error)
            },
            move |(total, failed, failed_uris, first_error): (
                usize,
                i32,
                Vec<String>,
                Option<String>,
            )| {
                if let Some(op_generation) = cut_op {
                    match this
                        .clipboard
                        .borrow_mut()
                        .finish_cut(op_generation, &failed_uris)
                    {
                        CutFinish::Consumed => this.clear_system_clipboard(),
                        CutFinish::Partial | CutFinish::Untouched => {}
                    }
                }
                this.manager.reload_selected();
                if failed == 0 {
                    this.toast(&crate::l10n::tr_num("pasted-items", total as u64));
                } else if let Some(error) = first_error {
                    this.error_dialog(&crate::l10n::tr("error-paste"), error);
                } else {
                    this.toast(&crate::l10n::tr_with_two_counts(
                        "paste-failed",
                        failed as u64,
                        total as u64,
                    ));
                }
            },
        );
    }

    /// Empties the system clipboard after a fully consumed cut, so the
    /// moved-away URIs cannot be pasted again. Called only when the
    /// clipboard still belongs to the completed operation.
    fn clear_system_clipboard(&self) {
        if let Some(display) = gdk::Display::default() {
            let clipboard = display.clipboard();
            clipboard.set_text("");
            self.clipboard.borrow_mut().our_provider = clipboard.content();
        }
    }

    pub fn trash_selected(&self) {
        let uris: Vec<String> = self
            .manager
            .selected_objects()
            .iter()
            .map(|o| o.uri())
            .collect();
        if uris.is_empty() {
            self.toast(&crate::l10n::tr("toast-nothing"));
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
            self.toast(&crate::l10n::tr_num("moved-trash", uris.len() as u64));
        } else {
            self.toast(&crate::l10n::tr_with_two_counts(
                "moved-trash-failed",
                failed as u64,
                uris.len() as u64,
            ));
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
            self.toast(&crate::l10n::tr("toast-nothing"));
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
                    this.toast(&crate::l10n::tr_num("restored-items", total as u64));
                } else if let Some(error) = first_error {
                    this.error_dialog(&crate::l10n::tr("error-restore"), error);
                } else {
                    this.toast(&crate::l10n::tr_with_two_counts(
                        "restored-failed",
                        failed as u64,
                        total as u64,
                    ));
                }
            },
        );
    }

    /// Empties the trash: confirm, then delete in background.
    pub fn empty_trash(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading(crate::l10n::tr("trash-empty-title"))
            .body(crate::l10n::tr("trash-empty-body"))
            .build();
        dialog.add_response("cancel", &crate::l10n::tr("dialog-cancel"));
        dialog.add_response("empty", &crate::l10n::tr("trash-empty-confirm"));
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
                        this.error_dialog(&crate::l10n::tr("error-trash"), error);
                    } else if failed == 0 {
                        this.toast(&crate::l10n::tr_num("trash-removed", total as u64));
                    } else {
                        this.toast(&crate::l10n::tr_with_two_counts(
                            "trash-remove-failed",
                            failed as u64,
                            total as u64,
                        ));
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
            self.toast(&crate::l10n::tr("toast-nothing"));
            return;
        }
        let dialog = adw::AlertDialog::builder()
            .heading(crate::l10n::tr("delete-title"))
            .body(crate::l10n::tr_num("delete-body", uris.len() as u64))
            .build();
        dialog.add_response("cancel", &crate::l10n::tr("dialog-cancel"));
        dialog.add_response("delete", &crate::l10n::tr("delete-confirm"));
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
                        this.toast(&crate::l10n::tr_num("deleted-items", total as u64));
                    } else {
                        this.toast(&crate::l10n::tr_with_two_counts(
                            "delete-failed-items",
                            failed as u64,
                            total as u64,
                        ));
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
            self.toast(&crate::l10n::tr("pin-select-folder"));
            return;
        }
        let uri = objs[0].uri();
        if kito_core::bookmarks::is_pinned(&uri) {
            match kito_core::bookmarks::unpin(&uri) {
                Ok(_) => self.toast(&crate::l10n::tr("pin-unpinned")),
                Err(e) => self.error_dialog(&crate::l10n::tr("error-unpin"), e.to_string()),
            }
        } else {
            match kito_core::bookmarks::pin(&objs[0].name(), &uri) {
                Ok(_) => self.toast(&crate::l10n::tr("pin-pinned")),
                Err(e) => self.error_dialog(&crate::l10n::tr("error-pin"), e.to_string()),
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
        let cancel_button = gtk::Button::builder()
            .label(crate::l10n::tr("dialog-cancel"))
            .build();
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
            self.toast(&crate::l10n::tr("rename-select"));
            return;
        }
        let uri = objs[0].uri();
        let name = objs[0].name();
        let this = self.clone();
        self.name_dialog(
            &crate::l10n::tr("rename-title"),
            &crate::l10n::tr("rename-placeholder"),
            &name,
            &crate::l10n::tr("rename-confirm"),
            move |text| match kito_core::rename(&uri, &text) {
                Ok(_) => {
                    this.manager.reload_selected();
                    this.toast(&crate::l10n::tr("renamed-ok"));
                }
                Err(e) => this.error_dialog(&crate::l10n::tr("error-rename"), e.to_string()),
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
            &crate::l10n::tr("new-folder-title"),
            &crate::l10n::tr("new-folder-placeholder"),
            &crate::l10n::tr("new-folder-initial"),
            &crate::l10n::tr("new-folder-confirm"),
            move |text| match kito_core::mkdir(&dest, &text) {
                Ok(_) => {
                    this.manager.reload_selected();
                    this.toast(&crate::l10n::tr("folder-created"));
                }
                Err(e) => this.error_dialog(&crate::l10n::tr("error-create-folder"), e.to_string()),
            },
        );
    }

    /// New file: the type chosen from the "Create" menu is only the initial
    /// suggested name, the real name is always written by the user.
    pub fn new_file(&self, template: String) {
        let Some(dest) = self.manager.selected_uri() else {
            return;
        };
        if !dest.starts_with("file://") {
            self.error_dialog(
                &crate::l10n::tr("term-cannot-here"),
                crate::l10n::tr("term-local-only"),
            );
            return;
        }
        let this = self.clone();
        self.name_dialog(
            &crate::l10n::tr("new-file-title"),
            &crate::l10n::tr("new-file-placeholder"),
            &template,
            &crate::l10n::tr("new-file-confirm"),
            move |text| match kito_core::create_file(&dest, &text) {
                Ok(_) => {
                    this.manager.reload_selected();
                    this.toast(&crate::l10n::tr_with_one("created-file", "name", &text));
                }
                Err(e) => this.error_dialog(&crate::l10n::tr("error-create-file"), e.to_string()),
            },
        );
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
                &crate::l10n::tr("term-cannot-here"),
                crate::l10n::tr("term-local-only"),
            );
            return;
        };
        let heading = if root {
            "error-terminal-root"
        } else {
            "error-terminal"
        };
        let choice = self.preferences.snapshot().terminal;
        if let Err(e) = crate::terminal::open(&path, root, &choice) {
            self.error_dialog(&crate::l10n::tr(heading), e.user_message());
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
                self.error_dialog(&crate::l10n::tr("error-props"), e.to_string());
                return;
            }
        };

        let dialog = adw::Dialog::builder()
            .title(crate::l10n::tr("props-title"))
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
                .label(crate::l10n::tr(if info.is_dir {
                    "props-folder"
                } else {
                    "props-file"
                }))
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
            _ => crate::l10n::tr("props-folder"),
        };
        grid.append(&prop_row(&crate::l10n::tr("props-type"), &type_label));
        let size = if info.is_dir {
            // Best-effort count: big folders don't block.
            kito_core::list_dir(&uri, true)
                .map(|entries| crate::l10n::tr_num("props-size-items", entries.len() as u64))
                .unwrap_or_else(|_| "—".to_string())
        } else {
            crate::file_list::human_size(info.size)
        };
        grid.append(&prop_row(&crate::l10n::tr("props-size"), &size));
        let location = info
            .uri
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or(&info.uri);
        grid.append(&prop_row(&crate::l10n::tr("props-location"), location));
        let modified = info
            .modified
            .and_then(|t| glib::DateTime::from_unix_local(t).ok())
            .and_then(|dt| dt.format("%Y-%m-%d %H:%M").ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "—".to_string());
        grid.append(&prop_row(&crate::l10n::tr("props-modified"), &modified));
        content.append(&grid);

        let close = gtk::Button::builder()
            .label(crate::l10n::tr("props-close"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Delivery through the channel future. One test (not two) so parallel
    /// test threads never contend for default-context ownership.
    #[test]
    fn run_in_thread_delivers_values_and_errors_on_the_main_loop() {
        use std::time::{Duration, Instant};

        let context = glib::MainContext::default();
        let pump = |slot: &Rc<RefCell<Option<String>>>| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while slot.borrow().is_none() {
                assert!(
                    Instant::now() < deadline,
                    "worker result was never delivered"
                );
                context.iteration(true);
            }
        };

        let done = Rc::new(RefCell::new(None));
        let done2 = done.clone();
        Ctx::run_in_thread(
            || 41 + 1,
            move |value: i32| *done2.borrow_mut() = Some(value.to_string()),
        );
        pump(&done);
        assert_eq!(done.borrow().as_deref(), Some("42"));

        let failed = Rc::new(RefCell::new(None));
        let failed2 = failed.clone();
        Ctx::run_in_thread(
            || "boom".to_string(),
            move |error: String| *failed2.borrow_mut() = Some(error),
        );
        pump(&failed);
        assert_eq!(failed.borrow().as_deref(), Some("boom"));
    }

    fn entries(uris: &[&str], cut: bool) -> Clipboard {
        Clipboard {
            uris: uris.iter().map(|s| s.to_string()).collect(),
            cut,
        }
    }

    const A: &str = "file:///tmp/A.txt";
    const B: &str = "file:///tmp/B.txt";

    #[test]
    fn external_copy_replaces_internal() {
        let mut clip = ClipTracker::default();
        let published_a = clip.publish(entries(&[A], false));
        clip.on_own_changed();
        // Another app publishes B afterwards.
        clip.on_external_changed();
        let published_b = format!("{B}\n");
        assert_eq!(
            clip.resolve(Some(&published_b), clip.generation),
            PasteDecision::External {
                uris: vec![B.to_string()]
            }
        );
        // A is gone: never pasted from stale state.
        assert_ne!(published_a, published_b);
    }

    #[test]
    fn external_plain_text_pastes_nothing() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], false));
        clip.on_own_changed();
        clip.on_external_changed();
        assert_eq!(
            clip.resolve(Some("just some text"), clip.generation),
            PasteDecision::Unsupported
        );
        assert_eq!(
            clip.resolve(None, clip.generation),
            PasteDecision::Unsupported
        );
    }

    #[test]
    fn successful_move_consumes_cut() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A, B], true));
        clip.on_own_changed();
        let op = clip.begin_cut().expect("first move dispatches");
        assert_eq!(clip.finish_cut(op, &[]), CutFinish::Consumed);
        // Consumed: no retry, not even from the old text.
        assert_eq!(
            clip.resolve(Some(&published), clip.generation),
            PasteDecision::External {
                uris: vec![A.to_string(), B.to_string()]
            }
        );
    }

    #[test]
    fn failed_move_retries_as_move() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A, B], true));
        clip.on_own_changed();
        let op = clip.begin_cut().expect("first move dispatches");
        assert_eq!(
            clip.finish_cut(op, &[A.to_string(), B.to_string()]),
            CutFinish::Partial
        );
        // Nothing moved: the next paste retries the full move, never a copy.
        assert_eq!(
            clip.resolve(Some(&published), clip.generation),
            PasteDecision::Internal {
                uris: vec![A.to_string(), B.to_string()],
                cut: true,
            }
        );
    }

    #[test]
    fn partial_move_retries_only_failed() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A, B], true));
        clip.on_own_changed();
        let op = clip.begin_cut().expect("first move dispatches");
        assert_eq!(clip.finish_cut(op, &[B.to_string()]), CutFinish::Partial);
        assert_eq!(
            clip.resolve(Some(&published), clip.generation),
            PasteDecision::Internal {
                uris: vec![B.to_string()],
                cut: true,
            }
        );
    }

    #[test]
    fn new_publish_during_move_stays_intact() {
        let mut clip = ClipTracker::default();
        let published_a = clip.publish(entries(&[A], true));
        let op = clip.begin_cut().expect("first move dispatches");
        // New copy while the old move runs: independent state.
        let published_b = clip.publish(entries(&[B], false));
        clip.on_own_changed();
        // Old completion touches nothing (not even with full success).
        assert_eq!(clip.finish_cut(op, &[]), CutFinish::Untouched);
        assert_eq!(
            clip.resolve(Some(&published_b), clip.generation),
            PasteDecision::Internal {
                uris: vec![B.to_string()],
                cut: false,
            }
        );
        assert_ne!(published_a, published_b);
    }

    #[test]
    fn external_replace_during_move_is_not_restored() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        let op = clip.begin_cut().expect("first move dispatches");
        clip.on_external_changed();
        assert_eq!(clip.finish_cut(op, &[]), CutFinish::Untouched);
        // Old cut is gone for good: foreign content pastes as copy.
        assert_eq!(
            clip.resolve(Some(B), clip.generation),
            PasteDecision::External {
                uris: vec![B.to_string()]
            }
        );
    }

    #[test]
    fn repeated_paste_during_move_dispatches_once() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        clip.on_own_changed();
        let op = clip.begin_cut().expect("first move dispatches");
        // Second paste of the same cut: refused, no duplicate move.
        assert_eq!(clip.begin_cut(), None);
        // After completion the guard is released.
        assert_eq!(clip.finish_cut(op, &[]), CutFinish::Consumed);
        clip.publish(entries(&[A], true));
        assert!(clip.begin_cut().is_some());
    }

    #[test]
    fn external_replace_invalidates_pending_cut() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        clip.on_own_changed();
        clip.on_external_changed();
        // Old cut is gone: B pastes as copy, never as move.
        assert_eq!(
            clip.resolve(Some(B), clip.generation),
            PasteDecision::External {
                uris: vec![B.to_string()]
            }
        );
    }

    #[test]
    fn own_publish_notification_keeps_state_valid() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A], false));
        // Notification caused by our own publish: still valid...
        clip.on_own_changed();
        assert_eq!(
            clip.resolve(Some(&published), clip.generation),
            PasteDecision::Internal {
                uris: vec![A.to_string()],
                cut: false,
            }
        );
        // ...even if delivered twice.
        clip.on_own_changed();
        assert_eq!(
            clip.resolve(Some(&published), clip.generation),
            PasteDecision::Internal {
                uris: vec![A.to_string()],
                cut: false,
            }
        );
    }

    #[test]
    fn async_read_after_change_is_discarded() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A], false));
        let read_generation = clip.generation;
        // Clipboard replaced while the async read was in flight.
        clip.on_external_changed();
        assert_eq!(
            clip.resolve(Some(&published), read_generation),
            PasteDecision::Stale
        );
        // A fresh read at the current generation resolves normally.
        assert_eq!(
            clip.resolve(Some(B), clip.generation),
            PasteDecision::External {
                uris: vec![B.to_string()]
            }
        );
    }

    #[test]
    fn external_uris_filter_file_lines() {
        assert_eq!(
            parse_external_uris("  file:///tmp/a.txt\nnot a file\nfile:///tmp/b.txt  "),
            vec![
                "file:///tmp/a.txt".to_string(),
                "file:///tmp/b.txt".to_string()
            ]
        );
        assert!(parse_external_uris("plain text\n").is_empty());
    }
}
