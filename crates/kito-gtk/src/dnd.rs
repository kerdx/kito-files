//! GTK file drag-and-drop helpers. File lists use GDK's interoperable
//! `GdkFileList` value, which is exported to other applications as URIs.

use gtk::gdk::prelude::*;
use gtk::{gdk, gio, glib, prelude::*};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferAction {
    Copy,
    Move,
}

pub type DropHandler = Rc<dyn Fn(Vec<String>, String, TransferAction, bool, gdk::Drop)>;
pub type Destination = Rc<dyn Fn() -> Option<String>>;
pub type ExternalMoveHandler = Rc<dyn Fn(Vec<String>)>;
pub type PrepareHandler = Rc<dyn Fn() -> Option<PreparedDrag>>;

#[derive(Clone)]
pub struct PreparedDrag {
    pub uris: Vec<String>,
    pub provider: gdk::ContentProvider,
}

thread_local! {
    static ACTIVE_DRAGS: RefCell<HashSet<usize>> = RefCell::default();
    static INTERNAL_DROPS: RefCell<HashSet<usize>> = RefCell::default();
}

fn drag_key(drag: &gdk::Drag) -> usize {
    drag.as_ptr() as usize
}

fn register_drag(drag: &gdk::Drag) {
    ACTIVE_DRAGS.with(|active| {
        active.borrow_mut().insert(drag_key(drag));
    });
}

fn finish_drag(drag: &gdk::Drag) -> bool {
    let key = drag_key(drag);
    ACTIVE_DRAGS.with(|active| {
        active.borrow_mut().remove(&key);
    });
    INTERNAL_DROPS.with(|drops| drops.borrow_mut().remove(&key))
}

fn mark_internal_drop(drop: &gdk::Drop) -> bool {
    let Some(drag) = drop.drag() else {
        return false;
    };
    let key = drag_key(&drag);
    let internal = ACTIVE_DRAGS.with(|active| active.borrow().contains(&key));
    if internal {
        INTERNAL_DROPS.with(|drops| {
            drops.borrow_mut().insert(key);
        });
    }
    internal
}

/// AdwTabBar reports the matching page but not its GdkDrop. This records the
/// active in-process drag so the source does not repeat an already completed move.
pub fn mark_active_internal_drop() -> bool {
    let key = ACTIVE_DRAGS.with(|active| active.borrow().iter().next().copied());
    if let Some(key) = key {
        INTERNAL_DROPS.with(|drops| {
            drops.borrow_mut().insert(key);
        });
        true
    } else {
        false
    }
}

pub fn transfer_action(
    selected: gdk::DragAction,
    offered: gdk::DragAction,
    supported: gdk::DragAction,
) -> Option<TransferAction> {
    if !selected.is_unique() || !offered.contains(selected) || !supported.contains(selected) {
        return None;
    }
    match selected {
        gdk::DragAction::COPY => Some(TransferAction::Copy),
        gdk::DragAction::MOVE => Some(TransferAction::Move),
        _ => None,
    }
}

fn preferred_drop_action(
    offered: gdk::DragAction,
    modifiers: gdk::ModifierType,
) -> gdk::DragAction {
    let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
    let wanted = match (ctrl, shift) {
        (true, true) => return gdk::DragAction::empty(),
        (true, false) => gdk::DragAction::COPY,
        (false, true) => gdk::DragAction::MOVE,
        (false, false) if offered.contains(gdk::DragAction::MOVE) => gdk::DragAction::MOVE,
        (false, false) => gdk::DragAction::COPY,
    };
    if offered.contains(wanted) {
        wanted
    } else {
        gdk::DragAction::empty()
    }
}

fn negotiate_drop_action(drop: &gdk::Drop, supported: gdk::DragAction) -> gdk::DragAction {
    let offered = drop.actions();
    let preferred = preferred_drop_action(offered, drop.device().modifier_state());
    let available = offered & supported;
    let selected = if !preferred.is_empty() && available.contains(preferred) {
        preferred
    } else {
        gdk::DragAction::empty()
    };
    drop.status(available, selected);
    selected
}

