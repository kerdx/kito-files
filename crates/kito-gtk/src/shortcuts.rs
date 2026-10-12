//! Shared accelerator definitions and native menu labels.
//!
//! Application accelerators are registered from this list and read back from
//! `GtkApplication` when a menu is built. View-local shortcuts are registered
//! and displayed from the same table so text entries keep their own bindings.

use gtk::prelude::*;
use std::cell::RefCell;
thread_local! { static CURRENT: RefCell<crate::preferences::model::Preferences> = RefCell::new(crate::preferences::model::Preferences::default()); }

#[derive(Clone, Copy, Debug)]
pub(crate) struct Binding {
    pub id: &'static str,
    pub action: &'static str,
    pub default: &'static str,
    pub local: bool,
}

pub(crate) const CATALOG: &[Binding] = &[
    Binding {
        id: "copy",
        action: "win.copy",
        default: "<Control>c",
        local: false,
    },
    Binding {
        id: "cut",
        action: "win.cut",
        default: "<Control>x",
        local: false,
    },
    Binding {
        id: "paste",
        action: "win.paste",
        default: "<Control>v",
        local: false,
    },
    Binding {
        id: "trash",
        action: "win.trash",
        default: "Delete",
        local: false,
    },
    Binding {
        id: "delete",
        action: "win.delete",
        default: "<Shift>Delete",
        local: false,
    },
    Binding {
        id: "rename",
        action: "win.rename",
        default: "F2",
        local: false,
    },
    Binding {
        id: "edit-path",
        action: "win.edit-path",
        default: "<Control>l",
        local: false,
    },
    Binding {
        id: "reload-f5",
        action: "win.reload",
        default: "F5",
        local: false,
    },
    Binding {
        id: "reload-ctrl-r",
        action: "win.reload",
        default: "<Control>r",
        local: false,
    },
    Binding {
        id: "new-folder",
        action: "win.new-folder",
        default: "<Control><Shift>n",
        local: false,
    },
    Binding {
        id: "preferences",
        action: "win.preferences",
        default: "<Control>comma",
        local: false,
    },
    Binding {
        id: "undo",
        action: "win.undo",
        default: "<Control>z",
        local: true,
    },
    Binding {
        id: "select-all",
        action: "win.select-all",
        default: "<Control>a",
        local: true,
    },
    Binding {
        id: "bookmark-current",
        action: "win.bookmark-current",
        default: "<Control>d",
        local: true,
    },
    Binding {
        id: "new-tab",
        action: "win.new-tab",
        default: "<Control>t",
        local: true,
    },
    Binding {
        id: "close-tab",
        action: "win.close-tab",
        default: "<Control>w",
        local: true,
    },
    Binding {
        id: "reopen-tab",
        action: "win.reopen-tab",
        default: "<Control><Shift>t",
        local: true,
    },
    Binding {
        id: "next-tab",
        action: "win.next-tab",
        default: "<Control>Tab",
        local: true,
    },
    Binding {
        id: "previous-tab",
        action: "win.previous-tab",
        default: "<Control><Shift>Tab",
        local: true,
    },
    Binding {
        id: "back",
        action: "win.back",
        default: "<Alt>Left",
        local: true,
    },
    Binding {
        id: "forward",
        action: "win.forward",
        default: "<Alt>Right",
        local: true,
    },
    Binding {
        id: "up",
        action: "win.up",
        default: "<Alt>Up",
        local: true,
    },
    Binding {
        id: "zoom-in-plus",
        action: "win.zoom-in",
        default: "<Control>plus",
        local: true,
    },
    Binding {
        id: "zoom-in-keypad",
        action: "win.zoom-in",
        default: "<Control>KP_Add",
        local: true,
    },
    Binding {
        id: "zoom-out-minus",
        action: "win.zoom-out",
        default: "<Control>minus",
        local: true,
    },
    Binding {
        id: "zoom-out-keypad",
        action: "win.zoom-out",
        default: "<Control>KP_Subtract",
        local: true,
    },
    Binding {
        id: "zoom-reset",
        action: "win.zoom-reset",
        default: "<Control>0",
        local: true,
    },
];

pub(crate) fn effective(
    binding: &Binding,
    preferences: &crate::preferences::model::Preferences,
) -> Option<String> {
    resolve_all(preferences).get(binding.id).cloned().flatten()
}

