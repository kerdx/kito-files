//! Preferences model and persistence. The dialog is in a separate module.

pub mod model;
pub mod storage;

use model::{DetailColumnWidths, OpenItems, Preferences, ViewMode, WindowControls};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

type OpenItemsListener = Rc<dyn Fn(OpenItems)>;
type LanguageListener = Rc<dyn Fn(kito_i18n::AppLang)>;
type WindowControlsListener = Rc<dyn Fn(WindowControls)>;
type DisplayListener = Rc<dyn Fn(Preferences)>;
type SidebarListener = Rc<dyn Fn(Preferences)>;
type BookmarkListener = Rc<dyn Fn()>;

/// Process-wide settings state shared by every window.
pub struct PreferenceStore {
    current: Rc<RefCell<Preferences>>,
    detail_width_generation: Cell<u64>,
    path: Option<PathBuf>,
    open_items_listeners: RefCell<Vec<OpenItemsListener>>,
    language_listeners: RefCell<Vec<LanguageListener>>,
    window_controls_listeners: RefCell<Vec<WindowControlsListener>>,
    display_listeners: RefCell<Vec<DisplayListener>>,
    sidebar_listeners: RefCell<Vec<SidebarListener>>,
    bookmark_listeners: RefCell<Vec<BookmarkListener>>,
}

impl PreferenceStore {
    pub fn load() -> Rc<Self> {
        let path = storage::config_path();
        if let Some(settings_path) = path.as_deref() {
            if let Err(error) = storage::migrate_custom_locations_to_bookmarks(
                settings_path,
                &kito_core::bookmarks::path(),
            ) {
                eprintln!("{}: {error}", crate::l10n::tr("error-migrate-bookmarks"));
            }
        }
        let current = path.as_deref().map(storage::load_from).unwrap_or_default();
        Rc::new(Self {
            current: Rc::new(RefCell::new(current)),
            detail_width_generation: Cell::new(0),
            path,
            open_items_listeners: RefCell::new(Vec::new()),
            language_listeners: RefCell::new(Vec::new()),
            window_controls_listeners: RefCell::new(Vec::new()),
            display_listeners: RefCell::new(Vec::new()),
            sidebar_listeners: RefCell::new(Vec::new()),
            bookmark_listeners: RefCell::new(Vec::new()),
        })
    }

    #[cfg(test)]
    fn at_path(path: PathBuf) -> Rc<Self> {
        Rc::new(Self {
            current: Rc::new(RefCell::new(storage::load_from(&path))),
            detail_width_generation: Cell::new(0),
            path: Some(path),
            open_items_listeners: RefCell::new(Vec::new()),
            language_listeners: RefCell::new(Vec::new()),
            window_controls_listeners: RefCell::new(Vec::new()),
            display_listeners: RefCell::new(Vec::new()),
            sidebar_listeners: RefCell::new(Vec::new()),
            bookmark_listeners: RefCell::new(Vec::new()),
        })
    }

    pub fn shared(&self) -> Rc<RefCell<Preferences>> {
        self.current.clone()
    }

    pub fn snapshot(&self) -> Preferences {
        self.current.borrow().clone()
    }

    pub fn set_default_view(&self, view: ViewMode) -> io::Result<()> {
        self.current.borrow_mut().default_view = view;
        self.save()
    }

    pub fn set_open_items(&self, behavior: OpenItems) -> io::Result<()> {
        self.current.borrow_mut().open_items = behavior;
        for listener in self.open_items_listeners.borrow().iter() {
            listener(behavior);
        }
        self.save()
    }

    pub fn set_terminal(&self, choice: model::TerminalChoice) -> io::Result<()> {
        self.current.borrow_mut().terminal = choice;
        self.save()
    }

    pub fn set_language(&self, language: kito_i18n::AppLang) -> io::Result<()> {
        self.current.borrow_mut().language = language;
        let result = self.save();
        for listener in self.language_listeners.borrow().iter() {
            listener(language);
        }
        result
    }

    pub fn set_window_controls_follow_system(&self, follow: bool) -> io::Result<()> {
        self.current.borrow_mut().window_controls.follow_system = follow;
        self.notify_window_controls();
        self.save()
    }

    pub fn set_window_controls_minimize(&self, show: bool) -> io::Result<()> {
        self.current.borrow_mut().window_controls.show_minimize = show;
        self.notify_window_controls();
        self.save()
    }

    pub fn set_window_controls_maximize(&self, show: bool) -> io::Result<()> {
        self.current.borrow_mut().window_controls.show_maximize = show;
        self.notify_window_controls();
        self.save()
    }

