//! GTK file drag-and-drop helpers. File lists use GDK's interoperable
//! `GdkFileList` value, which is exported to other applications as URIs.

use gtk::gdk::prelude::*;
use gtk::{gdk, gio, glib, prelude::*};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferAction {
    Copy,
    Move,
    Link,
}

pub type DropHandler = Rc<dyn Fn(Vec<String>, String, TransferAction, bool, gdk::Drop)>;
pub type Destination = Rc<dyn Fn() -> Option<String>>;
/// Destination resolved from the drop coordinates, in the coordinate
/// space of the target widget. `None` means the point is not a valid
/// target: the hover is declined and the drop refused.
pub type ResolveDestination = Rc<dyn Fn(f64, f64) -> Option<String>>;
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
        gdk::DragAction::LINK => Some(TransferAction::Link),
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
    } else if offered.contains(gdk::DragAction::LINK) {
        gdk::DragAction::LINK
    } else {
        gdk::DragAction::empty()
    }
}

fn preferred_for_target(
    offered: gdk::DragAction,
    modifiers: gdk::ModifierType,
    supported: gdk::DragAction,
) -> gdk::DragAction {
    let available = offered & supported;
    let wanted = preferred_drop_action(offered, modifiers);
    if !wanted.is_empty() && available.contains(wanted) {
        wanted
    } else if available.contains(gdk::DragAction::LINK) {
        gdk::DragAction::LINK
    } else if supported.contains(gdk::DragAction::LINK) && available.contains(gdk::DragAction::COPY)
    {
        // Special bookmark targets may safely reinterpret COPY as adding a
        // link-like entry. Ordinary folders keep the old no-fallback behavior.
        gdk::DragAction::COPY
    } else {
        gdk::DragAction::empty()
    }
}

fn action_bits(action: TransferAction) -> gdk::DragAction {
    match action {
        TransferAction::Copy => gdk::DragAction::COPY,
        TransferAction::Move => gdk::DragAction::MOVE,
        TransferAction::Link => gdk::DragAction::LINK,
    }
}

/// Pure policy for a coordinate-resolved destination: with no destination
/// under the pointer every action is declined, so the source is shown a
/// refusal instead of a transfer that may never start. Otherwise the usual
/// modifier-based negotiation applies.
fn destination_negotiation(
    offered: gdk::DragAction,
    modifiers: gdk::ModifierType,
    supported: gdk::DragAction,
    destination_available: bool,
) -> Option<TransferAction> {
    if !destination_available {
        return None;
    }
    transfer_action(
        preferred_for_target(offered, modifiers, supported),
        offered,
        supported,
    )
}

/// Negotiates the drop action for a coordinate-resolved destination.
/// While no destination resolves under the pointer every action is
/// declined, so the source sees an invalid target instead of a transfer
/// that may never start. The drop stays unclaimed for other targets.
fn negotiate_drop_action_for(
    drop: &gdk::Drop,
    supported: gdk::DragAction,
    destination_available: bool,
) -> gdk::DragAction {
    let offered = drop.actions();
    let available = offered & supported;
    let selected = destination_negotiation(
        offered,
        drop.device().modifier_state(),
        supported,
        destination_available,
    )
    .map(action_bits)
    .unwrap_or(gdk::DragAction::empty());
    drop.status(available, selected);
    selected
}

fn choose_drop_action(drop: &gdk::Drop, supported: gdk::DragAction) -> Option<TransferAction> {
    let offered = drop.actions();
    let selected = drop
        .drag()
        .map(|drag| drag.selected_action())
        .unwrap_or_else(gdk::DragAction::empty);
    choose_action(selected, offered, drop.device().modifier_state(), supported)
}

fn choose_action(
    selected: gdk::DragAction,
    offered: gdk::DragAction,
    modifiers: gdk::ModifierType,
    supported: gdk::DragAction,
) -> Option<TransferAction> {
    let action =
        if selected.is_unique() && offered.contains(selected) && supported.contains(selected) {
            selected
        } else {
            preferred_for_target(offered, modifiers, supported)
        };
    transfer_action(action, offered, supported)
}

