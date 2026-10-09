//! Theme icons for interface controls.
//!
//! Keep content icons from GIO untouched in `file_list`; these helpers are
//! only for controls, places, menus and devices. The ordered `GThemedIcon`
//! names are resolved by the current GTK icon theme when each `GtkImage` is
//! rendered, so changing themes does not leave a cached fallback behind.

use gtk::{gio, prelude::*};

/// Builds an ordered control icon: put the preferred symbolic name and its
/// symbolic fallbacks first, followed by suitable regular icons.
pub(crate) fn control_icon(names: &[&str]) -> gio::ThemedIcon {
    gio::ThemedIcon::from_names(names)
}

/// Adds symbolic variants to an icon supplied by GIO while preserving its
/// device-specific names and regular fallbacks. Non-themed GIO icons remain
/// unchanged.
pub(crate) fn device_icon(icon: &gio::Icon) -> gio::Icon {
    let Some(themed) = icon.clone().downcast::<gio::ThemedIcon>().ok() else {
        return icon.clone();
    };
    let source_names = themed.names();
    if source_names.is_empty() {
        return icon.clone();
    }

    let mut candidates = Vec::new();
    for name in &source_names {
        add_fallback_chain(&mut candidates, name, true);
    }
    for name in &source_names {
        add_fallback_chain(&mut candidates, name, false);
    }

    let refs = candidates.iter().map(String::as_str).collect::<Vec<_>>();
    control_icon(&refs).upcast()
}

fn add_fallback_chain(names: &mut Vec<String>, name: &str, symbolic: bool) {
    let mut base = name.strip_suffix("-symbolic").unwrap_or(name).to_string();
    loop {
        let candidate = if symbolic {
            format!("{base}-symbolic")
        } else {
            base.clone()
        };
        if !names.contains(&candidate) {
            names.push(candidate);
        }
        let Some((shorter, _)) = base.rsplit_once('-') else {
            break;
        };
        base = shorter.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_icon_keeps_specific_symbolic_names_ahead_of_regular_fallbacks() {
        let source = gio::ThemedIcon::from_names(&["drive-removable-media-usb", "drive-optical"]);
        let source_icon: gio::Icon = source.clone().upcast();
        let icon = device_icon(&source_icon);
        let names = icon
            .downcast::<gio::ThemedIcon>()
            .expect("themed GIO icon remains themed")
            .names()
            .into_iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            [
                "drive-removable-media-usb-symbolic",
                "drive-removable-media-symbolic",
                "drive-removable-symbolic",
                "drive-symbolic",
                "drive-optical-symbolic",
                "drive-removable-media-usb",
                "drive-removable-media",
                "drive-removable",
                "drive",
                "drive-optical",
            ]
            .map(str::to_string)
        );
    }

    #[test]
    fn specific_control_icons_keep_symbolic_and_normal_fallback_order() {
        let icon = control_icon(&[
            "folder-documents-symbolic",
            "folder-symbolic",
            "folder-documents",
            "folder",
        ]);
        assert_eq!(
            icon.names()
                .into_iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
            [
                "folder-documents-symbolic",
                "folder-symbolic",
                "folder-documents",
                "folder",
            ]
            .map(str::to_string)
        );
    }
}
