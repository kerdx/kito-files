//! Tabs with AdwTabView: each tab has its own view, mode, store and folder.
//! The sidebar and the buttons always operate on the selected tab.

use crate::file_list;
use crate::file_list::{FileObject, SecondaryHandler, ViewMode};
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
    /// List or "empty folder" page.
    stack: gtk::Stack,
    selection: RefCell<gtk::SingleSelection>,
    mode: Cell<ViewMode>,
    history: NavHistory,
    /// Hidden-view sync: the view is fresh only for the global value it
    /// was last successfully filled with.
    hidden_sync: HiddenSync,
    show_hidden: Rc<Cell<bool>>,
    window: adw::ApplicationWindow,
    on_navigate: OnNavigate,
    on_history: OnHistory,
    on_status: OnStatus,
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
/// committed. The loader is injectable so successes and failures are
/// testable without widgets or GIO.
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

/// Wires double-click / Enter to the freshly created view.
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
    let (widget, selection) = file_list::build_view(tab.mode.get(), &tab.store, &on_secondary);
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

    pub fn load(&self, uri: &str) {
        self.navigate(NavKind::Visit, uri);
    }

    /// Single navigation path: resolves the target, loads it, and only
    /// on success commits the history change and refreshes the buttons.
    /// A failed load leaves current URI, view content and both stacks
    /// untouched; the error dialog comes from `load_raw`.
    fn navigate(&self, kind: NavKind, uri: &str) {
        if run_navigation(&self.history, kind, uri, |target| self.load_raw(target)) {
            self.emit_history();
        }
    }

    /// Loads `uri` into the view, updating current URI, title, view and
    /// chrome on success. Returns whether it worked; shows a dialog on
    /// failure without touching any state.
    fn load_raw(&self, uri: &str) -> bool {
        match self.fill_store(uri) {
            Ok(n) => {
                self.page.set_title(&Self::title_for(uri));
                self.stack
                    .set_visible_child_name(if n == 0 { "empty" } else { "list" });
                (self.on_navigate)(uri, n, self.mode.get());
                let selected = usize::from(self.selection.borrow().selected_item().is_some());
                (self.on_status)(n, selected);
                true
            }
            Err(e) => {
                let dialog = adw::AlertDialog::builder()
                    .heading("Could not open folder")
                    .body(e.to_string())
                    .build();
                dialog.add_response("ok", "Ok");
                dialog.present(Some(&self.window));
                false
            }
        }
    }

    /// Refills the store for `uri` with the current global setting,
    /// restoring the selection when its entry is still listed. Shared by
    /// navigation and quiet refresh: no chrome, no dialog.
    fn fill_store(&self, uri: &str) -> Result<usize, glib::Error> {
        let keep = self.selected_objects().first().map(|obj| obj.uri());
        let n = file_list::reload(&self.store, uri, self.show_hidden.get())?;
        if let Some(uri) = keep {
            self.select_uri(&uri);
        }
        self.hidden_sync.mark_refreshed(self.show_hidden.get());
        Ok(n)
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

    pub fn go_back(&self) {
        self.navigate(NavKind::Back, "");
    }

    pub fn go_forward(&self) {
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
    pub fn reload(&self) {
        let uri = self.history.current_uri();
        if !uri.is_empty() {
            self.load(&uri);
        }
    }

    /// Applies a pending show_hidden change on selection, quietly: no
    /// title, crumbs, buttons or status updates (the caller re-syncs the
    /// active tab right after). Stays stale and silent on failure, so a
    /// later selection retries.
    fn sync_hidden_if_stale(&self) {
        if self.hidden_sync.needs_refresh(self.show_hidden.get()) {
            self.refresh_hidden();
        }
    }

    /// Reloads content for the current global setting. Marks the view
    /// fresh only on success; leaves everything else (directory,
    /// history, mode) alone.
    fn refresh_hidden(&self) {
        let uri = self.history.current_uri();
        if uri.is_empty() {
            return;
        }
        if let Ok(n) = self.fill_store(&uri) {
            self.stack
                .set_visible_child_name(if n == 0 { "empty" } else { "list" });
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
    pub fn activate_entry(&self, uri: &str, is_dir: bool) {
        if is_dir {
            self.load(uri);
        } else {
            // System default app (mimeapps.list), no "open with" window.
            if let Err(e) = gio::AppInfo::launch_default_for_uri(uri, gio::AppLaunchContext::NONE) {
                eprintln!("open file: {e}");
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
    ) -> Rc<Self> {
        let manager = Rc::new(Self {
            tab_view,
            window,
            on_navigate,
            on_history,
            on_status,
            show_hidden,
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
        let (scrolled, store) = file_list::build_file_view();
        // Placeholder page when the folder has no visible entries.
        let empty = adw::StatusPage::builder()
            .icon_name("folder")
            .title("This folder is empty")
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&scrolled, Some("list"));
        stack.add_named(&empty, Some("empty"));
        stack.set_visible_child_name("list");
        let page = self.tab_view.append(&stack);
        // Dummy selection: replaced by rebuild_view.
        let tab = Rc::new(FileTab {
            page: page.clone(),
            store,
            scrolled,
            stack,
            selection: RefCell::new(gtk::SingleSelection::new(Some(gio::ListStore::new::<
                FileObject,
            >()))),
            mode: Cell::new(ViewMode::default()),
            history: NavHistory::default(),
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
        let (_tmp, a, b, _c) = fixture();
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
