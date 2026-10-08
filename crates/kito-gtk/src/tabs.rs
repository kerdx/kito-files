//! Tabs with AdwTabView: each tab has its own view, mode, store and folder.
//! The sidebar and the buttons always operate on the selected tab.

use crate::file_list;
use crate::file_list::{FileObject, SecondaryHandler, ViewMode};
use crate::preferences::model::{OpenItems, Preferences};
use adw::prelude::*;
use gtk::{gio, glib};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Called on every navigation: updates the tab's pathbar + status + view.
pub type OnNavigate = Rc<dyn Fn(&str, usize, ViewMode)>;
/// Updates the back/forward buttons.
pub type OnHistory = Rc<dyn Fn(bool, bool)>;
/// Updates the status bar: total items, selected items.
pub type OnStatus = Rc<dyn Fn(usize, usize)>;

pub struct FileTab {
    page: adw::TabPage,
    store: gio::ListStore,
    scrolled: gtk::ScrolledWindow,
    /// List, "empty folder" or loading page.
    stack: gtk::Stack,
    empty_page: adw::StatusPage,
    loading_label: gtk::Label,
    selection: RefCell<gtk::SingleSelection>,
    mode: Cell<ViewMode>,
    open_items: Cell<OpenItems>,
    history: NavHistory,
    /// Async load generation: bumped on every navigation; only the latest
    /// worker result may touch history, store and chrome. Older results are
    /// discarded, so a slow folder can never overwrite a newer one.
    load_gen: LoadGen,
    /// Hidden-view sync: the view is fresh only for the global value it
    /// was last successfully filled with.
    hidden_sync: HiddenSync,
    show_hidden: Rc<Cell<bool>>,
    window: adw::ApplicationWindow,
    on_navigate: OnNavigate,
    on_history: OnHistory,
    on_status: OnStatus,
}

/// Generation protocol for asynchronous loads. The tab bumps the counter
/// on every navigation and each worker result carries the id it started
/// with; only `id == current` results are applied. Tab closure drops the
/// tab, so the weak upgrade fails and nothing is applied. No GTK, no GIO:
/// fully headless-testable.
#[derive(Default)]
struct LoadGen {
    next: Cell<u64>,
    current: Cell<u64>,
}

impl LoadGen {
    /// Starts a load: invalidates every previous one and returns the new id.
    fn start(&self) -> u64 {
        let id = self.next.get().wrapping_add(1);
        self.next.set(id);
        self.current.set(id);
        id
    }

    /// Whether the result with `id` is still the latest load.
    fn is_current(&self, id: u64) -> bool {
        self.current.get() == id
    }
}

/// Visible stack child after a load settles. Success shows the empty page
/// only for a confirmed empty folder; a failed very first load keeps the
/// plain list look it had before async loading existed.
fn settled_child(item_count: usize, ever_loaded: bool) -> &'static str {
    if item_count == 0 && ever_loaded {
        "empty"
    } else {
        "list"
    }
}

/// How the target was chosen: drives the history update on success.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NavKind {
    Visit,
    Back,
    Forward,
}

/// Navigation history: current URI + back/forward stacks. No GTK, no
/// GIO: fully headless-testable. `current` is shared with the
/// background gesture, which reads it without retaining the tab.
#[derive(Default)]
struct NavHistory {
    current: Rc<RefCell<String>>,
    back: RefCell<Vec<String>>,
    forward: RefCell<Vec<String>>,
}

impl NavHistory {
    /// Target URI for the action, or `None` when there is nothing to do
    /// (empty URI, empty back/forward stacks). A visit always loads,
    /// even to the current URI (refresh); `uri` is ignored otherwise.
    fn target(&self, kind: NavKind, uri: &str) -> Option<String> {
        match kind {
            NavKind::Visit => {
                if uri.is_empty() {
                    None
                } else {
                    Some(uri.to_string())
                }
            }
            NavKind::Back => self.back.borrow().last().cloned(),
            NavKind::Forward => self.forward.borrow().last().cloned(),
        }
    }

    /// History changes for a SUCCESSFUL load of `target` (`previous` is
    /// the current URI from before the load). Never runs on failure, so
    /// a failed load leaves everything untouched.
    fn commit(&self, kind: NavKind, previous: &str, target: &str) {
        match kind {
            NavKind::Visit => {
                if !previous.is_empty() && previous != target {
                    self.back.borrow_mut().push(previous.to_string());
                    self.forward.borrow_mut().clear();
                }
            }
            NavKind::Back => {
                self.back.borrow_mut().pop();
                self.forward.borrow_mut().push(previous.to_string());
            }
            NavKind::Forward => {
                self.forward.borrow_mut().pop();
                self.back.borrow_mut().push(previous.to_string());
            }
        }
        *self.current.borrow_mut() = target.to_string();
    }