/// Refuses a drop at the current position, keeping the `GtkDropTargetAsync`
/// contract in exactly one place.
///
/// `complete` performs the single completion of the drop (always an empty
/// action here, so the source is told the drop failed and keeps its data).
/// The value returned to GTK is what the `drop` signal handler must report:
/// `TRUE` means this handler owns the completion and GTK stops processing
/// the event, `FALSE` means "not a drop zone" and lets the event keep
/// propagating while the drop is left untouched
/// (`gtk_drop_target_async_drop` / `GtkDropTargetAsync::drop` in
/// gtk/gtkdroptargetasync.c).
///
/// Finishing the drop and then returning `FALSE` would hand an
/// already-finished `GdkDrop` to the next controller — a GLib critical from
/// the second `gdk_drop_finish()` (`gdk_drop_finish` requires the DROPPING
/// state) — and returning `FALSE` without finishing would leave the drop
/// unfinished (`gdk_drop_finalize` warns about that) with the source waiting
/// for a completion that never arrives. One finish, one handler.
fn refuse_drop(complete: impl FnOnce()) -> bool {
    complete();
    true
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

fn parse_uri_list(bytes: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let uris: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| {
            glib::Uri::parse(line, glib::UriFlags::NONE).is_ok_and(|uri| !uri.scheme().is_empty())
        })
        .map(str::to_string)
        .collect();
    (!uris.is_empty()).then_some(uris)
}

const MAX_URI_LIST_BYTES: usize = 1024 * 1024;

fn read_uri_stream(
    stream: gio::InputStream,
    mut accumulated: Vec<u8>,
    callback: Box<dyn FnOnce(Result<Vec<u8>, glib::Error>)>,
) {
    // Read one byte beyond the maximum when necessary, so an exactly-full
    // payload can still be distinguished from an oversized one at EOF.
    let remaining = MAX_URI_LIST_BYTES + 1 - accumulated.len();
    let read_stream = stream.clone();
    stream.read_bytes_async(
        remaining.min(64 * 1024),
        glib::Priority::DEFAULT,
        gio::Cancellable::NONE,
        move |result| match result {
            Err(error) => callback(Err(error)),
            Ok(bytes) if bytes.is_empty() => callback(Ok(accumulated)),
            Ok(bytes) => {
                accumulated.extend_from_slice(bytes.as_ref());
                if accumulated.len() > MAX_URI_LIST_BYTES {
                    callback(Err(glib::Error::new(
                        gio::IOErrorEnum::NoSpace,
                        "URI list exceeds the maximum size",
                    )));
                } else {
                    read_uri_stream(read_stream, accumulated, callback);
                }
            }
        },
    );
}

