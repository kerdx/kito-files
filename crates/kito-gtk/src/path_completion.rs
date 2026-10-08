//! Local path resolution and asynchronous completion for the header path bar.

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::Duration,
};

const DEBOUNCE: Duration = Duration::from_millis(120);
const BATCH_SIZE: i32 = 128;
const MAX_SUGGESTIONS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectoryQuery {
    directory: PathBuf,
    prefix: String,
}

#[derive(Debug)]
struct DirectoryCandidate {
    name: String,
    is_directory: bool,
}

/// Resolve entry text without interpreting percent escapes in local paths.
/// Unchanged text preserves the exact URI, including non-UTF-8 paths shown
/// lossily by the breadcrumb bar. Relative paths are rooted at the selected tab.
pub(crate) fn resolve_path_input(
    text: &str,
    shown_text: &str,
    shown_uri: &str,
    current_uri: &str,
    home: &Path,
) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    if text == shown_text {
        return Some(shown_uri.to_string());
    }
    if text.contains("://") {
        return Some(text.to_string());
    }

    let path = local_path(text, current_uri, home)?;
    Some(gio::File::for_path(path).uri().to_string())
}

fn local_path(text: &str, current_uri: &str, home: &Path) -> Option<PathBuf> {
    if text == "~" {
        return Some(home.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        return Some(home.join(rest));
    }

    let path = PathBuf::from(text);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(gio::File::for_uri(current_uri).path()?.join(path))
    }
}

fn directory_query(text: &str, current_uri: &str, home: &Path) -> Option<DirectoryQuery> {
    if text.contains("://") || text.is_empty() {
        return None;
    }

    let (parent, prefix) = if text == "~" {
        ("~", "")
    } else if text.ends_with('/') {
        (text, "")
    } else if let Some((parent, prefix)) = text.rsplit_once('/') {
        (if parent.is_empty() { "/" } else { parent }, prefix)
    } else {
        ("", text)
    };

    let directory = if parent.is_empty() {
        gio::File::for_uri(current_uri).path()?
    } else if parent == "~" {
        home.to_path_buf()
    } else if let Some(rest) = parent.strip_prefix("~/") {
        home.join(rest)
    } else {
        let path = PathBuf::from(parent);
        if path.is_absolute() {
            path
        } else {
            gio::File::for_uri(current_uri).path()?.join(path)
        }
    };

    Some(DirectoryQuery {
        directory,
        prefix: prefix.to_string(),
    })
}

fn filter_directories(
    candidates: impl IntoIterator<Item = DirectoryCandidate>,
    prefix: &str,
    show_hidden: bool,
) -> Vec<String> {
    let include_hidden = show_hidden || prefix.starts_with('.');
    let mut names: Vec<String> = candidates
        .into_iter()
        .filter(|candidate| candidate.is_directory)
        .map(|candidate| candidate.name)
        .filter(|name| name.starts_with(prefix))
        .filter(|name| include_hidden || !name.starts_with('.'))
        .collect();
    sort_and_limit(&mut names);
    names
}

fn merge_suggestions(mut previous: Vec<String>, new: Vec<String>) -> Vec<String> {
    previous.extend(new);
    sort_and_limit(&mut previous);
    previous
}

fn sort_and_limit(names: &mut Vec<String>) {
    names.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right))
    });
    names.truncate(MAX_SUGGESTIONS);
}

fn common_prefix(names: &[String]) -> Option<String> {
    let mut iter = names.iter();
    let mut prefix: Vec<char> = iter.next()?.chars().collect();
    for name in iter {
        let chars: Vec<char> = name.chars().collect();
        let common = prefix
            .iter()
            .zip(chars.iter())
            .take_while(|(left, right)| left == right)
            .count();
        prefix.truncate(common);
    }
    Some(prefix.into_iter().collect())
}

fn replace_last_segment(text: &str, segment: &str) -> String {
    if text == "~" {
        return format!("~/{segment}");
    }
    if text.ends_with('/') {
        return format!("{text}{segment}");
    }
    if let Some(index) = text.rfind('/') {
        return format!("{}{segment}", &text[..=index]);
    }
    segment.to_string()
}