    fn current_uri(&self) -> String {
        self.current.borrow().clone()
    }

    fn shared_current(&self) -> Rc<RefCell<String>> {
        self.current.clone()
    }

    fn can_go_back(&self) -> bool {
        !self.back.borrow().is_empty()
    }

    fn can_go_forward(&self) -> bool {
        !self.forward.borrow().is_empty()
    }
}

/// Runs one navigation: resolves the target, loads it, and commits the
/// history change only on success. Returns whether anything was
/// committed. Headless mirror of `FileTab::navigate`'s async discipline
/// (resolve target, commit only on successful load): the loader is
/// injectable so successes and failures are testable without widgets,
/// threads or GIO.
#[cfg(test)]
fn run_navigation(
    history: &NavHistory,
    kind: NavKind,
    uri: &str,
    load: impl FnOnce(&str) -> bool,
) -> bool {
    let Some(target) = history.target(kind, uri) else {
        return false;
    };
    let previous = history.current_uri();
    if load(&target) {
        history.commit(kind, &previous, &target);
        return true;
    }
    false
}

fn activate_at(tab: &Rc<FileTab>, pos: u32) {
    let selection = tab.selection.borrow().clone();
    let Some(obj) = selection.item(pos).and_downcast::<FileObject>() else {
        return;
    };
    tab.activate_entry(&obj.uri(), obj.is_dir());
}

fn mode_for_new_tab(preferences: &Preferences) -> ViewMode {
    preferences.default_view
}

/// Wires GTK activation (pointer or Enter) to the freshly created view.
fn wire_activate(tab: &Rc<FileTab>, widget: &gtk::Widget) {
    if let Ok(view) = widget.clone().downcast::<gtk::ColumnView>() {
        let tab = tab.clone();
        view.connect_activate(move |_, pos| activate_at(&tab, pos));
    } else if let Ok(view) = widget.clone().downcast::<gtk::GridView>() {
        let tab = tab.clone();
        view.connect_activate(move |_, pos| activate_at(&tab, pos));
    }
}

/// `true` if `uri` is the trash (GIO backend, gvfs).
fn is_trash_uri(uri: &str) -> bool {
    uri.starts_with("trash:")
}

/// Rebuilds widget + selection for the tab's mode.
/// Right-click first selects the row, then opens the menu on it.
fn rebuild_view(tab: &Rc<FileTab>) {
    let on_secondary: SecondaryHandler = Rc::new({
        let tab = tab.clone();
        move |obj: &FileObject, x: f64, y: f64, anchor: &gtk::Widget| {
            if let Some(pos) = tab.store.find(obj) {
                tab.selection.borrow().select_item(pos, true);
            }
            // In trash the menu changes: restore instead of rename.
            if is_trash_uri(&tab.history.current_uri()) {
                crate::context_menu::show_trash(anchor, x, y, &tab.window);
            } else {
                crate::context_menu::show(anchor, x, y, &tab.window);
            }
        }
    });
    let (widget, selection) = file_list::build_view(
        tab.mode.get(),
        &tab.store,
        &on_secondary,
        tab.open_items.get(),
    );
    wire_activate(tab, &widget);
    // Selection -> status bar (selected items).
    {
        let on_status = tab.on_status.clone();
        let store = tab.store.clone();
        selection.connect_notify_local(Some("selected-item"), move |selection, _| {
            let selected = usize::from(selection.selected_item().is_some());
            on_status(store.n_items() as usize, selected);
        });
    }
    *tab.selection.borrow_mut() = selection;
    tab.scrolled.set_child(Some(&widget));
}

impl FileTab {
    fn title_for(uri: &str) -> String {
        uri.trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("/")
            .to_string()
    }

    pub fn set_mode(self: &Rc<Self>, mode: ViewMode) {
        if self.mode.get() == mode {
            return;
        }
        self.mode.set(mode);
        rebuild_view(self);
        self.sync_chrome();
    }

    /// Rebuilds this tab's translated row widgets while preserving its
    /// current selection, folder, view mode, and navigation history.
    fn retranslate(self: &Rc<Self>) {
        let selected_uri = self.selected_objects().first().map(|obj| obj.uri());
        self.empty_page.set_title(&crate::l10n::tr("empty-folder"));
        self.loading_label
            .set_text(&crate::l10n::tr("loading-folder"));
        rebuild_view(self);
        if let Some(uri) = selected_uri {
            self.select_uri(&uri);
        }
    }

    fn set_open_items(&self, behavior: OpenItems) {
        self.open_items.set(behavior);
        if let Some(widget) = self.scrolled.child() {
            file_list::set_open_items(&widget, behavior);
        }
    }

    pub fn load(self: &Rc<Self>, uri: &str) {
        self.navigate(NavKind::Visit, uri);
    }