fn choose_drop_action(drop: &gdk::Drop, supported: gdk::DragAction) -> Option<TransferAction> {
    let offered = drop.actions();
    let selected = drop
        .drag()
        .map(|drag| drag.selected_action())
        .filter(|action| !action.is_empty())
        .unwrap_or_else(|| preferred_drop_action(offered, drop.device().modifier_state()));
    transfer_action(selected, offered, supported)
}

pub fn prepared_file_list(uris: &[String]) -> Option<PreparedDrag> {
    if uris.is_empty() {
        return None;
    }
    let files: Vec<gio::File> = uris.iter().map(|uri| gio::File::for_uri(uri)).collect();
    let list = gdk::FileList::from_array(&files);
    let file_list_provider = gdk::ContentProvider::for_value(&list.to_value());
    let uri_list = uris
        .iter()
        .map(|uri| format!("{uri}\r\n"))
        .collect::<String>();
    let uri_provider = gdk::ContentProvider::for_bytes(
        "text/uri-list",
        &glib::Bytes::from_owned(uri_list.into_bytes()),
    );
    Some(PreparedDrag {
        uris: uris.to_vec(),
        provider: gdk::ContentProvider::new_union(&[file_list_provider, uri_provider]),
    })
}

pub fn file_list_uris(value: &glib::Value) -> Option<Vec<String>> {
    let files = value.get::<gdk::FileList>().ok()?.files();
    Some(
        files
            .into_iter()
            .map(|file| file.uri().to_string())
            .collect(),
    )
}

pub fn attach_drop_target(
    widget: &impl IsA<gtk::Widget>,
    destination: Destination,
    on_drop: DropHandler,
) {
    let actions = gdk::DragAction::COPY | gdk::DragAction::MOVE;
    let formats = gdk::ContentFormats::builder()
        .add_type(gdk::FileList::static_type())
        .add_mime_type("text/uri-list")
        .build();
    let target = gtk::DropTargetAsync::new(Some(formats), actions);
    target.connect_accept(move |_, drop| {
        let formats = drop.formats();
        (formats.contains_type(gdk::FileList::static_type())
            || formats.contain_mime_type("text/uri-list"))
            && drop.actions().intersects(actions)
    });
    target.connect_drag_enter(move |_, drop, _, _| negotiate_drop_action(drop, actions));
    target.connect_drag_motion(move |_, drop, _, _| negotiate_drop_action(drop, actions));
    target.connect_drop(move |_, drop, _, _| {
        let Some(action) = choose_drop_action(drop, actions) else {
            drop.finish(gdk::DragAction::empty());
            return false;
        };
        let Some(destination) = destination() else {
            drop.finish(gdk::DragAction::empty());
            return false;
        };
        let destination = destination.to_string();
        let read_drop = drop.clone();
        let callback_drop = drop.clone();
        let on_drop = on_drop.clone();
        read_drop.read_value_async(
            gdk::FileList::static_type(),
            glib::Priority::DEFAULT,
            gio::Cancellable::NONE,
            move |result| {
                let drop = callback_drop;
                let Ok(value) = result else {
                    drop.finish(gdk::DragAction::empty());
                    return;
                };
                let Some(uris) = file_list_uris(&value).filter(|uris| !uris.is_empty()) else {
                    drop.finish(gdk::DragAction::empty());
                    return;
                };
                let internal = mark_internal_drop(&drop);
                on_drop(uris, destination, action, internal, drop);
            },
        );
        true
    });
    widget.add_controller(target);
}