    pub fn set_window_controls_close(&self, show: bool) -> io::Result<()> {
        self.current.borrow_mut().window_controls.show_close = show;
        self.notify_window_controls();
        self.save()
    }

    pub fn set_icon_zoom(&self, zoom: u8) -> io::Result<()> {
        let zoom = zoom.clamp(model::ICON_ZOOM_MIN, model::ICON_ZOOM_MAX);
        if self.current.borrow().icon_zoom == zoom {
            return Ok(());
        }
        self.current.borrow_mut().icon_zoom = zoom;
        self.notify_display();
        self.save()
    }

    pub fn set_show_size_column(&self, show: bool) -> io::Result<()> {
        if self.current.borrow().show_size_column == show {
            return Ok(());
        }
        self.current.borrow_mut().show_size_column = show;
        self.notify_display();
        self.save()
    }

    pub fn set_show_type_column(&self, show: bool) -> io::Result<()> {
        if self.current.borrow().show_type_column == show {
            return Ok(());
        }
        self.current.borrow_mut().show_type_column = show;
        self.notify_display();
        self.save()
    }

    pub fn set_show_modified_column(&self, show: bool) -> io::Result<()> {
        if self.current.borrow().show_modified_column == show {
            return Ok(());
        }
        self.current.borrow_mut().show_modified_column = show;
        self.notify_display();
        self.save()
    }

    /// Update the live snapshot without touching the settings file and return
    /// a store-wide revision token for any deferred persistence.
    pub fn update_detail_column_widths(&self, widths: DetailColumnWidths) -> u64 {
        let generation = self
            .detail_width_generation
            .get()
            .checked_add(1)
            .expect("detail column width generation exhausted");
        self.detail_width_generation.set(generation);
        self.current.borrow_mut().detail_column_widths = widths.clamped();
        generation
    }

    /// Persist widths only if no newer in-memory update has superseded them.
    pub fn persist_detail_column_widths(
        &self,
        widths: DetailColumnWidths,
        generation: u64,
    ) -> io::Result<()> {
        let widths = widths.clamped();
        if generation != self.detail_width_generation.get()
            || widths != self.current.borrow().detail_column_widths
        {
            return Ok(());
        }
        self.save()
    }

    /// Persist the complete current snapshot (including any debounced values).
    pub fn flush(&self) -> io::Result<()> {
        self.save()
    }

    pub fn set_sidebar_hidden(&self, id: &str, hidden: bool) -> io::Result<()> {
        let previous = self.snapshot();
        let mut prefs = self.current.borrow_mut();
        if hidden && !prefs.hidden_sidebar_ids.iter().any(|item| item == id) {
            prefs.hidden_sidebar_ids.push(id.to_string());
        } else if !hidden {
            prefs.hidden_sidebar_ids.retain(|item| item != id);
        }
        drop(prefs);
        self.save_sidebar_change(previous)
    }

    pub fn restore_sidebar(&self) -> io::Result<()> {
        let previous = self.snapshot();
        self.current.borrow_mut().hidden_sidebar_ids.clear();
        self.save_sidebar_change(previous)
    }

    #[cfg(test)]
    pub fn add_sidebar_uri(&self, uri: String) -> io::Result<()> {
        let previous = self.snapshot();
        let mut prefs = self.current.borrow_mut();
        if !prefs.custom_sidebar_uris.contains(&uri) {
            prefs.custom_sidebar_uris.push(uri);
        }
        drop(prefs);
        self.save_sidebar_change(previous)
    }

    pub fn remove_sidebar_uri(&self, uri: &str) -> io::Result<()> {
        let previous = self.snapshot();
        {
            let mut preferences = self.current.borrow_mut();
            preferences.custom_sidebar_uris.retain(|item| item != uri);
            // Custom locations have their own namespace; do not unhide a
            // bookmark that happens to point at the same URI.
            let hidden_id = format!("custom:{uri}");
            preferences
                .hidden_sidebar_ids
                .retain(|item| item != &hidden_id);
        }
        self.save_sidebar_change(previous)
    }

    /// Toggle a Kito-only favorite without touching GTK/Freedesktop bookmarks.
    pub fn toggle_favorite(&self, uri: &str) -> io::Result<bool> {
        let previous = self.snapshot();
        let added = {
            let mut preferences = self.current.borrow_mut();
            if let Some(index) = preferences
                .favorite_uris
                .iter()
                .position(|item| item == uri)
            {
                preferences.favorite_uris.remove(index);
                false
            } else {
                preferences.favorite_uris.push(uri.to_string());
                true
            }
        };
        self.save_sidebar_change(previous).map(|()| added)
    }