    /// Single navigation path: resolves the target, then enumerates and
    /// sorts it off the UI thread. History is committed only when the latest
    /// worker result succeeds; a failed load leaves current URI, view
    /// content and both stacks untouched and shows a dialog instead.
    /// Store and current URI always move together, so path, selection and
    /// file actions never refer to different folders, not even mid-load:
    /// stale results (newer navigation, closed tab) are discarded.
    fn navigate(self: &Rc<Self>, kind: NavKind, uri: &str) {
        let Some(target) = self.history.target(kind, uri) else {
            return;
        };
        let previous = self.history.current_uri();
        let id = self.load_gen.start();
        let show_hidden = self.show_hidden.get();
        // Discreet loading state, distinct from the "empty folder" page.
        // The store keeps the previous folder until the new one succeeds.
        self.stack.set_visible_child_name("loading");
        let weak = Rc::downgrade(self);
        let (tx, rx) = async_channel::bounded::<Result<Vec<kito_core::Entry>, String>>(1);
        let worker_target = target.clone();
        std::thread::spawn(move || {
            let result =
                kito_core::list_dir(&worker_target, show_hidden).map_err(|e| e.to_string());
            let _ = tx.send_blocking(result);
        });
        glib::spawn_future_local(async move {
            let Ok(result) = rx.recv().await else {
                return;
            };
            let Some(tab) = weak.upgrade() else {
                return;
            };
            if !tab.load_gen.is_current(id) {
                return;
            }
            match result {
                Ok(entries) => tab.apply_loaded(id, kind, &previous, &target, entries, show_hidden),
                Err(message) => tab.apply_load_error(message),
            }
        });
    }

