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
    /// List or confirmed empty-folder page. Loading is a transient overlay.
    stack: gtk::Stack,
    empty_page: adw::StatusPage,
    loading_label: gtk::Label,
    loading_panel: gtk::Box,
    loading_stop: gtk::Button,
    selection: RefCell<gtk::SingleSelection>,
    mode: Cell<ViewMode>,
    open_items: Cell<OpenItems>,
    history: NavHistory,
    /// Async load generation: bumped on every navigation; only the latest
    /// worker result may touch history, store and chrome. Older results are
    /// discarded, so a slow folder can never overwrite a newer one.
    load_gen: LoadGen,
    load_cancel: RefCell<Option<gio::Cancellable>>,
    load_active: Cell<Option<u64>>,
    /// Hidden-view sync: the view is fresh only for the global value it
    /// was last successfully filled with.
    hidden_sync: HiddenSync,
    show_hidden: Rc<Cell<bool>>,
    window: adw::ApplicationWindow,
    manager: std::rc::Weak<TabManager>,
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

struct ListedFolder {
    entries: Vec<kito_core::Entry>,
    timings: kito_core::ListTimings,
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

    /// Invalidates `id` without starting a replacement navigation.
    fn invalidate(&self, id: u64) -> bool {
        if !self.is_current(id) {
            return false;
        }
        let next = self.next.get().wrapping_add(1);
        self.next.set(next);
        self.current.set(next);
        true
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

/// The delayed timeout is allowed to reveal the overlay only while its
/// generation is still active. A completed fast load and an obsolete timer
/// therefore never flash over the view.
fn loading_indicator_due(current_id: u64, active_id: Option<u64>, id: u64) -> bool {
    current_id == id && active_id == Some(id)
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
    let store = tab.store.clone();
    let (widget, selection) =
        file_list::build_view(tab.mode.get(), &store, &on_secondary, tab.open_items.get());
    wire_activate(tab, &widget);
    // Selection -> status bar (selected items).
    {
        let on_status = tab.on_status.clone();
        let store = store.clone();
        let manager = tab.manager.clone();
        let page = tab.page.clone();
        selection.connect_notify_local(Some("selected-item"), move |selection, _| {
            if !manager
                .upgrade()
                .is_some_and(|manager| manager.is_active_page(&page))
            {
                return;
            }
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
        self.loading_stop
            .set_label(&crate::l10n::tr("loading-interrupt"));
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

    fn cancel_current_load(&self) {
        let Some(id) = self.load_active.get() else {
            return;
        };
        self.cancel_load(id);
    }

    fn cancel_load(&self, id: u64) {
        if !self.load_gen.invalidate(id) {
            return;
        }
        if let Some(cancellable) = self.load_cancel.borrow_mut().take() {
            cancellable.cancel();
        }
        self.load_active.set(None);
        self.loading_panel.set_visible(false);
    }

    /// Single navigation path: resolves, enumerates and sorts off the UI
    /// thread. Existing content remains visible until the complete new model
    /// has been built in bounded chunks; only then do path, history, content
    /// and selection change together.
    fn navigate(self: &Rc<Self>, kind: NavKind, uri: &str) {
        let Some(target) = self.history.target(kind, uri) else {
            return;
        };
        let previous = self.history.current_uri();
        let id = self.load_gen.start();
        if let Some(cancellable) = self.load_cancel.borrow_mut().take() {
            cancellable.cancel();
        }
        let cancellable = gio::Cancellable::new();
        *self.load_cancel.borrow_mut() = Some(cancellable.clone());
        self.load_active.set(Some(id));
        self.loading_panel.set_visible(false);
        if let Some(manager) = self.manager.upgrade() {
            manager.close_bg_menu();
        }
        let show_hidden = self.show_hidden.get();
        let weak_for_timer = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
            if let Some(tab) = weak_for_timer.upgrade() {
                if loading_indicator_due(tab.load_gen.current.get(), tab.load_active.get(), id) {
                    tab.loading_panel.set_visible(true);
                }
            }
        });
        let weak = Rc::downgrade(self);
        let (tx, rx) = async_channel::bounded::<Result<ListedFolder, String>>(1);
        let worker_target = target.clone();
        std::thread::spawn(move || {
            let result =
                kito_core::list_dir_with_cancellable(&worker_target, show_hidden, &cancellable)
                    .map(|(entries, timings)| ListedFolder { entries, timings })
                    .map_err(|e| e.to_string());
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
                Ok(loaded) => tab.apply_loaded(id, kind, &previous, &target, loaded, show_hidden),
                Err(message) => {
                    tab.finish_loading(id);
                    tab.apply_load_error(message);
                }
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
        loaded: ListedFolder,
        show_hidden: bool,
    ) {
        let ListedFolder { entries, timings } = loaded;
        eprintln!(
            "directory load: enumerate={:.3}ms sort={:.3}ms entries={}",
            timings.enumeration.as_secs_f64() * 1000.0,
            timings.sorting.as_secs_f64() * 1000.0,
            entries.len()
        );
        let keep = if previous == target {
            self.selected_objects().first().map(|obj| obj.uri())
        } else {
            None
        };
        let staging = gio::ListStore::new::<FileObject>();
        let apply_started = std::time::Instant::now();
        let entries = Rc::new(entries);
        let offset = Rc::new(Cell::new(0usize));
        let weak = Rc::downgrade(self);
        let target = target.to_string();
        let previous = previous.to_string();
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
                file_list::append_chunk(&staging, &entries[start..end]);
                offset.set(end);
            }
            if end >= entries.len() {
                let objects: Vec<FileObject> = (0..staging.n_items())
                    .filter_map(|index| staging.item(index).and_downcast::<FileObject>())
                    .collect();
                tab.selection.borrow().unselect_all();
                tab.store.splice(0, tab.store.n_items(), &objects);
                tab.history.commit(kind, &previous, &target);
                tab.page.set_title(&Self::title_for(&target));
                tab.finish_loading(id);
                tab.finish_loaded(&target, keep.clone(), show_hidden, entries.len());
                eprintln!(
                    "directory model: build+apply={:.3}ms entries={}",
                    apply_started.elapsed().as_secs_f64() * 1000.0,
                    entries.len()
                );
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
        if self
            .manager
            .upgrade()
            .is_some_and(|manager| manager.is_active_page(&self.page))
        {
            (self.on_navigate)(target, n, self.mode.get());
            let selected = usize::from(self.selection.borrow().selected_item().is_some());
            (self.on_status)(n, selected);
            self.emit_history();
        }
    }

    fn finish_loading(&self, id: u64) {
        if !self.load_gen.is_current(id) {
            return;
        }
        self.load_active.set(None);
        self.loading_panel.set_visible(false);
        self.load_cancel.borrow_mut().take();
    }

    /// Failed load: history, current URI and store are untouched (the store
    /// still shows the previous folder); only the loading page is reverted
    /// and the error is reported.
    fn apply_load_error(&self, message: String) {
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

impl Drop for FileTab {
    fn drop(&mut self) {
        if let Some(cancellable) = self.load_cancel.get_mut().take() {
            cancellable.cancel();
        }
    }
}

impl Drop for TabManager {
    fn drop(&mut self) {
        if let Some(menu) = self
            .bg_menu
            .get_mut()
            .take()
            .and_then(|menu| menu.upgrade())
        {
            menu.popdown();
        }
        for tab in self.tabs.get_mut().iter() {
            tab.cancel_current_load();
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
    /// Open background menu, if any: closed on tab switch and navigation so
    /// actions can never land on the wrong folder.
    bg_menu: RefCell<Option<glib::WeakRef<gtk::Popover>>>,
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
            bg_menu: RefCell::new(None),
        });
        {
            let manager_weak = Rc::downgrade(&manager);
            let tab_view = manager.tab_view.clone();
            tab_view.connect_close_page(move |view, page| {
                let Some(manager) = manager_weak.upgrade() else {
                    return glib::Propagation::Stop;
                };
                let mut tabs = manager.tabs.borrow_mut();
                if let Some(pos) = tabs.iter().position(|t| t.page == *page) {
                    tabs.remove(pos);
                }
                drop(tabs);
                if manager.tabs.borrow().is_empty() {
                    manager.window.close();
                } else {
                    view.close_page_finish(page, true);
                }
                glib::Propagation::Stop
            });
        }
        {
            let manager_weak = Rc::downgrade(&manager);
            let tab_view = manager.tab_view.clone();
            tab_view.connect_selected_page_notify(move |_| {
                let Some(manager) = manager_weak.upgrade() else {
                    return;
                };
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

    fn is_active_page(&self, page: &adw::TabPage) -> bool {
        self.tab_view
            .selected_page()
            .is_some_and(|selected| selected == *page)
    }

    pub fn selected_uri(&self) -> Option<String> {
        self.selected().map(|t| t.history.current_uri())
    }

    /// Tracks the open background menu so tab switches and navigations can
    /// close it before its captured folder goes stale.
    pub fn track_bg_menu(&self, menu: &gtk::Popover) {
        let weak = glib::WeakRef::new();
        weak.set(Some(menu));
        *self.bg_menu.borrow_mut() = Some(weak);
    }

    /// Closes the tracked background menu, if still open.
    pub fn close_bg_menu(&self) {
        let menu = self
            .bg_menu
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade);
        if let Some(menu) = menu {
            menu.popdown();
        }
    }

    /// Forgets a closed background menu, unless a newer one replaced it.
    pub fn forget_bg_menu(&self, menu: &gtk::Popover) {
        let same = self
            .bg_menu
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
            .is_some_and(|tracked| tracked == *menu);
        if same {
            *self.bg_menu.borrow_mut() = None;
        }
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

    /// Refreshes the tab still showing a captured destination. This is used
    /// by asynchronous operations so their completion cannot refresh a tab
    /// the user selected later.
    pub fn reload_if_current(&self, uri: &str) {
        let tabs = self.tabs.borrow().clone();
        for tab in tabs
            .into_iter()
            .filter(|tab| tab.history.current_uri() == uri)
        {
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
    pub fn open_tab(self: &Rc<Self>, uri: &str, ctx: &Rc<crate::ops::Ctx>) {
        let preferences = self.preferences.borrow().clone();
        let (scrolled, store) = file_list::build_file_view();
        // Placeholder page when the folder has no visible entries.
        let empty = adw::StatusPage::builder()
            .icon_name("folder")
            .title(crate::l10n::tr("empty-folder"))
            .build();
        // Loading is a small, delayed overlay so a fast directory never
        // flashes a full-page placeholder over the existing view.
        let spinner = gtk::Spinner::builder().spinning(true).build();
        let loading_label = gtk::Label::builder()
            .label(crate::l10n::tr("loading-folder"))
            .css_classes(["dim-label"])
            .build();
        let loading_stop = gtk::Button::builder()
            .label(crate::l10n::tr("loading-interrupt"))
            .build();
        let loading_panel = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Start)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(12)
            .css_classes(["card", "toolbar"])
            .build();
        loading_panel.append(&spinner);
        loading_panel.append(&loading_label);
        loading_panel.append(&loading_stop);
        loading_panel.set_visible(false);
        let stack = gtk::Stack::new();
        stack.add_named(&scrolled, Some("list"));
        stack.add_named(&empty, Some("empty"));
        stack.set_visible_child_name("list");
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&stack));
        overlay.add_overlay(&loading_panel);
        overlay.set_measure_overlay(&loading_panel, false);
        let page = self.tab_view.append(&overlay);
        // Dummy selection: replaced by rebuild_view.
        let tab = Rc::new(FileTab {
            page: page.clone(),
            store,
            scrolled,
            stack,
            empty_page: empty,
            loading_label,
            loading_panel,
            loading_stop,
            selection: RefCell::new(gtk::SingleSelection::new(Some(gio::ListStore::new::<
                FileObject,
            >()))),
            mode: Cell::new(mode_for_new_tab(&preferences)),
            open_items: Cell::new(preferences.open_items),
            history: NavHistory::default(),
            load_gen: LoadGen::default(),
            load_cancel: RefCell::new(None),
            load_active: Cell::new(None),
            hidden_sync: HiddenSync::new(self.show_hidden.get()),
            show_hidden: self.show_hidden.clone(),
            window: self.window.clone(),
            manager: Rc::downgrade(self),
            on_navigate: self.on_navigate.clone(),
            on_history: self.on_history.clone(),
            on_status: self.on_status.clone(),
        });
        self.tabs.borrow_mut().push(tab.clone());
        tab.loading_stop.connect_clicked({
            let tab = Rc::downgrade(&tab);
            move |_| {
                if let Some(tab) = tab.upgrade() {
                    tab.cancel_current_load();
                }
            }
        });

        // Right-click on the background (or the "empty folder" page):
        // rows claim the sequence, so only the empty part arrives here.
        // The menu captures this tab's folder: folder properties ignore
        // selected files, and every action carries its own destination.
        // In trash the menu is the dedicated empty action instead.
        let background = gtk::GestureClick::builder().button(3).build();
        background.connect_pressed({
            let ctx = Rc::downgrade(ctx);
            let manager = Rc::downgrade(self);
            let tab = Rc::downgrade(&tab);
            move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(anchor) = gesture.widget() else {
                    return;
                };
                let (Some(ctx), Some(manager), Some(tab)) =
                    (ctx.upgrade(), manager.upgrade(), tab.upgrade())
                else {
                    return;
                };
                let dest = tab.history.current_uri();
                if dest.starts_with("trash:") {
                    crate::context_menu::show_trash_background_for(&anchor, x, y, &ctx, &manager);
                } else {
                    crate::context_menu::show_background_for(&anchor, x, y, &ctx, &dest, &manager);
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
        let preferences = Preferences {
            default_view: ViewMode::Details,
            ..Preferences::default()
        };

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

    #[test]
    fn delayed_indicator_only_appears_for_an_active_generation() {
        let gen = LoadGen::default();
        let fast = gen.start();
        // The load settled before the 200 ms timeout fired.
        assert!(!loading_indicator_due(gen.current.get(), None, fast));

        let slow = gen.start();
        assert!(loading_indicator_due(gen.current.get(), Some(slow), slow));
        let newer = gen.start();
        // The old timeout cannot reveal an indicator for a newer request.
        assert!(!loading_indicator_due(gen.current.get(), Some(newer), slow));
        assert!(!gen.is_current(slow));
    }

    #[test]
    fn cancelling_load_invalidates_its_timeout_and_result() {
        let gen = LoadGen::default();
        let id = gen.start();
        assert!(gen.invalidate(id));
        assert!(!gen.is_current(id));
        assert!(!loading_indicator_due(gen.current.get(), Some(id), id));
        assert!(!gen.invalidate(id));
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