fn resolve_all(
    preferences: &crate::preferences::model::Preferences,
) -> std::collections::HashMap<&'static str, Option<String>> {
    let mut result = std::collections::HashMap::new();
    let mut used = Vec::new();
    // Protect untouched defaults first; process overrides in stable catalog order.
    for binding in CATALOG
        .iter()
        .filter(|b| !preferences.keyboard_shortcuts.contains_key(b.id))
    {
        let value = normalized(binding.default);
        if let Some(value) = value.filter(|v| !used.contains(v)) {
            used.push(value.clone());
            result.insert(binding.id, Some(value));
        } else {
            result.insert(binding.id, None);
        }
    }
    for binding in CATALOG
        .iter()
        .filter(|b| preferences.keyboard_shortcuts.contains_key(b.id))
    {
        let override_value = preferences.keyboard_shortcuts.get(binding.id);
        let requested = override_value.and_then(|v| v.as_deref());
        let candidate = requested
            .filter(|v| valid_accelerator(v))
            .and_then(normalized);
        let chosen = candidate.filter(|v| !used.contains(v)).or_else(|| {
            // Invalid/colliding custom values safely fall back only when available.
            if override_value.is_some_and(Option::is_some)
                && requested.is_some_and(|v| !valid_accelerator(v))
            {
                normalized(binding.default).filter(|v| !used.contains(v))
            } else {
                None
            }
        });
        if let Some(value) = chosen {
            used.push(value.clone());
            result.insert(binding.id, Some(value));
        } else {
            result.insert(binding.id, None);
        }
    }
    result
}

fn normalized(value: &str) -> Option<String> {
    let (key, mods) = gtk::accelerator_parse(value)?;
    Some(gtk::accelerator_name(key, mods).to_string())
}

pub(crate) fn valid_accelerator(value: &str) -> bool {
    let Some((key, modifiers)) = gtk::accelerator_parse(value) else {
        return false;
    };
    if !gtk::accelerator_valid(key, modifiers) {
        return false;
    }
    if gtk::ShortcutTrigger::parse_string(value).is_none() {
        return false;
    }
    // Unmodified printable keys are unsafe in views and text controls.
    let protected = gtk::gdk::ModifierType::CONTROL_MASK
        | gtk::gdk::ModifierType::ALT_MASK
        | gtk::gdk::ModifierType::SUPER_MASK;
    if !modifiers.intersects(protected) && key.to_unicode().is_some_and(|c| !c.is_control()) {
        return false;
    }
    true
}

pub(crate) fn accelerator_values(
    preferences: &crate::preferences::model::Preferences,
    action: &str,
) -> Vec<String> {
    CATALOG
        .iter()
        .filter(|item| !item.local && item.action == action)
        .filter_map(|item| effective(item, preferences))
        .collect()
}

pub(crate) const APPLICATION_ACCELERATORS: &[(&str, &[&str])] = &[
    ("win.copy", &["<Control>c"]),
    ("win.cut", &["<Control>x"]),
    ("win.paste", &["<Control>v"]),
    ("win.trash", &["Delete"]),
    ("win.delete", &["<Shift>Delete"]),
    ("win.rename", &["F2"]),
    ("win.edit-path", &["<Control>l"]),
    ("win.reload", &["F5", "<Control>r"]),
    ("win.new-folder", &["<Control><Shift>n"]),
    ("win.preferences", &["<Control>comma"]),
];

/// These shortcuts are local to a file view by design. In particular, Ctrl+A
/// must continue to select text when an entry or dialog has focus.
pub(crate) fn register_application_accelerators(
    app: &impl IsA<gtk::Application>,
    preferences: &crate::preferences::model::Preferences,
) {
    CURRENT.with(|current| *current.borrow_mut() = preferences.clone());
    let app: &gtk::Application = app.as_ref();
    for (action, _) in APPLICATION_ACCELERATORS {
        let accelerators = accelerator_values(preferences, action);
        let refs = accelerators.iter().map(String::as_str).collect::<Vec<_>>();
        app.set_accels_for_action(action, &refs);
    }
}

/// Resolves the action's current application associations plus any local view
/// bindings from their registration table. Keep application order: the first
/// accelerator is the primary label and later ones are tooltip alternatives.
pub(crate) fn accelerators_for_action(
    app: &impl IsA<gtk::Application>,
    detailed_action_name: &str,
) -> Vec<String> {
    let app: &gtk::Application = app.as_ref();
    let registered = app
        .accels_for_action(detailed_action_name)
        .into_iter()
        .map(|accelerator| accelerator.to_string());
    let mut result = registered.collect::<Vec<_>>();
    CURRENT.with(|current| {
        let prefs = current.borrow();
        for binding in CATALOG
            .iter()
            .filter(|binding| binding.local && binding.action == detailed_action_name)
        {
            if let Some(accelerator) = effective(binding, &prefs) {
                if !result.contains(&accelerator) {
                    result.push(accelerator);
                }
            }
        }
    });
    result
}

pub(crate) fn shortcut_column_width(app: &impl IsA<gtk::Application>, actions: &[String]) -> i32 {
    let mut width = 0;
    for accelerator in actions
        .iter()
        .filter_map(|action| accelerators_for_action(app, action).into_iter().next())
    {
        if let Some(text) = formatted_accelerator(&accelerator) {
            let label = gtk::Label::new(Some(&text));
            let (_, natural, _, _) = label.measure(gtk::Orientation::Horizontal, -1);
            width = width.max(natural);
        }
    }
    width
}