    /// Commits a successful load and streams its rows in bounded chunks.
    fn apply_loaded(
        self: &Rc<Self>,
        id: u64,
        kind: NavKind,
        previous: &str,
        target: &str,
        entries: Vec<kito_core::Entry>,
        show_hidden: bool,
    ) {
        self.history.commit(kind, previous, target);
        self.emit_history();
        self.page.set_title(&Self::title_for(target));
        // The previous folder's selection is restored only if still listed
        // (refresh of the same folder); navigating elsewhere clears it.
        let keep = self.selected_objects().first().map(|obj| obj.uri());
        if entries.is_empty() {
            self.store.remove_all();
            self.finish_loaded(target, keep, show_hidden, 0);
            return;
        }
        // Small folders skip the idle round-trip: one bulk replace.
        if entries.len() <= file_list::LOAD_CHUNK {
            file_list::replace_all(&self.store, &entries);
            self.finish_loaded(target, keep, show_hidden, entries.len());
            return;
        }
        self.store.remove_all();
        let entries = Rc::new(entries);
        let offset = Rc::new(Cell::new(0usize));
        let weak = Rc::downgrade(self);
        let target = target.to_string();
        glib::idle_add_local(move || {
            let Some(tab) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !tab.load_gen.is_current(id) {
                return glib::ControlFlow::Break;
            }
            let start = offset.get();
            let end = (start + file_list::LOAD_CHUNK).min(entries.len());
            if start < end {
                file_list::append_chunk(&tab.store, &entries[start..end]);
                offset.set(end);
            }
            if end >= entries.len() {
                tab.finish_loaded(&target, keep.clone(), show_hidden, entries.len());
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Final chrome for a completed load: selection, freshness, stack page,
    /// pathbar and status bar.
    fn finish_loaded(&self, target: &str, keep: Option<String>, show_hidden: bool, n: usize) {
        if let Some(uri) = keep {
            self.select_uri(&uri);
        }
        self.hidden_sync.mark_refreshed(show_hidden);
        self.stack.set_visible_child_name(settled_child(n, true));
        (self.on_navigate)(target, n, self.mode.get());
        let selected = usize::from(self.selection.borrow().selected_item().is_some());
        (self.on_status)(n, selected);
    }

    /// Failed load: history, current URI and store are untouched (the store
    /// still shows the previous folder); only the loading page is reverted
    /// and the error is reported.
    fn apply_load_error(&self, message: String) {
        let ever_loaded = !self.history.current_uri().is_empty();
        self.stack
            .set_visible_child_name(settled_child(self.store.n_items() as usize, ever_loaded));
        let dialog = adw::AlertDialog::builder()
            .heading(crate::l10n::tr("error-open-folder"))
            .body(message)
            .build();
        dialog.add_response("ok", &crate::l10n::tr("dialog-ok"));
        dialog.present(Some(&self.window));
    }

    /// Selects the entry with `uri`, if listed. No-op otherwise.
    fn select_uri(&self, uri: &str) {
        let listed: Vec<String> = (0..self.store.n_items())
            .filter_map(|i| {
                self.store
                    .item(i)
                    .and_downcast::<FileObject>()
                    .map(|obj| obj.uri())
            })
            .collect();
        if let Some(pos) = find_uri_index(&listed, uri) {
            self.selection.borrow().select_item(pos, true);
        }
    }

    fn emit_history(&self) {
        (self.on_history)(self.history.can_go_back(), self.history.can_go_forward());
    }

    pub fn go_back(self: &Rc<Self>) {
        self.navigate(NavKind::Back, "");
    }

    pub fn go_forward(self: &Rc<Self>) {
        self.navigate(NavKind::Forward, "");
    }

    /// Realigns pathbar + status + history + view to the tab (tab switch).
    fn sync_chrome(&self) {
        (self.on_navigate)(
            &self.history.current_uri(),
            self.store.n_items() as usize,
            self.mode.get(),
        );
        let selected = usize::from(self.selection.borrow().selected_item().is_some());
        (self.on_status)(self.store.n_items() as usize, selected);
        self.emit_history();
    }

    /// Reloads the tab's current folder.
    pub fn reload(self: &Rc<Self>) {
        let uri = self.history.current_uri();
        if !uri.is_empty() {
            self.load(&uri);
        }
    }

    /// Applies a pending show_hidden change on selection: reloads with the
    /// current global setting through the normal async path (same-URI visit
    /// leaves history untouched). Stays stale on failure, so a later
    /// selection retries.
    fn sync_hidden_if_stale(self: &Rc<Self>) {
        if self.hidden_sync.needs_refresh(self.show_hidden.get()) {
            self.reload();
        }
    }

    /// Selected object in the tab's view (single selection).
    pub fn selected_objects(&self) -> Vec<FileObject> {
        self.selection
            .borrow()
            .selected_item()
            .and_downcast::<FileObject>()
            .into_iter()
            .collect()
    }

    /// Double-click / Enter / "Open" item: enters or launches.
    pub fn activate_entry(self: &Rc<Self>, uri: &str, is_dir: bool) {
        if is_dir {
            self.load(uri);
        } else {
            // System default app (mimeapps.list), no "open with" window.
            if let Err(e) = gio::AppInfo::launch_default_for_uri(uri, gio::AppLaunchContext::NONE) {
                eprintln!("open file: {e}");
                let dialog = adw::AlertDialog::builder()
                    .heading(crate::l10n::tr("error-open-file"))
                    .body(crate::l10n::tr_with_one(
                        "error-open-file-detail",
                        "error",
                        &e.to_string(),
                    ))
                    .build();
                dialog.add_response("ok", &crate::l10n::tr("dialog-ok"));
                dialog.present(Some(&self.window));
            }
        }
    }
}

pub struct TabManager {
    tab_view: adw::TabView,
    window: adw::ApplicationWindow,
    on_navigate: OnNavigate,
    on_history: OnHistory,
    on_status: OnStatus,
    show_hidden: Rc<Cell<bool>>,
    preferences: Rc<RefCell<Preferences>>,
    tabs: RefCell<Vec<Rc<FileTab>>>,
}

impl TabManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tab_view: adw::TabView,
        window: adw::ApplicationWindow,
        on_navigate: OnNavigate,
        on_history: OnHistory,
        on_status: OnStatus,
        show_hidden: Rc<Cell<bool>>,
        preferences: Rc<RefCell<Preferences>>,
    ) -> Rc<Self> {
        let manager = Rc::new(Self {
            tab_view,
            window,
            on_navigate,
            on_history,
            on_status,
            show_hidden,
            preferences,
            tabs: RefCell::new(Vec::new()),
        });
        {
            let manager = manager.clone();
            let window = manager.window.clone();
            let tab_view = manager.tab_view.clone();
            tab_view.connect_close_page(move |view, page| {
                let mut tabs = manager.tabs.borrow_mut();
                if let Some(pos) = tabs.iter().position(|t| t.page == *page) {
                    tabs.remove(pos);
                }
                drop(tabs);
                if manager.tabs.borrow().is_empty() {
                    window.close();
                } else {
                    view.close_page_finish(page, true);
                }
                glib::Propagation::Stop
            });
        }
        {
            let manager = manager.clone();
            let tab_view = manager.tab_view.clone();
            tab_view.connect_selected_page_notify(move |_| {
                if let Some(tab) = manager.selected() {
                    // Inactive tabs apply a pending show_hidden change
                    // first, then the active chrome is re-synced over any
                    // stray updates.
                    tab.sync_hidden_if_stale();
                    tab.sync_chrome();
                }
            });
        }
        manager
    }

    pub fn selected(&self) -> Option<Rc<FileTab>> {
        let selected = self.tab_view.selected_page()?;
        self.tabs
            .borrow()
            .iter()
            .find(|t| t.page == selected)
            .cloned()
    }

    pub fn selected_uri(&self) -> Option<String> {
        self.selected().map(|t| t.history.current_uri())
    }

    /// Loads `uri` into the selected tab (used by sidebar and up arrow).
    pub fn load_selected(&self, uri: &str) {
        if let Some(tab) = self.selected() {
            tab.load(uri);
        }
    }

    /// Reloads the selected tab (after a file operation).
    pub fn reload_selected(&self) {
        if let Some(tab) = self.selected() {
            tab.reload();
        }
    }

    /// Toggles hidden files: the active tab reloads now, background tabs
    /// apply it lazily when selected.
    pub fn set_show_hidden(&self, show: bool) {
        if self.show_hidden.get() == show {
            return;
        }
        self.show_hidden.set(show);
        self.reload_selected();
    }

    /// Sets the selected tab's view.
    pub fn set_mode(&self, mode: ViewMode) {
        if let Some(tab) = self.selected() {
            tab.set_mode(mode);
        }
    }

    /// Update pointer activation in every tab owned by this window.
    pub fn set_open_items(&self, behavior: OpenItems) {
        for tab in self.tabs.borrow().iter() {
            tab.set_open_items(behavior);
        }
    }

    /// Refreshes translated view cells and the active chrome across tabs.
    pub fn retranslate(&self) {
        for tab in self.tabs.borrow().iter() {
            tab.retranslate();
        }
        if let Some(tab) = self.selected() {
            tab.sync_chrome();
        }
    }

    pub fn go_back(&self) {
        if let Some(tab) = self.selected() {
            tab.go_back();
        }
    }

    pub fn go_forward(&self) {
        if let Some(tab) = self.selected() {
            tab.go_forward();
        }
    }

    /// Selected objects in the active tab (context menu, shortcuts).
    pub fn selected_objects(&self) -> Vec<FileObject> {
        self.selected()
            .map(|t| t.selected_objects())
            .unwrap_or_default()
    }

    /// Opens `uri` in a new tab and selects it.
    pub fn open_tab(self: &Rc<Self>, uri: &str) {
        let preferences = self.preferences.borrow().clone();
        let (scrolled, store) = file_list::build_file_view();
        // Placeholder page when the folder has no visible entries.
        let empty = adw::StatusPage::builder()
            .icon_name("folder")
            .title(crate::l10n::tr("empty-folder"))
            .build();
        // Discreet loading page, distinct from the empty-folder page.
        let spinner = gtk::Spinner::builder()
            .spinning(true)
            .width_request(48)
            .height_request(48)
            .halign(gtk::Align::Center)
            .build();
        let loading_label = gtk::Label::builder()
            .label(crate::l10n::tr("loading-folder"))
            .halign(gtk::Align::Center)
            .css_classes(["dim-label"])
            .build();
        let loading_page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        loading_page.append(&spinner);
        loading_page.append(&loading_label);
        let stack = gtk::Stack::new();
        stack.add_named(&scrolled, Some("list"));
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&loading_page, Some("loading"));
        stack.set_visible_child_name("list");
        let page = self.tab_view.append(&stack);
        // Dummy selection: replaced by rebuild_view.
        let tab = Rc::new(FileTab {
            page: page.clone(),
            store,
            scrolled,
            stack,
            empty_page: empty,
            loading_label,
            selection: RefCell::new(gtk::SingleSelection::new(Some(gio::ListStore::new::<
                FileObject,
            >()))),
            mode: Cell::new(mode_for_new_tab(&preferences)),
            open_items: Cell::new(preferences.open_items),
            history: NavHistory::default(),
            load_gen: LoadGen::default(),
            hidden_sync: HiddenSync::new(self.show_hidden.get()),
            show_hidden: self.show_hidden.clone(),
            window: self.window.clone(),
            on_navigate: self.on_navigate.clone(),
            on_history: self.on_history.clone(),
            on_status: self.on_status.clone(),
        });
        self.tabs.borrow_mut().push(tab.clone());

        // Right-click on the background (or the "empty folder" page):
        // rows claim the sequence, so only the empty part arrives here.
        // Menu with "New Folder…", in trash "Empty Trash…".
        let background = gtk::GestureClick::builder().button(3).build();
        background.connect_pressed({
            let window = self.window.clone();
            let current = tab.history.shared_current();
            move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(anchor) = gesture.widget() else {
                    return;
                };
                if is_trash_uri(&current.borrow()) {
                    crate::context_menu::show_trash_background(&anchor, x, y, &window);
                } else {
                    crate::context_menu::show_background(&anchor, x, y, &window);
                }
            }
        });
        tab.stack.add_controller(background);

        rebuild_view(&tab);

        self.tab_view.set_selected_page(&page);
        tab.load(uri);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(path: &std::path::Path) -> String {
        format!("file://{}", path.display())
    }

    fn fixture() -> (tempfile::TempDir, String, String, String) {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["A", "B", "C"] {
            std::fs::create_dir(tmp.path().join(name)).unwrap();
        }
        let a = uri(&tmp.path().join("A"));
        let b = uri(&tmp.path().join("B"));
        let c = uri(&tmp.path().join("C"));
        (tmp, a, b, c)
    }

    fn snapshot(history: &NavHistory) -> (String, Vec<String>, Vec<String>) {
        (
            history.current_uri(),
            history.back.borrow().clone(),
            history.forward.borrow().clone(),
        )
    }

    #[test]
    fn default_view_is_read_for_new_tabs_only() {
        let existing_tab_mode = Cell::new(ViewMode::Compact);
        let mut preferences = Preferences::default();
        preferences.default_view = ViewMode::Details;

        let new_tab_mode = mode_for_new_tab(&preferences);
        assert_eq!(existing_tab_mode.get(), ViewMode::Compact);
        assert_eq!(new_tab_mode, ViewMode::Details);
    }

    #[test]
    fn successful_trip_back_and_forward() {
        let (_tmp, a, b, c) = fixture();
        let history = NavHistory::default();
        assert!(run_navigation(&history, NavKind::Visit, &a, |_| true));
        assert!(run_navigation(&history, NavKind::Visit, &b, |_| true));
        assert!(run_navigation(&history, NavKind::Visit, &c, |_| true));
        assert_eq!(
            snapshot(&history),
            (c.clone(), vec![a.clone(), b.clone()], vec![])
        );
        assert!(history.can_go_back() && !history.can_go_forward());

        assert!(run_navigation(&history, NavKind::Back, "", |_| true));
        assert_eq!(
            snapshot(&history),
            (b.clone(), vec![a.clone()], vec![c.clone()])
        );
        assert!(run_navigation(&history, NavKind::Back, "", |_| true));
        assert_eq!(
            snapshot(&history),
            (a.clone(), vec![], vec![c.clone(), b.clone()])
        );
        assert!(!history.can_go_back() && history.can_go_forward());

        assert!(run_navigation(&history, NavKind::Forward, "", |_| true));
        assert_eq!(
            snapshot(&history),
            (b.clone(), vec![a.clone()], vec![c.clone()])
        );
    }

    #[test]
    fn failed_visit_keeps_history() {
        let (_tmp, a, _b, _c) = fixture();
        let history = NavHistory::default();
        assert!(run_navigation(&history, NavKind::Visit, &a, |_| true));
        // Target missing: nothing loads, nothing changes.
        assert!(!run_navigation(
            &history,
            NavKind::Visit,
            "file:///no/such/dir",
            |_| false
        ));
        assert_eq!(snapshot(&history), (a.clone(), vec![], vec![]));
    }

    #[test]
    fn back_to_removed_dir_keeps_state() {
        let (tmp, a, b, _c) = fixture();
        let history = NavHistory::default();
        assert!(run_navigation(&history, NavKind::Visit, &a, |_| true));
        assert!(run_navigation(&history, NavKind::Visit, &b, |_| true));
        std::fs::remove_dir(tmp.path().join("A")).unwrap();
        // A is gone: back fails, current/view/stacks unchanged.
        assert!(!run_navigation(&history, NavKind::Back, "", |_| false));
        assert_eq!(snapshot(&history), (b.clone(), vec![a.clone()], vec![]));
    }

    #[test]
    fn forward_to_removed_dir_keeps_state() {
        let (tmp, a, b, _c) = fixture();
        let history = NavHistory::default();
        assert!(run_navigation(&history, NavKind::Visit, &a, |_| true));
        assert!(run_navigation(&history, NavKind::Visit, &b, |_| true));
        assert!(run_navigation(&history, NavKind::Back, "", |_| true));
        std::fs::remove_dir(tmp.path().join("B")).unwrap();
        assert!(!run_navigation(&history, NavKind::Forward, "", |_| false));
        assert_eq!(snapshot(&history), (a.clone(), vec![], vec![b.clone()]));
    }

    #[test]
    fn refresh_changes_nothing() {
        let (_tmp, a, b, _c) = fixture();
        let history = NavHistory::default();
        assert!(run_navigation(&history, NavKind::Visit, &a, |_| true));
        assert!(run_navigation(&history, NavKind::Visit, &b, |_| true));
        let before = snapshot(&history);
        // Refresh succeeds: content reloads, history untouched.
        assert!(run_navigation(&history, NavKind::Visit, &b, |_| true));
        assert_eq!(snapshot(&history), before);
        // Refresh fails: still untouched.
        assert!(!run_navigation(&history, NavKind::Visit, &b, |_| false));
        assert_eq!(snapshot(&history), before);
    }

    #[test]
    fn forward_cleared_only_on_success() {
        let (_tmp, a, b, c) = fixture();
        let history = NavHistory::default();
        assert!(run_navigation(&history, NavKind::Visit, &a, |_| true));
        assert!(run_navigation(&history, NavKind::Visit, &b, |_| true));
        assert!(run_navigation(&history, NavKind::Back, "", |_| true));
        // New visit fails: forward stack survives.
        assert!(!run_navigation(&history, NavKind::Visit, &c, |_| false));
        assert_eq!(snapshot(&history), (a.clone(), vec![], vec![b.clone()]));
        // New visit succeeds: forward cleared, previous recorded.
        assert!(run_navigation(&history, NavKind::Visit, &c, |_| true));
        assert_eq!(snapshot(&history), (c.clone(), vec![a.clone()], vec![]));
    }

    #[test]
    fn histories_are_independent_per_tab() {
        let (_tmp, a, b, c) = fixture();
        let first = NavHistory::default();
        let second = NavHistory::default();
        assert!(run_navigation(&first, NavKind::Visit, &a, |_| true));
        assert!(run_navigation(&first, NavKind::Visit, &b, |_| true));
        assert!(run_navigation(&second, NavKind::Visit, &c, |_| true));
        assert_eq!(snapshot(&first), (b.clone(), vec![a.clone()], vec![]));
        assert_eq!(snapshot(&second), (c.clone(), vec![], vec![]));
        assert!(!run_navigation(&second, NavKind::Back, "", |_| true));
        assert_eq!(snapshot(&first), (b.clone(), vec![a.clone()], vec![]));
    }

    #[test]
    fn superseded_loads_are_discarded() {
        let gen = LoadGen::default();
        let first = gen.start();
        assert!(gen.is_current(first));
        let second = gen.start();
        // The first navigation is superseded before its worker finishes.
        assert!(!gen.is_current(first));
        assert!(gen.is_current(second));
        // Rapid retargeting keeps only the latest.
        let third = gen.start();
        assert!(!gen.is_current(second));
        assert!(gen.is_current(third));
    }

    /// Async mirror of the failure tests above: a Back load is superseded
    /// by a visit, the stale worker then finishes late (discarded), and the
    /// newer visit fails (no commit). History, stacks and content stay put.
    #[test]
    fn out_of_order_and_failed_results_keep_state() {
        let (_tmp, a, b, c) = fixture();
        let history = NavHistory::default();
        assert!(run_navigation(&history, NavKind::Visit, &a, |_| true));
        assert!(run_navigation(&history, NavKind::Visit, &b, |_| true));
        assert_eq!(snapshot(&history), (b.clone(), vec![a.clone()], vec![]));
        let gen = LoadGen::default();

        // Back to A starts...
        let back_id = gen.start();
        let back_target = history.target(NavKind::Back, "").unwrap();
        let back_previous = history.current_uri();
        assert_eq!(back_target, a);
        // ...but a visit to C supersedes it before any worker finishes.
        let visit_id = gen.start();
        let visit_target = history.target(NavKind::Visit, &c).unwrap();
        let visit_previous = history.current_uri();
        assert!(!gen.is_current(back_id));
        assert!(gen.is_current(visit_id));

        // Back worker finishes late with success: stale, must not commit.
        if gen.is_current(back_id) {
            history.commit(NavKind::Back, &back_previous, &back_target);
            panic!("stale load must not commit");
        }
        assert_eq!(snapshot(&history), (b.clone(), vec![a.clone()], vec![]));

        // Visit worker fails: no commit, stacks untouched.
        let visit_ok = false;
        if visit_ok && gen.is_current(visit_id) {
            history.commit(NavKind::Visit, &visit_previous, &visit_target);
        }
        assert_eq!(snapshot(&history), (b.clone(), vec![a.clone()], vec![]));

        // Retried visit succeeds and commits exactly once.
        assert!(run_navigation(&history, NavKind::Visit, &c, |_| true));
        assert_eq!(
            snapshot(&history),
            (c.clone(), vec![a.clone(), b.clone()], vec![])
        );
    }

    #[test]
    fn settled_pages_keep_loading_and_empty_distinct() {
        // Confirmed content: empty page only for a confirmed empty folder.
        assert_eq!(settled_child(0, true), "empty");
        assert_eq!(settled_child(5, true), "list");
        // Failed very first load: the plain list look, never the empty page.
        assert_eq!(settled_child(0, false), "list");
        assert_eq!(settled_child(3, false), "list");
    }

    #[test]
    fn closed_tab_results_are_dropped() {
        // apply_loaded/apply_load_error upgrade a weak tab reference first;
        // a closed tab upgrades to nothing, so late results touch no widget.
        let tab = Rc::new(7u32);
        let weak = Rc::downgrade(&tab);
        assert!(weak.upgrade().is_some());
        drop(tab);
        assert!(weak.upgrade().is_none());
    }
}

/// Per-tab hidden-view sync: the view is fresh only for the global
/// value it was last successfully filled with. Drives lazy refresh on
/// tab selection; failures keep it stale for a retry. Pure state, no
/// widgets: headless-testable.
#[derive(Default)]
struct HiddenSync {
    shown: Cell<bool>,
}

impl HiddenSync {
    fn new(shown: bool) -> Self {
        Self {
            shown: Cell::new(shown),
        }
    }

