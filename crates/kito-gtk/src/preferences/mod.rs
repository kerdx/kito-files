//! Preferences model and persistence. The dialog is in a separate module.

pub mod model;
pub mod storage;

use model::{OpenItems, Preferences, ViewMode, WindowControls};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

type OpenItemsListener = Rc<dyn Fn(OpenItems)>;
type LanguageListener = Rc<dyn Fn(kito_i18n::AppLang)>;
type WindowControlsListener = Rc<dyn Fn(WindowControls)>;

/// Process-wide settings state shared by every window.
pub struct PreferenceStore {
    current: Rc<RefCell<Preferences>>,
    path: Option<PathBuf>,
    open_items_listeners: RefCell<Vec<OpenItemsListener>>,
    language_listeners: RefCell<Vec<LanguageListener>>,
    window_controls_listeners: RefCell<Vec<WindowControlsListener>>,
}

impl PreferenceStore {
    pub fn load() -> Rc<Self> {
        let path = storage::config_path();
        let current = path.as_deref().map(storage::load_from).unwrap_or_default();
        Rc::new(Self {
            current: Rc::new(RefCell::new(current)),
            path,
            open_items_listeners: RefCell::new(Vec::new()),
            language_listeners: RefCell::new(Vec::new()),
            window_controls_listeners: RefCell::new(Vec::new()),
        })
    }

    #[cfg(test)]
    fn at_path(path: PathBuf) -> Rc<Self> {
        Rc::new(Self {
            current: Rc::new(RefCell::new(storage::load_from(&path))),
            path: Some(path),
            open_items_listeners: RefCell::new(Vec::new()),
            language_listeners: RefCell::new(Vec::new()),
            window_controls_listeners: RefCell::new(Vec::new()),
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