fn result_is_current(
    result_generation: u64,
    result_text: &str,
    current_generation: u64,
    current_text: &str,
) -> bool {
    result_generation == current_generation && result_text == current_text
}

struct CompletionState {
    entry: glib::WeakRef<gtk::Entry>,
    stack: glib::WeakRef<gtk::Stack>,
    popover: glib::WeakRef<gtk::Popover>,
    scrolled: glib::WeakRef<gtk::ScrolledWindow>,
    list: glib::WeakRef<gtk::ListBox>,
    generation: u64,
    current_text: String,
    suggestions: Vec<String>,
    selected: Option<usize>,
    debounce: Option<glib::SourceId>,
    cancellable: Option<gio::Cancellable>,
}

impl CompletionState {
    fn cancel_pending(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(source) = self.debounce.take() {
            source.remove();
        }
        if let Some(cancellable) = self.cancellable.take() {
            cancellable.cancel();
        }
    }

    fn invalidate(&mut self) {
        self.cancel_pending();
        self.suggestions.clear();
        self.selected = None;
        if let Some(list) = self.list.upgrade() {
            clear_list(&list);
        }
        if let Some(popover) = self.popover.upgrade() {
            popover.popdown();
        }
    }

    fn is_current(&self, generation: u64, text: &str) -> bool {
        result_is_current(generation, text, self.generation, &self.current_text)
    }
}

impl Drop for CompletionState {
    fn drop(&mut self) {
        if let Some(source) = self.debounce.take() {
            source.remove();
        }
        if let Some(cancellable) = self.cancellable.take() {
            cancellable.cancel();
        }
    }
}

/// Owns the popover state; widget references in async callbacks are weak, so
/// destroying the path entry cancels outstanding searches without a cycle.
#[derive(Clone)]
pub(crate) struct PathAutocomplete {
    state: Rc<RefCell<CompletionState>>,
}

impl PathAutocomplete {
    pub(crate) fn new(
        entry: &gtk::Entry,
        stack: &gtk::Stack,
        window: &gtk::Window,
        current_uri: Rc<dyn Fn() -> Option<String>>,
        show_hidden: Rc<dyn Fn() -> bool>,
    ) -> Self {
        let popover = gtk::Popover::builder()
            // Autohide makes GtkPopover modal and captures keyboard focus.
            // Outside clicks and Escape are handled explicitly below.
            .autohide(false)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .focusable(false)
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .max_content_height(280)
            .propagate_natural_height(true)
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(true)
            .focusable(false)
            .build();
        list.update_property(&[gtk::accessible::Property::Label(&crate::l10n::tr(
            "path-suggestions",
        ))]);
        scrolled.set_child(Some(&list));
        popover.set_child(Some(&scrolled));
        popover.set_parent(entry);

        let state = Rc::new(RefCell::new(CompletionState {
            entry: entry.downgrade(),
            stack: stack.downgrade(),
            popover: popover.downgrade(),
            scrolled: scrolled.downgrade(),
            list: list.downgrade(),
            generation: 0,
            current_text: String::new(),
            suggestions: Vec::new(),
            selected: None,
            debounce: None,
            cancellable: None,
        }));

        popover.connect_closed({
            let state = Rc::downgrade(&state);
            move |_| {
                if let Some(state) = state.upgrade() {
                    // The popover can close while a directory batch is still
                    // pending. Don't let that response reopen it after a click
                    // outside; try_borrow also avoids reentry for our own
                    // synchronous popdown calls.
                    if let Ok(mut state) = state.try_borrow_mut() {
                        state.cancel_pending();
                    }
                }
            }
        });

        let outside_click = gtk::GestureClick::builder().button(0).build();
        outside_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        outside_click.connect_pressed({
            let window = window.downgrade();
            let entry = entry.downgrade();
            let popover = popover.downgrade();
            move |_, _, x, y| {
                let (Some(window), Some(entry), Some(popover)) =
                    (window.upgrade(), entry.upgrade(), popover.upgrade())
                else {
                    return;
                };
                let inside_completion =
                    window
                        .pick(x, y, gtk::PickFlags::DEFAULT)
                        .is_some_and(|target| {
                            target == entry.clone().upcast::<gtk::Widget>()
                                || target.is_ancestor(&entry)
                                || target == popover.clone().upcast::<gtk::Widget>()
                                || target.is_ancestor(&popover)
                        });
                if !inside_completion {
                    popover.popdown();
                }
            }
        });
        window.add_controller(outside_click);

        window.connect_is_active_notify({
            let popover = popover.downgrade();
            move |window| {
                if !window.is_active() {
                    if let Some(popover) = popover.upgrade() {
                        popover.popdown();
                    }
                }
            }
        });

        entry.connect_changed({
            let state = state.clone();
            move |entry| {
                schedule_search(
                    &state,
                    entry,
                    current_uri(),
                    show_hidden(),
                    glib::home_dir(),
                );
            }
        });

        list.connect_row_activated({
            let state = Rc::downgrade(&state);
            move |_, row| {
                if let Some(state) = state.upgrade() {
                    accept_suggestion(&state, row.index() as usize, true);
                }
            }
        });

        entry.add_controller(build_key_controller(
            Rc::downgrade(&state),
            stack.downgrade(),
        ));

        Self { state }
    }

