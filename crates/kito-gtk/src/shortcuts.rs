//! Shared accelerator definitions and native menu labels.
//!
//! Application accelerators are registered from this list and read back from
//! `GtkApplication` when a menu is built. View-local shortcuts are registered
//! and displayed from the same table so text entries keep their own bindings.

use gtk::prelude::*;

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
pub(crate) const FILE_VIEW_SHORTCUTS: &[(&str, &str)] = &[
    ("<Control>a", "win.select-all"),
    ("<Control>t", "win.new-tab"),
    ("<Control>w", "win.close-tab"),
    ("<Control><Shift>t", "win.reopen-tab"),
    ("<Control>Tab", "win.next-tab"),
    ("<Control><Shift>Tab", "win.previous-tab"),
    ("<Alt>Left", "win.back"),
    ("<Alt>Right", "win.forward"),
    ("<Alt>Up", "win.up"),
    ("<Control>plus", "win.zoom-in"),
    ("<Control>KP_Add", "win.zoom-in"),
    ("<Control>minus", "win.zoom-out"),
    ("<Control>KP_Subtract", "win.zoom-out"),
    ("<Control>0", "win.zoom-reset"),
];

pub(crate) fn register_application_accelerators(app: &impl IsA<gtk::Application>) {
    let app: &gtk::Application = app.as_ref();
    for (action, accelerators) in APPLICATION_ACCELERATORS {
        app.set_accels_for_action(action, accelerators);
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
    resolve_accelerators(detailed_action_name, registered)
}

fn resolve_accelerators(
    detailed_action_name: &str,
    registered: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut accelerators = Vec::new();
    for accelerator in registered.into_iter().chain(
        FILE_VIEW_SHORTCUTS
            .iter()
            .filter(|(_, action)| *action == detailed_action_name)
            .map(|(accelerator, _)| (*accelerator).to_string()),
    ) {
        if !accelerators.contains(&accelerator) {
            accelerators.push(accelerator);
        }
    }
    accelerators
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
    fn view_menu_shortcut_comes_from_the_file_view_controller_definition() {
        assert_eq!(
            resolve_accelerators("win.select-all", std::iter::empty()),
            vec!["<Control>a".to_string()]
        );
        assert!(resolve_accelerators("win.copy", std::iter::empty()).is_empty());
    }

    #[test]
    fn registered_order_keeps_the_primary_before_alternatives() {
        assert_eq!(
            resolve_accelerators("win.reload", ["F5".to_string(), "<Control>r".to_string()]),
            vec!["F5".to_string(), "<Control>r".to_string()]
        );
    }
}