    pub fn add_favorite(&self, uri: &str) -> io::Result<()> {
        if self.is_favorite(uri) {
            return Ok(());
        }
        let previous = self.snapshot();
        self.current
            .borrow_mut()
            .favorite_uris
            .push(uri.to_string());
        self.save_sidebar_change(previous)
    }

    pub fn is_favorite(&self, uri: &str) -> bool {
        self.current
            .borrow()
            .favorite_uris
            .iter()
            .any(|item| item == uri)
    }

    fn save_sidebar_change(&self, previous: Preferences) -> io::Result<()> {
        let result = self.save();
        if result.is_ok() {
            let snapshot = self.snapshot();
            for listener in self.sidebar_listeners.borrow().iter() {
                listener(snapshot.clone());
            }
        } else {
            *self.current.borrow_mut() = previous;
        }
        result
    }

    pub fn subscribe_sidebar(&self, listener: SidebarListener) {
        self.sidebar_listeners.borrow_mut().push(listener);
    }

    pub fn notify_bookmarks_changed(&self) {
        for listener in self.bookmark_listeners.borrow().iter() {
            listener();
        }
    }

    pub fn subscribe_bookmarks(&self, listener: BookmarkListener) {
        self.bookmark_listeners.borrow_mut().push(listener);
    }

    fn notify_window_controls(&self) {
        let controls = self.current.borrow().window_controls;
        for listener in self.window_controls_listeners.borrow().iter() {
            listener(controls);
        }
    }

    pub fn subscribe_open_items(&self, listener: OpenItemsListener) {
        self.open_items_listeners.borrow_mut().push(listener);
    }

    pub fn subscribe_language(&self, listener: LanguageListener) {
        self.language_listeners.borrow_mut().push(listener);
    }

    pub fn subscribe_window_controls(&self, listener: WindowControlsListener) {
        self.window_controls_listeners.borrow_mut().push(listener);
    }

    pub fn subscribe_display(&self, listener: DisplayListener) {
        self.display_listeners.borrow_mut().push(listener);
    }

    fn notify_display(&self) {
        let snapshot = self.snapshot();
        for listener in self.display_listeners.borrow().iter() {
            listener(snapshot.clone());
        }
    }

    fn save(&self) -> io::Result<()> {
        match &self.path {
            Some(path) => storage::save_to(path, &self.current.borrow()),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no configuration directory is available",
            )),
        }
    }
}

