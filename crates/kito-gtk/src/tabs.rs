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
    current: Rc<RefCell<String>>,
    back_stack: RefCell<Vec<String>>,
    forward_stack: RefCell<Vec<String>>,
    show_hidden: Rc<Cell<bool>>,
    window: adw::ApplicationWindow,
    on_navigate: OnNavigate,
    on_history: OnHistory,
    on_status: OnStatus,
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
            if is_trash_uri(&tab.current.borrow()) {
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
        // User navigation: record in history (reload excluded:
        // same URI, nothing to record).
        let current = self.current.borrow().clone();
        if !current.is_empty() && current != uri {
            self.back_stack.borrow_mut().push(current);
            self.forward_stack.borrow_mut().clear();
        }
        self.load_raw(uri);
    }

    fn load_raw(&self, uri: &str) {
        match file_list::reload(&self.store, uri, self.show_hidden.get()) {
            Ok(n) => {
                *self.current.borrow_mut() = uri.to_string();
                self.page.set_title(&Self::title_for(uri));
                self.stack
                    .set_visible_child_name(if n == 0 { "empty" } else { "list" });
                (self.on_navigate)(uri, n, self.mode.get());
                self.emit_history();
            }
            Err(e) => {
                let dialog = adw::AlertDialog::builder()
                    .heading("Could not open folder")
                    .body(e.to_string())
                    .build();
                dialog.add_response("ok", "Ok");
                dialog.present(Some(&self.window));
            }
        }
    }

    fn emit_history(&self) {
        (self.on_history)(
            !self.back_stack.borrow().is_empty(),
            !self.forward_stack.borrow().is_empty(),
        );
    }

    pub fn go_back(&self) {
        let Some(prev) = self.back_stack.borrow_mut().pop() else {
            return;
        };
        self.forward_stack
            .borrow_mut()
            .push(self.current.borrow().clone());
        self.load_raw(&prev);
    }

    pub fn go_forward(&self) {
        let Some(next) = self.forward_stack.borrow_mut().pop() else {
            return;
        };
        self.back_stack
            .borrow_mut()
            .push(self.current.borrow().clone());
        self.load_raw(&next);
    }

    /// Realigns pathbar + status + history + view to the tab (tab switch).
    fn sync_chrome(&self) {
        (self.on_navigate)(
            &self.current.borrow(),
            self.store.n_items() as usize,
            self.mode.get(),
        );
        let selected = usize::from(self.selection.borrow().selected_item().is_some());
        (self.on_status)(self.store.n_items() as usize, selected);
        self.emit_history();
    }

    /// Reloads the tab's current folder.
    pub fn reload(&self) {
        let uri = self.current.borrow().clone();
        if !uri.is_empty() {
            self.load(&uri);
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
        self.selected().map(|t| t.current.borrow().clone())
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

    /// Toggles hidden files and reloads the tab.
    pub fn set_show_hidden(&self, show: bool) {
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
            current: Rc::new(RefCell::new(String::new())),
            back_stack: RefCell::new(Vec::new()),
            forward_stack: RefCell::new(Vec::new()),
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
            let current = tab.current.clone();
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