/// Creates the shortcut-column cell. Empty cells reserve the same measured
/// width so menu action names line up without padding text with spaces.
pub(crate) fn shortcut_cell(
    app: &impl IsA<gtk::Application>,
    detailed_action_name: Option<&str>,
    column_width: i32,
) -> gtk::Widget {
    let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    cell.set_size_request(column_width, -1);

    if let Some((accelerator, alternatives)) = detailed_action_name
        .map(|action| accelerators_for_action(app, action))
        .and_then(|mut accelerators| {
            if accelerators.is_empty() {
                None
            } else {
                let primary = accelerators.remove(0);
                Some((primary, accelerators))
            }
        })
    {
        if let Some(text) = formatted_accelerator(&accelerator) {
            let label = gtk::Label::builder()
                .label(&text)
                .halign(gtk::Align::End)
                .hexpand(true)
                .build();
            label.set_valign(gtk::Align::Center);
            label.set_can_focus(false);
            label.set_can_target(false);
            label.add_css_class("dim-label");

            let alternatives = alternatives
                .into_iter()
                .filter_map(|accelerator| formatted_accelerator(&accelerator))
                .collect::<Vec<_>>();
            if !alternatives.is_empty() {
                label.set_tooltip_text(Some(&alternatives.join(" · ")));
            }
            cell.append(&label);
        }
    }

    cell.upcast()
}

fn formatted_accelerator(accelerator: &str) -> Option<String> {
    let (key, modifiers) = gtk::accelerator_parse(accelerator)?;
    Some(gtk::accelerator_get_label(key, modifiers).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_d_is_file_view_local_and_does_not_become_an_application_accelerator() {
        assert!(!APPLICATION_ACCELERATORS
            .iter()
            .any(|(action, _)| *action == "win.bookmark-current"));
        assert!(CATALOG.iter().any(|item| item.local
            && item.default == "<Control>d"
            && item.action == "win.bookmark-current"));
    }

    #[test]
    fn catalog_covers_each_default_and_has_unique_stable_ids() {
        let ids = CATALOG
            .iter()
            .map(|item| item.id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), CATALOG.len());
        for (action, accelerators) in APPLICATION_ACCELERATORS {
            for accelerator in *accelerators {
                assert!(
                    CATALOG.iter().any(|item| !item.local
                        && item.action == *action
                        && item.default == *accelerator),
                    "missing {action} {accelerator}"
                );
            }
        }
        assert_eq!(CATALOG.len(), 27);
        assert!(CATALOG.iter().all(|item| !item.id.is_empty()
            && !item.action.is_empty()
            && !item.default.is_empty()));
        assert_eq!(
            CATALOG.iter().filter(|item| !item.local).count(),
            APPLICATION_ACCELERATORS
                .iter()
                .map(|(_, values)| values.len())
                .sum()
        );
    }

    #[test]
    #[ignore = "accelerator parsing requires GTK initialization; run with a display"]
    fn invalid_persisted_accelerators_fall_back_to_defaults() {
        gtk::test_synced(|| {
            let mut prefs = crate::preferences::model::Preferences::default();
            prefs
                .keyboard_shortcuts
                .insert("copy".into(), Some("definitely not a shortcut".into()));
            assert_eq!(
                effective(&CATALOG[0], &prefs).as_deref(),
                Some("<Control>c")
            );
            assert!(!valid_accelerator("a"));
        });
    }

    #[test]
    #[ignore = "accelerator parsing requires GTK initialization; run with a display"]
    fn shortcut_overrides_preserve_disable_and_resolve_conflicts_uniquely() {
        gtk::test_synced(|| {
            let mut prefs = crate::preferences::model::Preferences::default();
            prefs.keyboard_shortcuts.insert("trash".into(), None);
            assert_eq!(
                effective(CATALOG.iter().find(|b| b.id == "trash").unwrap(), &prefs),
                None
            );

            prefs
                .keyboard_shortcuts
                .insert("copy".into(), Some("<Control>x".into()));
            prefs
                .keyboard_shortcuts
                .insert("cut".into(), Some("<Control>v".into()));
            let resolved = resolve_all(&prefs);
            let values = resolved
                .values()
                .filter_map(Option::as_ref)
                .collect::<Vec<_>>();
            assert_eq!(
                values.len(),
                values
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
            );
        });
    }

    #[test]
    #[ignore = "accelerator parsing requires GTK initialization; run with a display"]
    fn accelerator_validation_rejects_unmodified_shift_printable_only() {
        gtk::test_synced(|| {
            assert!(!valid_accelerator("<Shift>a"));
            assert!(valid_accelerator("<Control>a"));
            assert!(valid_accelerator("F2"));
            assert!(valid_accelerator("Delete"));
        });
    }
}