    pub(crate) fn invalidate_and_close(&self) {
        self.state.borrow_mut().invalidate();
    }

    pub(crate) fn weak_retranslator(&self) -> Rc<dyn Fn()> {
        let state = Rc::downgrade(&self.state);
        Rc::new(move || {
            if let Some(state) = state.upgrade() {
                if let Some(list) = state.borrow().list.upgrade() {
                    let label = crate::l10n::tr("path-suggestions");
                    list.update_property(&[gtk::accessible::Property::Label(&label)]);
                }
            }
        })
    }

    /// A weak callback is safe to retain in the tab navigation callback: it
    /// closes completion when a different tab or location becomes active.
    pub(crate) fn weak_invalidator(&self) -> Rc<dyn Fn()> {
        let state = Rc::downgrade(&self.state);
        Rc::new(move || {
            if let Some(state) = state.upgrade() {
                state.borrow_mut().invalidate();
            }
        })
    }
}

fn clear_list(list: &gtk::ListBox) {
    list.select_row(None::<&gtk::ListBoxRow>);
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn schedule_search(
    state: &Rc<RefCell<CompletionState>>,
    entry: &gtk::Entry,
    current_uri: Option<String>,
    show_hidden: bool,
    home: PathBuf,
) {
    let text = entry.text().to_string();
    let generation = {
        let mut state = state.borrow_mut();
        state.invalidate();
        state.current_text = text.clone();
        state.generation
    };

    if !editor_is_visible(&state.borrow()) {
        return;
    }
    let Some(current_uri) = current_uri else {
        return;
    };
    let Some(query) = directory_query(&text, &current_uri, &home) else {
        return;
    };

    let weak_state = Rc::downgrade(state);
    let text_for_timer = text.clone();
    let source = glib::timeout_add_local_once(DEBOUNCE, move || {
        let Some(state) = weak_state.upgrade() else {
            return;
        };
        {
            let mut borrowed = state.borrow_mut();
            borrowed.debounce = None;
            if !borrowed.is_current(generation, &text_for_timer)
                || borrowed
                    .entry
                    .upgrade()
                    .is_none_or(|entry| entry.text().as_str() != text_for_timer)
                || !editor_is_visible(&borrowed)
            {
                return;
            }
        }
        start_search(
            Rc::downgrade(&state),
            generation,
            text_for_timer,
            query,
            show_hidden,
        );
    });
    state.borrow_mut().debounce = Some(source);
}

fn start_search(
    state: Weak<RefCell<CompletionState>>,
    generation: u64,
    text: String,
    query: DirectoryQuery,
    show_hidden: bool,
) {
    let cancellable = gio::Cancellable::new();
    let Some(inner) = state.upgrade() else {
        return;
    };
    {
        let mut inner = inner.borrow_mut();
        if !inner.is_current(generation, &text) {
            return;
        }
        inner.cancellable = Some(cancellable.clone());
    }

    let cancellable_for_callback = cancellable.clone();
    gio::File::for_path(query.directory).enumerate_children_async(
        "standard::name,standard::type",
        gio::FileQueryInfoFlags::NONE,
        glib::Priority::DEFAULT,
        Some(&cancellable),
        move |result| match result {
            Ok(enumerator) => read_batch(
                state,
                generation,
                text,
                query.prefix,
                show_hidden,
                enumerator,
                cancellable_for_callback,
                Vec::new(),
            ),
            Err(_) => finish_search(&state, generation, &text, Vec::new()),
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn read_batch(
    state: Weak<RefCell<CompletionState>>,
    generation: u64,
    text: String,
    prefix: String,
    show_hidden: bool,
    enumerator: gio::FileEnumerator,
    cancellable: gio::Cancellable,
    mut candidates: Vec<String>,
) {
    if !state_is_current(&state, generation, &text) {
        return;
    }
    let enumerator_for_callback = enumerator.clone();
    let cancellable_for_callback = cancellable.clone();
    enumerator.next_files_async(
        BATCH_SIZE,
        glib::Priority::DEFAULT,
        Some(&cancellable),
        move |result| match result {
            Ok(infos) if infos.is_empty() => {
                finish_search(&state, generation, &text, candidates);
            }
            Ok(infos) => {
                let new_suggestions = filter_directories(
                    infos.into_iter().map(|info| DirectoryCandidate {
                        name: info.name().to_string_lossy().into_owned(),
                        is_directory: info.file_type() == gio::FileType::Directory,
                    }),
                    &prefix,
                    show_hidden,
                );
                candidates = merge_suggestions(candidates, new_suggestions);
                if !candidates.is_empty() {
                    update_suggestions(&state, generation, &text, candidates.clone(), false);
                }
                read_batch(
                    state,
                    generation,
                    text,
                    prefix,
                    show_hidden,
                    enumerator_for_callback,
                    cancellable_for_callback,
                    candidates,
                );
            }
            Err(_) => finish_search(&state, generation, &text, Vec::new()),
        },
    );
}

fn state_is_current(state: &Weak<RefCell<CompletionState>>, generation: u64, text: &str) -> bool {
    state.upgrade().is_some_and(|state| {
        let state = state.borrow();
        state.is_current(generation, text) && editor_is_visible(&state)
    })
}

fn editor_is_visible(state: &CompletionState) -> bool {
    state
        .stack
        .upgrade()
        .and_then(|stack| stack.visible_child_name())
        .is_some_and(|name| name.as_str() == "edit")
}

fn finish_search(
    state: &Weak<RefCell<CompletionState>>,
    generation: u64,
    text: &str,
    suggestions: Vec<String>,
) {
    update_suggestions(state, generation, text, suggestions, true);
}

fn update_suggestions(
    state: &Weak<RefCell<CompletionState>>,
    generation: u64,
    text: &str,
    suggestions: Vec<String>,
    finished: bool,
) {
    let Some(inner) = state.upgrade() else {
        return;
    };
    let (entry, list, popover, scrolled, popup_width, entry_height, position, selection, changed) = {
        let mut inner = inner.borrow_mut();
        if !inner.is_current(generation, text) {
            return;
        }
        let Some(entry) = inner.entry.upgrade() else {
            return;
        };
        if entry.text().as_str() != text || !editor_is_visible(&inner) {
            return;
        }
        if finished {
            inner.cancellable = None;
        }
        let changed = inner.suggestions != suggestions;
        if changed {
            inner.suggestions = suggestions;
            inner.selected = None;
        }
        let popup_width = entry.width().max(1);
        (
            entry.clone(),
            inner.list.upgrade(),
            inner.popover.upgrade(),
            inner.scrolled.upgrade(),
            popup_width,
            entry.height(),
            entry.position(),
            entry.selection_bounds(),
            changed,
        )
    };

    if changed {
        if let Some(list) = list {
            clear_list(&list);
            let names = inner.borrow().suggestions.clone();
            for name in names {
                let row = suggestion_row(&suggestion_display_path(text, &name));
                list.append(&row);
            }
        }
    }
    if let Some(popover) = popover {
        if inner.borrow().suggestions.is_empty() {
            if finished {
                popover.popdown();
            }
        } else {
            let was_visible = popover.is_visible();
            if !was_visible {
                if let Some(scrolled) = scrolled {
                    scrolled.set_min_content_width(popup_width);
                    scrolled.set_max_content_width(popup_width);
                    scrolled.set_width_request(popup_width);
                }
                // Popovers are centered on their pointing rectangle. Match it
                // to the entry width so both edges line up exactly.
                let anchor = gdk::Rectangle::new(0, 0, popup_width, entry_height);
                popover.set_pointing_to(Some(&anchor));
                popover.popup();
            }
            if changed || !was_visible {
                restore_entry_focus_after_popup(&entry, &popover, position, selection);
            }
        }
    }
}

fn restore_entry_focus_after_popup(
    entry: &gtk::Entry,
    popover: &gtk::Popover,
    position: i32,
    selection: Option<(i32, i32)>,
) {
    let entry = entry.downgrade();
    let popover = popover.downgrade();
    glib::idle_add_local_once(move || {
        if popover
            .upgrade()
            .is_some_and(|popover| popover.is_visible())
        {
            if let Some(entry) = entry.upgrade() {
                entry.grab_focus();
                if let Some((start, end)) = selection {
                    entry.select_region(start, end);
                } else {
                    entry.set_position(position);
                }
            }
        }
    });
}

fn suggestion_display_path(text: &str, name: &str) -> String {
    format!(
        "{}/",
        replace_last_segment(text, name).trim_end_matches('/')
    )
}

fn suggestion_row(name: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::builder().focusable(false).build();
    row.update_property(&[gtk::accessible::Property::Label(name)]);
    let label = gtk::Label::builder()
        .label(name)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(64);
    row.set_child(Some(&label));
    row
}

fn build_key_controller(
    state: Weak<RefCell<CompletionState>>,
    stack: glib::WeakRef<gtk::Stack>,
) -> gtk::EventControllerKey {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    controller.connect_key_pressed(move |_, key, _, _| {
        let Some(inner) = state.upgrade() else {
            return glib::Propagation::Proceed;
        };
        match key {
            gdk::Key::Down | gdk::Key::Up => {
                let visible = inner
                    .borrow()
                    .popover
                    .upgrade()
                    .is_some_and(|popover| popover.is_visible());
                if visible {
                    let delta = if key == gdk::Key::Down { 1 } else { -1 };
                    move_selection(&inner, delta);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
            gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                let (selected, visible) = {
                    let state = inner.borrow();
                    (
                        state.selected,
                        state
                            .popover
                            .upgrade()
                            .is_some_and(|popover| popover.is_visible()),
                    )
                };
                if visible {
                    if let Some(index) = selected {
                        accept_suggestion(&inner, index, false);
                    } else {
                        complete_common_prefix(&inner);
                    }
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                let (visible, selected) = {
                    let state = inner.borrow();
                    (
                        state
                            .popover
                            .upgrade()
                            .is_some_and(|popover| popover.is_visible()),
                        state.selected,
                    )
                };
                if let Some(index) = selected.filter(|_| visible) {
                    accept_suggestion(&inner, index, true);
                    glib::Propagation::Stop
                } else {
                    inner.borrow_mut().invalidate();
                    glib::Propagation::Proceed
                }
            }
            gdk::Key::Escape => {
                let visible = inner
                    .borrow()
                    .popover
                    .upgrade()
                    .is_some_and(|popover| popover.is_visible());
                if visible {
                    inner.borrow_mut().invalidate();
                } else {
                    inner.borrow_mut().invalidate();
                    if let Some(stack) = stack.upgrade() {
                        stack.set_visible_child_name("crumbs");
                    }
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    controller
}

fn move_selection(state: &Rc<RefCell<CompletionState>>, delta: isize) {
    let row = {
        let mut state = state.borrow_mut();
        if state.suggestions.is_empty() {
            return;
        }
        let len = state.suggestions.len();
        let selected = match state.selected {
            Some(current) => (current as isize + delta).rem_euclid(len as isize) as usize,
            None if delta > 0 => 0,
            None => len - 1,
        };
        state.selected = Some(selected);
        state
            .list
            .upgrade()
            .and_then(|list| list.row_at_index(selected as i32))
    };
    if let Some(row) = row {
        if let Some(list) = state.borrow().list.upgrade() {
            list.select_row(Some(&row));
        }
    }
}

fn accept_suggestion(state: &Rc<RefCell<CompletionState>>, index: usize, keep_focus: bool) {
    let (entry, text, name) = {
        let state = state.borrow();
        let Some(entry) = state.entry.upgrade() else {
            return;
        };
        let Some(name) = state.suggestions.get(index) else {
            return;
        };
        (entry, state.current_text.clone(), name.clone())
    };
    let completed = format!("{}/", replace_last_segment(&text, &name));
    if let Some(popover) = state.borrow().popover.upgrade() {
        popover.popdown();
    }
    entry.set_text(&completed);
    entry.set_position(-1);
    if keep_focus {
        entry.grab_focus();
    }
}

fn complete_common_prefix(state: &Rc<RefCell<CompletionState>>) {
    let (entry, text, names) = {
        let state = state.borrow();
        let Some(entry) = state.entry.upgrade() else {
            return;
        };
        (entry, state.current_text.clone(), state.suggestions.clone())
    };
    let Some(prefix) = common_prefix(&names) else {
        return;
    };
    if replace_last_segment(&text, &prefix) != text {
        entry.set_text(&replace_last_segment(&text, &prefix));
        entry.set_position(-1);
        entry.grab_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};
    use tempfile::tempdir;

    fn directory_uri(path: &Path) -> String {
        gio::File::for_path(path).uri().to_string()
    }

    #[test]
    fn local_paths_resolve_against_active_tab_and_keep_special_characters() {
        let temp = tempdir().unwrap();
        let base = temp.path().join("base with spaces");
        fs::create_dir(&base).unwrap();
        let current = directory_uri(&base);
        let home = temp.path().join("home");

        let relative =
            resolve_path_input("percent % and ü", "/old", "file:///old", &current, &home).unwrap();
        assert_eq!(
            gio::File::for_uri(&relative).path().unwrap(),
            base.join("percent % and ü")
        );

        let home_path = resolve_path_input("~/Downloads", "", "", &current, &home).unwrap();
        assert_eq!(
            gio::File::for_uri(&home_path).path().unwrap(),
            home.join("Downloads")
        );

        let absolute = base.join("100% literal ü");
        let absolute_uri = resolve_path_input(
            absolute.to_str().unwrap(),
            "/old",
            "file:///old",
            &current,
            &home,
        )
        .unwrap();
        assert_eq!(gio::File::for_uri(&absolute_uri).path().unwrap(), absolute);
    }

    #[test]
    fn unchanged_text_and_manual_uris_keep_their_meaning() {
        let current = "file:///tmp";
        assert_eq!(
            resolve_path_input(
                "/lossy name",
                "/lossy name",
                "file:///encoded%FF",
                current,
                Path::new("/home/test")
            ),
            Some("file:///encoded%FF".to_string())
        );
        assert_eq!(
            resolve_path_input("sftp://host/path", "", "", current, Path::new("/home/test")),
            Some("sftp://host/path".to_string())
        );
    }

    #[test]
    fn completion_query_splits_parent_and_prefix_without_decoding() {
        let base = Path::new("/home/test/base");
        let current = directory_uri(base);
        let home = Path::new("/home/test");
        assert_eq!(
            directory_query("~/Documents/My%20", &current, home),
            Some(DirectoryQuery {
                directory: home.join("Documents"),
                prefix: "My%20".into(),
            })
        );
        assert_eq!(
            directory_query("relative/sub", &current, home),
            Some(DirectoryQuery {
                directory: base.join("relative"),
                prefix: "sub".into(),
            })
        );
        assert_eq!(
            directory_query("/tmp/space dir/100%", &current, home),
            Some(DirectoryQuery {
                directory: PathBuf::from("/tmp/space dir"),
                prefix: "100%".into(),
            })
        );
        assert_eq!(directory_query("sftp://host/path", &current, home), None);
    }

    #[test]
    fn completion_filters_files_and_hidden_directories_and_sorts_stably() {
        let candidates = vec![
            DirectoryCandidate {
                name: "alpha".into(),
                is_directory: true,
            },
            DirectoryCandidate {
                name: "Alpine".into(),
                is_directory: true,
            },
            DirectoryCandidate {
                name: ".archive".into(),
                is_directory: true,
            },
            DirectoryCandidate {
                name: "alpine.txt".into(),
                is_directory: false,
            },
        ];
        assert_eq!(filter_directories(candidates, "Al", false), vec!["Alpine"]);
        let hidden = vec![
            DirectoryCandidate {
                name: ".archive".into(),
                is_directory: true,
            },
            DirectoryCandidate {
                name: ".assets".into(),
                is_directory: true,
            },
        ];
        assert_eq!(
            filter_directories(hidden, ".a", false),
            vec![".archive", ".assets"]
        );
        let sorted = vec![
            DirectoryCandidate {
                name: "Beta".into(),
                is_directory: true,
            },
            DirectoryCandidate {
                name: "alpha".into(),
                is_directory: true,
            },
            DirectoryCandidate {
                name: "Alpine".into(),
                is_directory: true,
            },
        ];
        assert_eq!(
            filter_directories(sorted, "", true),
            vec!["alpha", "Alpine", "Beta"]
        );
        assert_eq!(
            merge_suggestions(vec!["Beta".into()], vec!["Gamma".into(), "Alpha".into()]),
            vec!["Alpha", "Beta", "Gamma"]
        );
        let many = (0..55)
            .map(|index| DirectoryCandidate {
                name: format!("dir-{index:02}"),
                is_directory: true,
            })
            .collect::<Vec<_>>();
        assert_eq!(filter_directories(many, "", true).len(), MAX_SUGGESTIONS);
    }

    #[test]
    fn common_prefix_and_replacement_support_unicode_and_tilde() {
        let names = vec!["Caffè A".to_string(), "Caffè B".to_string()];
        assert_eq!(common_prefix(&names).as_deref(), Some("Caffè "));
        assert_eq!(replace_last_segment("~/Doc", "Documents"), "~/Documents");
        assert_eq!(replace_last_segment("folder/", "sub"), "folder/sub");
        assert_eq!(
            suggestion_display_path("/home/test/Sca", "Scaricati"),
            "/home/test/Scaricati/"
        );
        assert_eq!(suggestion_display_path("~/", "Downloads"), "~/Downloads/");
    }

    #[test]
    fn obsolete_results_are_discarded_by_generation_and_text() {
        assert!(!result_is_current(4, "/tmp/old", 5, "/tmp/new"));
        assert!(!result_is_current(5, "/tmp/old", 5, "/tmp/new"));
        assert!(result_is_current(5, "/tmp/new", 5, "/tmp/new"));
    }

    #[test]
    fn enumeration_suggests_symlinked_directories() {
        let temp = tempdir().unwrap();
        let target = temp.path().join("actual directory");
        fs::create_dir(&target).unwrap();
        let link = temp.path().join("linked directory");
        symlink(&target, &link).unwrap();

        let results = Rc::new(RefCell::new(None));
        let results_for_callback = results.clone();
        let context = glib::MainContext::new();
        let main_loop = glib::MainLoop::new(Some(&context), false);
        let loop_for_callback = main_loop.clone();
        context
            .with_thread_default(|| {
                gio::File::for_path(temp.path()).enumerate_children_async(
                    "standard::name,standard::type",
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                    None::<&gio::Cancellable>,
                    move |result| match result {
                        Ok(enumerator) => enumerator.next_files_async(
                            64,
                            glib::Priority::DEFAULT,
                            None::<&gio::Cancellable>,
                            move |result| {
                                let candidates =
                                    result.unwrap().into_iter().map(|info| DirectoryCandidate {
                                        name: info.name().to_string_lossy().into_owned(),
                                        is_directory: info.file_type() == gio::FileType::Directory,
                                    });
                                *results_for_callback.borrow_mut() =
                                    Some(filter_directories(candidates, "linked", false));
                                loop_for_callback.quit();
                            },
                        ),
                        Err(error) => panic!("directory enumeration failed: {error}"),
                    },
                );
                main_loop.run();
            })
            .unwrap();
        assert_eq!(
            results.borrow().as_ref().unwrap(),
            &["linked directory".to_string()]
        );
    }
}
