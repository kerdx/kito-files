//! Preferences model and persistence. The dialog is in a separate module.

pub mod model;
pub mod storage;

use model::{OpenItems, Preferences, ViewMode};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

type OpenItemsListener = Rc<dyn Fn(OpenItems)>;

/// Process-wide settings state shared by every window.
pub struct PreferenceStore {
    current: Rc<RefCell<Preferences>>,
    path: Option<PathBuf>,
    open_items_listeners: RefCell<Vec<OpenItemsListener>>,
}

impl PreferenceStore {
    pub fn load() -> Rc<Self> {
        let path = storage::config_path();
        let current = path.as_deref().map(storage::load_from).unwrap_or_default();
        Rc::new(Self {
            current: Rc::new(RefCell::new(current)),
            path,
            open_items_listeners: RefCell::new(Vec::new()),
        })
    }

    #[cfg(test)]
    fn at_path(path: PathBuf) -> Rc<Self> {
        Rc::new(Self {
            current: Rc::new(RefCell::new(storage::load_from(&path))),
            path: Some(path),
            open_items_listeners: RefCell::new(Vec::new()),
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
        self.save()
    }

    pub fn subscribe_open_items(&self, listener: OpenItemsListener) {
        self.open_items_listeners.borrow_mut().push(listener);
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
}