/// Install row controllers after each factory bind. List rows are recycled,
/// so replacing only drag/drop controllers prevents stale row identities.
pub fn attach_row_drag_drop(
    widget: &impl IsA<gtk::Widget>,
    uri: String,
    is_dir: bool,
    prepare: PrepareHandler,
    on_drop: DropHandler,
    external_move: ExternalMoveHandler,
) {
    let widget = widget.as_ref();
    let controllers = widget.observe_controllers();
    let mut remove = Vec::new();
    for index in 0..controllers.n_items() {
        if let Some(controller) = controllers
            .item(index)
            .and_downcast::<gtk::EventController>()
        {
            if controller.clone().downcast::<gtk::DragSource>().is_ok()
                || controller
                    .clone()
                    .downcast::<gtk::DropTargetAsync>()
                    .is_ok()
            {
                remove.push(controller);
            }
        }
    }
    for controller in remove {
        widget.remove_controller(&controller);
    }

    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
    let drag_uris: Rc<RefCell<Option<Vec<String>>>> = Rc::new(RefCell::new(None));
    source.connect_prepare({
        let drag_uris = drag_uris.clone();
        move |_, _, _| {
            let prepared = prepare()?;
            *drag_uris.borrow_mut() = Some(prepared.uris);
            Some(prepared.provider)
        }
    });
    source.connect_drag_begin(|_, drag| register_drag(drag));
    source.connect_drag_end({
        let drag_uris = drag_uris.clone();
        move |_, drag, delete_data| {
            let internal = finish_drag(drag);
            if delete_data && !internal && drag.selected_action() == gdk::DragAction::MOVE {
                if let Some(uris) = drag_uris.borrow_mut().take() {
                    external_move(uris);
                }
            } else {
                drag_uris.borrow_mut().take();
            }
        }
    });
    widget.add_controller(source);

    if is_dir {
        let destination = Rc::new(move || Some(uri.clone()));
        attach_drop_target(widget, destination, on_drop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_action_must_be_offered_and_supported() {
        let both = gdk::DragAction::COPY | gdk::DragAction::MOVE;
        assert_eq!(
            transfer_action(gdk::DragAction::COPY, both, both),
            Some(TransferAction::Copy)
        );
        assert_eq!(
            transfer_action(gdk::DragAction::MOVE, both, both),
            Some(TransferAction::Move)
        );
        assert_eq!(
            transfer_action(gdk::DragAction::MOVE, gdk::DragAction::COPY, both),
            None
        );
        assert_eq!(
            transfer_action(gdk::DragAction::COPY, both, gdk::DragAction::MOVE),
            None
        );
    }

    #[test]
    fn link_and_ambiguous_actions_are_never_accepted() {
        let both = gdk::DragAction::COPY | gdk::DragAction::MOVE;
        assert_eq!(
            transfer_action(gdk::DragAction::LINK, gdk::DragAction::LINK, both),
            None
        );
        assert_eq!(transfer_action(both, both, both), None);
    }

    #[test]
    fn modifiers_choose_only_an_offered_copy_or_move_action() {
        let both = gdk::DragAction::COPY | gdk::DragAction::MOVE;
        assert_eq!(
            preferred_drop_action(both, gdk::ModifierType::CONTROL_MASK),
            gdk::DragAction::COPY
        );
        assert_eq!(
            preferred_drop_action(both, gdk::ModifierType::SHIFT_MASK),
            gdk::DragAction::MOVE
        );
        assert!(preferred_drop_action(
            both,
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
        )
        .is_empty());
        assert_eq!(
            preferred_drop_action(gdk::DragAction::COPY, gdk::ModifierType::SHIFT_MASK),
            gdk::DragAction::empty()
        );
    }

    #[test]
    fn file_list_provider_advertises_interoperable_uri_list() {
        let prepared = prepared_file_list(&[
            "file:///tmp/report%20final.txt".to_string(),
            "file:///tmp/caf%C3%A9.txt".to_string(),
        ])
        .unwrap();
        let formats = prepared.provider.formats();
        assert!(formats.contains_type(gdk::FileList::static_type()));
        assert!(formats.contain_mime_type("text/uri-list"));
    }
}