pub fn attach_drop_target(
    widget: &impl IsA<gtk::Widget>,
    destination: Destination,
    on_drop: DropHandler,
) {
    attach_drop_target_with_actions(
        widget,
        destination,
        on_drop,
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
}

pub fn attach_drop_target_with_actions(
    widget: &impl IsA<gtk::Widget>,
    destination: Destination,
    on_drop: DropHandler,
    actions: gdk::DragAction,
) {
    attach_drop_target_with_actions_and_hover(widget, destination, on_drop, actions, None);
}

pub fn attach_drop_target_with_actions_and_hover(
    widget: &impl IsA<gtk::Widget>,
    destination: Destination,
    on_drop: DropHandler,
    actions: gdk::DragAction,
    hover: Option<Rc<dyn Fn(bool)>>,
) {
    let resolve: ResolveDestination = Rc::new(move |_, _| destination());
    attach_drop_target_resolving(widget, resolve, on_drop, actions, hover);
}

/// Drop target whose destination depends on the pointer position, such as
/// a tab bar where each tab is a different folder. The resolver runs on
/// every motion and on the drop itself, in the widget's coordinate space.
pub fn attach_drop_target_with_resolver(
    widget: &impl IsA<gtk::Widget>,
    resolve: ResolveDestination,
    on_drop: DropHandler,
    actions: gdk::DragAction,
) {
    attach_drop_target_resolving(widget, resolve, on_drop, actions, None);
}

fn attach_drop_target_resolving(
    widget: &impl IsA<gtk::Widget>,
    resolve: ResolveDestination,
    on_drop: DropHandler,
    actions: gdk::DragAction,
    hover: Option<Rc<dyn Fn(bool)>>,
) {
    let formats = gdk::ContentFormats::builder()
        .add_type(gdk::FileList::static_type())
        .add_mime_type("text/uri-list")
        .build();
    let target = gtk::DropTargetAsync::new(Some(formats), actions);
    target.connect_drag_enter({
        let hover = hover.clone();
        let resolve = resolve.clone();
        move |_, drop, x, y| {
            if let Some(hover) = &hover {
                hover(true);
            }
            let valid = resolve(x, y).is_some();
            negotiate_drop_action_for(drop, actions, valid)
        }
    });
    target.connect_drag_motion({
        let hover = hover.clone();
        let resolve = resolve.clone();
        move |_, drop, x, y| {
            if let Some(hover) = &hover {
                hover(true);
            }
            let valid = resolve(x, y).is_some();
            negotiate_drop_action_for(drop, actions, valid)
        }
    });
    target.connect_drag_leave({
        let hover = hover.clone();
        move |_, _| {
            if let Some(hover) = &hover {
                hover(false);
            }
        }
    });
    target.connect_drop(move |_, drop, x, y| {
        if let Some(hover) = &hover {
            hover(false);
        }
        let Some(action) = choose_drop_action(drop, actions) else {
            return refuse_drop(|| drop.finish(gdk::DragAction::empty()));
        };
        let Some(destination) = resolve(x, y) else {
            return refuse_drop(|| drop.finish(gdk::DragAction::empty()));
        };
        let read_drop = drop.clone();
        let callback_drop = drop.clone();
        let on_drop = on_drop.clone();
        read_drop.read_value_async(
            gdk::FileList::static_type(),
            glib::Priority::DEFAULT,
            gio::Cancellable::NONE,
            move |result| {
                let drop = callback_drop;
                if let Ok(value) = result {
                    if let Some(uris) = file_list_uris(&value).filter(|uris| !uris.is_empty()) {
                        let internal = mark_internal_drop(&drop);
                        on_drop(uris, destination, action, internal, drop);
                        return;
                    }
                }
                let fallback_drop = drop.clone();
                let fallback_on_drop = on_drop.clone();
                let fallback_destination = destination.clone();
                drop.read_async(
                    &["text/uri-list"],
                    glib::Priority::DEFAULT,
                    gio::Cancellable::NONE,
                    move |result| {
                        let Ok((stream, _)) = result else {
                            fallback_drop.finish(gdk::DragAction::empty());
                            return;
                        };
                        read_uri_stream(
                            stream,
                            Vec::new(),
                            Box::new(move |result| {
                                let Ok(bytes) = result else {
                                    fallback_drop.finish(gdk::DragAction::empty());
                                    return;
                                };
                                let Some(uris) = parse_uri_list(&bytes) else {
                                    fallback_drop.finish(gdk::DragAction::empty());
                                    return;
                                };
                                let internal = mark_internal_drop(&fallback_drop);
                                fallback_on_drop(
                                    uris,
                                    fallback_destination,
                                    action,
                                    internal,
                                    fallback_drop,
                                );
                            }),
                        );
                    },
                );
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
    source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE | gdk::DragAction::LINK);
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
    fn refusing_a_drop_completes_it_exactly_once_and_reports_it_as_handled() {
        // The refusal completes the drop itself, so it must also report the
        // drop as handled: GTK then stops the event and no other controller
        // can finish (or read) the same `GdkDrop` a second time.
        let completions = std::cell::Cell::new(0usize);
        assert!(refuse_drop(|| completions.set(completions.get() + 1)));
        assert_eq!(completions.get(), 1);
    }

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
    fn link_is_accepted_only_when_the_drop_target_supports_it() {
        let both = gdk::DragAction::COPY | gdk::DragAction::MOVE;
        assert_eq!(
            transfer_action(gdk::DragAction::LINK, gdk::DragAction::LINK, both),
            None
        );
        assert_eq!(
            transfer_action(
                gdk::DragAction::LINK,
                gdk::DragAction::LINK,
                gdk::DragAction::LINK
            ),
            Some(TransferAction::Link)
        );
        assert_eq!(transfer_action(both, both, both), None);
        assert_eq!(
            preferred_drop_action(gdk::DragAction::LINK, gdk::ModifierType::empty()),
            gdk::DragAction::LINK
        );
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
    fn special_destination_never_selects_move_and_falls_back_to_copy() {
        let safe = gdk::DragAction::COPY | gdk::DragAction::LINK;
        let offered = gdk::DragAction::COPY | gdk::DragAction::MOVE;
        assert_eq!(
            preferred_for_target(offered, gdk::ModifierType::empty(), safe),
            gdk::DragAction::COPY
        );
        assert_eq!(
            preferred_for_target(offered, gdk::ModifierType::empty(), offered),
            gdk::DragAction::MOVE
        );
        assert_ne!(
            preferred_for_target(offered, gdk::ModifierType::SHIFT_MASK, safe),
            gdk::DragAction::MOVE
        );
        let folders = gdk::DragAction::COPY | gdk::DragAction::MOVE;
        assert!(preferred_for_target(
            gdk::DragAction::COPY,
            gdk::ModifierType::SHIFT_MASK,
            folders,
        )
        .is_empty());
        assert!(preferred_for_target(
            folders,
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
            folders,
        )
        .is_empty());
    }

    #[test]
    fn selected_action_is_used_only_when_supported_and_offered() {
        let folder = gdk::DragAction::COPY | gdk::DragAction::MOVE;
        let special = gdk::DragAction::COPY | gdk::DragAction::LINK;
        let modifiers = gdk::ModifierType::empty();

        assert_eq!(
            choose_action(gdk::DragAction::MOVE, folder, modifiers, folder),
            Some(TransferAction::Move)
        );
        assert_eq!(
            choose_action(gdk::DragAction::MOVE, folder, modifiers, special),
            Some(TransferAction::Copy)
        );
        assert_eq!(
            choose_action(
                gdk::DragAction::LINK,
                gdk::DragAction::COPY | gdk::DragAction::LINK,
                modifiers,
                special
            ),
            Some(TransferAction::Link)
        );
        assert_eq!(
            choose_action(
                gdk::DragAction::empty(),
                gdk::DragAction::empty(),
                modifiers,
                special
            ),
            None
        );
    }

    #[test]
    fn no_resolved_destination_declines_every_action() {
        let offered = gdk::DragAction::COPY | gdk::DragAction::MOVE;
        // Without a destination under the pointer nothing may be offered,
        // so an external source never believes a transfer will start.
        assert_eq!(
            destination_negotiation(offered, gdk::ModifierType::empty(), offered, false),
            None
        );
        assert_eq!(
            destination_negotiation(
                gdk::DragAction::LINK,
                gdk::ModifierType::CONTROL_MASK,
                gdk::DragAction::COPY | gdk::DragAction::LINK,
                false
            ),
            None
        );
    }

    #[test]
    fn resolved_destination_negotiates_move_for_external_sources() {
        let offered = gdk::DragAction::COPY | gdk::DragAction::MOVE;
        // The tab bar advertises COPY|MOVE like folder targets: an external
        // MOVE drag stays selectable, so the feature is not silently
        // downgraded to copy while the ack is deferred.
        assert_eq!(
            destination_negotiation(offered, gdk::ModifierType::empty(), offered, true),
            Some(TransferAction::Move)
        );
        assert_eq!(
            destination_negotiation(offered, gdk::ModifierType::CONTROL_MASK, offered, true),
            Some(TransferAction::Copy)
        );
        assert_eq!(
            destination_negotiation(offered, gdk::ModifierType::SHIFT_MASK, offered, true),
            Some(TransferAction::Move)
        );
        // A COPY-only offer still resolves Copy.
        assert_eq!(
            destination_negotiation(
                gdk::DragAction::COPY,
                gdk::ModifierType::empty(),
                gdk::DragAction::COPY,
                true
            ),
            Some(TransferAction::Copy)
        );
    }

    #[test]
    fn uri_list_parser_keeps_multiple_encoded_uris_and_ignores_comments() {
        assert_eq!(
            parse_uri_list(b"# comment\r\nfile:///tmp/a%20b\r\nfile:///tmp/folder/\n"),
            Some(vec![
                "file:///tmp/a%20b".to_string(),
                "file:///tmp/folder/".to_string()
            ])
        );
        assert_eq!(parse_uri_list(b"\r\n# only comment\nplain text\n"), None);
        assert_eq!(parse_uri_list(b"\xff"), None);
    }

    fn collect_memory_stream(chunks: &[&[u8]]) -> Result<Vec<u8>, glib::Error> {
        let stream = gio::MemoryInputStream::new();
        for chunk in chunks {
            stream.add_bytes(&glib::Bytes::from(*chunk));
        }
        let context = glib::MainContext::new();
        let _guard = context.acquire().unwrap();
        let loop_ = glib::MainLoop::new(Some(&context), false);
        let result = Rc::new(RefCell::new(None));
        context
            .with_thread_default(|| {
                read_uri_stream(
                    stream.upcast(),
                    Vec::new(),
                    Box::new({
                        let loop_ = loop_.clone();
                        let result = result.clone();
                        move |read_result| {
                            *result.borrow_mut() = Some(read_result);
                            loop_.quit();
                        }
                    }),
                );
                loop_.run();
            })
            .unwrap();
        let outcome = result.borrow_mut().take().unwrap();
        outcome
    }

    #[test]
    fn uri_list_stream_reads_through_short_chunks_to_eof() {
        let bytes =
            collect_memory_stream(&[b"file:///tmp/a%", b"20b\r\n", b"file:///tmp/dir\n"]).unwrap();
        assert_eq!(
            parse_uri_list(&bytes),
            Some(vec![
                "file:///tmp/a%20b".to_string(),
                "file:///tmp/dir".to_string()
            ])
        );
    }

    #[test]
    fn oversized_uri_list_stream_is_rejected_before_parsing() {
        let oversized = vec![b'x'; MAX_URI_LIST_BYTES + 1];
        let result = collect_memory_stream(&[&oversized]);
        assert!(result.is_err());
        assert!(parse_uri_list(&result.unwrap_or_default()).is_none());
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