    /// True when the view still reflects an older setting.
    fn needs_refresh(&self, global: bool) -> bool {
        self.shown.get() != global
    }

    /// Marks the view fresh for `global`. Call only after a successful
    /// fill done with that same value.
    fn mark_refreshed(&self, global: bool) {
        self.shown.set(global);
    }
}

/// Index of `uri` in a listing, for restoring the selection after a
/// refresh. Pure over URIs so selection matching stays headless-testable.
fn find_uri_index(listed: &[String], uri: &str) -> Option<u32> {
    listed.iter().position(|u| u == uri).map(|i| i as u32)
}

#[cfg(test)]
mod hidden_tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("visible.txt"), b"v").unwrap();
        std::fs::write(tmp.path().join(".hidden"), b"h").unwrap();
        let uri = format!("file://{}", tmp.path().display());
        (tmp, uri)
    }

    /// Listed URIs collected exactly like `FileTab::select_uri` does.
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

    fn reload_hidden(uri: &str, show_hidden: bool) -> Vec<String> {
        let store = gio::ListStore::new::<FileObject>();
        file_list::reload(&store, uri, show_hidden).unwrap();
        listed_uris(&store)
    }

    #[test]
    fn hidden_shows_on_select() {
        let (_tmp, uri) = fixture();
        // Tab born before the toggle: stale until selected.
        let sync = HiddenSync::new(false);
        assert!(sync.needs_refresh(true));
        assert_eq!(reload_hidden(&uri, false).len(), 1);
        // On selection it refreshes with the current value...
        sync.mark_refreshed(true);
        assert!(!sync.needs_refresh(true));
        // ...and the hidden entry is listed.
        assert_eq!(reload_hidden(&uri, true).len(), 2);
    }

    #[test]
    fn hidden_hides_on_select() {
        let (_tmp, uri) = fixture();
        let sync = HiddenSync::new(true);
        assert!(sync.needs_refresh(false));
        sync.mark_refreshed(false);
        assert!(!sync.needs_refresh(false));
        assert_eq!(reload_hidden(&uri, false).len(), 1);
        assert!(reload_hidden(&uri, true).len() == 2);
    }

    #[test]
    fn new_tab_uses_current_value() {
        assert!(!HiddenSync::new(true).needs_refresh(true));
        assert!(!HiddenSync::new(false).needs_refresh(false));
    }

    #[test]
    fn last_toggle_wins_before_select() {
        let sync = HiddenSync::new(false);
        // Toggled on: stale while off-view is shown...
        assert!(sync.needs_refresh(true));
        // ...selected while on: applied...
        sync.mark_refreshed(true);
        // ...toggled off again: stale again, last value wins...
        assert!(sync.needs_refresh(false));
        // ...selected while off: applied, nothing pending.
        sync.mark_refreshed(false);
        assert!(!sync.needs_refresh(false));
    }

    #[test]
    fn hidden_only_dir_transitions_empty_page() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".only-hidden"), b"h").unwrap();
        let uri = format!("file://{}", tmp.path().display());
        // No visible entries -> the "empty" page; with hidden -> the list.
        assert!(reload_hidden(&uri, false).is_empty());
        assert_eq!(reload_hidden(&uri, true).len(), 1);
    }

    #[test]
    fn failed_refresh_stays_stale_and_retries() {
        let sync = HiddenSync::new(false);
        assert!(sync.needs_refresh(true));
        // Load failed: no mark, still stale...
        assert!(sync.needs_refresh(true));
        // ...a later selection retries and succeeds.
        sync.mark_refreshed(true);
        assert!(!sync.needs_refresh(true));
    }

    #[test]
    fn selection_matching_on_real_listing() {
        let (_tmp, uri) = fixture();
        let listed = reload_hidden(&uri, true);
        assert_eq!(listed.len(), 2);
        let visible = listed.iter().find(|u| u.ends_with("visible.txt")).unwrap();
        // Still listed after refresh: restorable at its index.
        let pos = find_uri_index(&listed, visible).expect("visible entry matches");
        assert_eq!(listed[pos as usize], *visible);
        // Gone entry: no match, selection simply clears.
        assert_eq!(find_uri_index(&listed, "file:///no/such/file.txt"), None);
        // Hidden entry is matchable when shown...
        let hidden = listed.iter().find(|u| u.ends_with(".hidden")).unwrap();
        assert!(find_uri_index(&listed, hidden).is_some());
        // ...and absent when hidden.
        let plain = reload_hidden(&uri, false);
        assert_eq!(find_uri_index(&plain, hidden), None);
    }
}