use std::io;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_are_shared_and_open_behavior_notifies_subscribers() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::at_path(dir.path().join("settings.conf"));
        let notified = Rc::new(RefCell::new(None));
        store.subscribe_open_items({
            let notified = notified.clone();
            Rc::new(move |value| *notified.borrow_mut() = Some(value))
        });
        store.set_open_items(OpenItems::SingleClick).unwrap();
        assert_eq!(store.shared().borrow().open_items, OpenItems::SingleClick);
        assert_eq!(*notified.borrow(), Some(OpenItems::SingleClick));
    }

    #[test]
    fn detail_width_updates_are_memory_only_until_persist_or_flush() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        let store = PreferenceStore::at_path(path.clone());
        let widths = DetailColumnWidths {
            name: 321,
            size: 91,
            kind: 251,
            modified: 191,
        };

        store.update_detail_column_widths(widths);
        assert_eq!(store.snapshot().detail_column_widths, widths);
        assert!(!path.exists());

        // Simulate application shutdown while the debounce is still pending.
        store.flush().unwrap();
        assert_eq!(storage::load_from(&path).detail_column_widths, widths);

        let latest = DetailColumnWidths {
            name: 355,
            ..widths
        };
        let latest_generation = store.update_detail_column_widths(latest);
        store
            .persist_detail_column_widths(latest, latest_generation)
            .unwrap();
        assert_eq!(storage::load_from(&path).detail_column_widths, latest);
    }

    #[test]
    fn stale_detail_width_debounce_cannot_overwrite_latest_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        let store = PreferenceStore::at_path(path.clone());
        let generation_one_widths = DetailColumnWidths {
            name: 321,
            size: 91,
            kind: 251,
            modified: 191,
        };
        let generation_one = store.update_detail_column_widths(generation_one_widths);
        store
            .persist_detail_column_widths(generation_one_widths, generation_one)
            .unwrap();

        let generation_two_widths = DetailColumnWidths {
            name: 355,
            size: 105,
            kind: 265,
            modified: 205,
        };
        let generation_two = store.update_detail_column_widths(generation_two_widths);
        assert!(generation_two > generation_one);

        // An old view's delayed timeout must not overwrite either the file or
        // the live snapshot after another view has resized the columns.
        store
            .persist_detail_column_widths(generation_one_widths, generation_one)
            .unwrap();
        assert_eq!(store.snapshot().detail_column_widths, generation_two_widths);
        assert_eq!(
            storage::load_from(&path).detail_column_widths,
            generation_one_widths
        );

        store
            .persist_detail_column_widths(generation_two_widths, generation_two)
            .unwrap();
        assert_eq!(
            storage::load_from(&path).detail_column_widths,
            generation_two_widths
        );

        // Shutdown flushes the latest in-memory snapshot even if its debounce
        // has not completed.
        let shutdown_widths = DetailColumnWidths {
            name: 380,
            ..generation_two_widths
        };
        store.update_detail_column_widths(shutdown_widths);
        store.flush().unwrap();
        assert_eq!(
            storage::load_from(&path).detail_column_widths,
            shutdown_widths
        );
    }

    #[test]
    fn sidebar_settings_persist_and_notify_all_subscribers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        let store = PreferenceStore::at_path(path.clone());
        let a = Rc::new(RefCell::new(0));
        let b = Rc::new(RefCell::new(0));
        for target in [a.clone(), b.clone()] {
            store.subscribe_sidebar({
                let target = target.clone();
                Rc::new(move |_| *target.borrow_mut() += 1)
            });
        }
        store.set_sidebar_hidden("builtin:home", true).unwrap();
        store
            .add_sidebar_uri("smb://host/share/a,b".into())
            .unwrap();
        assert_eq!((*a.borrow(), *b.borrow()), (2, 2));
        assert_eq!(
            storage::load_from(&path).hidden_sidebar_ids,
            vec!["builtin:home"]
        );
        assert_eq!(
            storage::load_from(&path).custom_sidebar_uris,
            vec!["smb://host/share/a,b"]
        );
        store.restore_sidebar().unwrap();
        store.remove_sidebar_uri("smb://host/share/a,b").unwrap();
        assert_eq!(storage::load_from(&path), Preferences::default());
    }

    #[test]
    fn sidebar_and_bookmark_notifications_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::at_path(dir.path().join("settings.conf"));
        let sidebar_count = Rc::new(RefCell::new(0));
        let bookmark_count = Rc::new(RefCell::new(0));
        store.subscribe_sidebar({
            let count = sidebar_count.clone();
            Rc::new(move |_| *count.borrow_mut() += 1)
        });
        store.subscribe_bookmarks({
            let count = bookmark_count.clone();
            Rc::new(move || *count.borrow_mut() += 1)
        });

        store.set_sidebar_hidden("test", true).unwrap();
        assert_eq!((*sidebar_count.borrow(), *bookmark_count.borrow()), (1, 0));

        store.notify_bookmarks_changed();
        assert_eq!((*sidebar_count.borrow(), *bookmark_count.borrow()), (1, 1));
    }

    #[test]
    fn favorites_are_kito_local_and_never_rewrite_shared_bookmarks() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.conf");
        let gtk_bookmarks = dir.path().join("gtk-3.0/bookmarks");
        std::fs::create_dir_all(gtk_bookmarks.parent().unwrap()).unwrap();
        let uri = "file:///tmp/shared-target";
        let bookmark_contents = format!("{uri} GTK bookmark\n");
        std::fs::write(&gtk_bookmarks, &bookmark_contents).unwrap();

        let store = PreferenceStore::at_path(settings.clone());
        let notifications = Rc::new(RefCell::new(0));
        store.subscribe_sidebar({
            let notifications = notifications.clone();
            Rc::new(move |_| *notifications.borrow_mut() += 1)
        });
        store.add_favorite(uri).unwrap();
        store.add_favorite(uri).unwrap();
        assert_eq!(*notifications.borrow(), 1);
        assert_eq!(storage::load_from(&settings).favorite_uris, [uri]);
        assert_eq!(
            std::fs::read_to_string(&gtk_bookmarks).unwrap(),
            bookmark_contents
        );
        assert!(store.is_favorite(uri));
        store.toggle_favorite(uri).unwrap();
        assert_eq!(*notifications.borrow(), 2);
        assert!(!store.is_favorite(uri));
        assert_eq!(
            std::fs::read_to_string(&gtk_bookmarks).unwrap(),
            bookmark_contents
        );
    }

    #[test]
    fn removing_custom_location_only_clears_its_namespaced_hidden_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        let store = PreferenceStore::at_path(path.clone());
        let uri = "smb://host/share/custom";
        store.add_sidebar_uri(uri.into()).unwrap();
        store
            .set_sidebar_hidden(&format!("custom:{uri}"), true)
            .unwrap();
        store
            .set_sidebar_hidden(&format!("bookmark:{uri}"), true)
            .unwrap();

        store.remove_sidebar_uri(uri).unwrap();

        let saved = storage::load_from(&path);
        assert!(!saved.custom_sidebar_uris.iter().any(|item| item == uri));
        assert!(!saved
            .hidden_sidebar_ids
            .iter()
            .any(|item| item == &format!("custom:{uri}")));
        assert!(saved
            .hidden_sidebar_ids
            .iter()
            .any(|item| item == &format!("bookmark:{uri}")));
    }

    #[test]
    fn language_changes_notify_all_open_window_subscribers() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::at_path(dir.path().join("settings.conf"));
        let notified = Rc::new(RefCell::new(Vec::new()));
        for _ in 0..2 {
            store.subscribe_language({
                let notified = notified.clone();
                Rc::new(move |language| notified.borrow_mut().push(language))
            });
        }

        store.set_language(kito_i18n::AppLang::English).unwrap();

        assert_eq!(
            *notified.borrow(),
            vec![kito_i18n::AppLang::English, kito_i18n::AppLang::English]
        );
        assert_eq!(store.snapshot().language, kito_i18n::AppLang::English);
    }

    #[test]
    fn window_controls_changes_notify_open_windows_and_new_windows_follow_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::at_path(dir.path().join("settings.conf"));
        // Two open windows subscribed.
        let first = Rc::new(RefCell::new(Vec::new()));
        let second = Rc::new(RefCell::new(Vec::new()));
        for target in [first.clone(), second.clone()] {
            store.subscribe_window_controls({
                let target = target.clone();
                Rc::new(move |controls| target.borrow_mut().push(controls))
            });
        }

        store.set_window_controls_follow_system(false).unwrap();
        store.set_window_controls_minimize(false).unwrap();

        let expected = WindowControls {
            follow_system: false,
            show_minimize: false,
            show_maximize: true,
            show_close: true,
        };
        assert_eq!(store.snapshot().window_controls, expected);
        // New windows read the same snapshot.
        assert_eq!(store.snapshot().window_controls, expected);
        for notified in [first.clone(), second.clone()] {
            let notified = notified.borrow();
            assert_eq!(notified.len(), 2);
            assert_eq!(*notified.last().unwrap(), expected);
            // Custom layout keeps the remaining buttons on the right.
            assert_eq!(
                notified.last().unwrap().decoration_layout(),
                Some(":maximize,close".to_string())
            );
        }
    }

    #[test]
    fn window_controls_auto_to_custom_to_auto_preserves_custom_choices() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::at_path(dir.path().join("settings.conf"));
        store.set_window_controls_follow_system(false).unwrap();
        store.set_window_controls_maximize(false).unwrap();
        store.set_window_controls_close(false).unwrap();
        let custom = WindowControls {
            follow_system: false,
            show_minimize: true,
            show_maximize: false,
            show_close: false,
        };
        assert_eq!(store.snapshot().window_controls, custom);

        // Temporarily back to automatic: override removed, choices kept.
        store.set_window_controls_follow_system(true).unwrap();
        let automatic = store.snapshot().window_controls;
        assert!(automatic.follow_system);
        assert_eq!(automatic.decoration_layout(), None);
        assert!(automatic.show_minimize);
        assert!(!automatic.show_maximize);
        assert!(!automatic.show_close);

        // Returning to custom restores the same layout.
        store.set_window_controls_follow_system(false).unwrap();
        assert_eq!(store.snapshot().window_controls, custom);
        assert_eq!(
            store.snapshot().window_controls.decoration_layout(),
            Some(":minimize".to_string())
        );
    }

    #[test]
    fn window_controls_allow_hiding_all_buttons() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::at_path(dir.path().join("settings.conf"));
        store.set_window_controls_follow_system(false).unwrap();
        store.set_window_controls_minimize(false).unwrap();
        store.set_window_controls_maximize(false).unwrap();
        store.set_window_controls_close(false).unwrap();
        let controls = store.snapshot().window_controls;
        assert!(!controls.follow_system);
        assert_eq!(controls.custom_layout(), ":");
        assert_eq!(controls.decoration_layout(), Some(":".to_string()));
    }
}
