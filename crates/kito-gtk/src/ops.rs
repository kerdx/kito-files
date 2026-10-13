//! File operations: clipboard, trash, delete, rename, copy/move.
//! Mutations use the application-wide FIFO on a worker; read-only background
//! work uses the regular helper. Results return to GTK for toast + reload.

use crate::job_center::{JobCenter, JobCompletionGuard, JobItemState};
use crate::operation_queue::{OperationQueue, UndoRecord, UndoStartError};
use crate::preferences::PreferenceStore;
use crate::tabs::TabManager;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Clone, Default)]
pub struct Clipboard {
    pub uris: Vec<String>,
    pub cut: bool,
}

/// A previously failed cut may be retried only with the object identity
/// captured before its first MOVE. `Unavailable` is an explicit fail-closed
/// policy, not the absence of a prior failure.
#[derive(Clone, Debug, PartialEq, Eq)]
enum MoveIdentityGuard {
    Stable(kito_core::FileIdentity),
    /// A known partial, uncertain or otherwise non-retryable MOVE result.
    /// Keep its diagnostic marker and never attempt another move from Ctrl+V.
    Withheld(String),
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CutDispatch {
    generation: u64,
    uris: Vec<String>,
    identity_guards: HashMap<String, MoveIdentityGuard>,
}

/// Outcome of applying a finished cut: what the completion may do
/// with the system clipboard (only when it still belongs to the op).
#[derive(Debug, PartialEq)]
enum CutFinish {
    /// Everything moved: internal state consumed. GDK has no atomic
    /// compare-and-clear, so the published text remains rather than risking
    /// replacing a foreign provider which raced the asynchronous move.
    Consumed,
    /// Some entries failed or were not part of this dispatch: unresolved
    /// cut entries remain in the internal clipboard.
    Partial,
    /// Clipboard moved on meanwhile: leave the new state alone.
    Untouched,
}

/// What to paste for freshly-read system clipboard text.
#[derive(Debug, PartialEq)]
enum PasteDecision {
    /// Clipboard changed again while reading: drop the result.
    Stale,
    /// System text still matches what Kito published: use the internal
    /// entries, preserving the copy/cut intent.
    Internal { uris: Vec<String>, cut: bool },
    /// Foreign content with usable files: copy them, never move.
    External { uris: Vec<String> },
    /// No usable files: explain, never fall back to old URIs.
    Unsupported,
}

/// What the UI must do for a resolved paste, including the tracker
/// interaction. Free of GDK and widgets so the consumed-cut tombstone, the
/// refusal of a concurrent move and the tracker borrow discipline are
/// testable.
#[derive(Debug, PartialEq)]
enum PasteAction {
    /// Show this localized message; start no job.
    Explain(&'static str),
    /// Paste these URIs as copies; never move.
    Copy(Vec<String>),
    /// Start this cut move.
    Move(CutDispatch),
    /// A move for this generation is already running.
    MoveRefused,
}

fn paste_action(
    clipboard: &RefCell<ClipTracker>,
    decision: &PasteDecision,
    text: Option<&str>,
) -> PasteAction {
    match decision {
        PasteDecision::Stale => PasteAction::Explain("clip-changed"),
        PasteDecision::Internal { uris, cut: true } if uris.is_empty() => {
            // A successfully moved cut keeps a consumed tombstone because
            // the system provider may still hold its published URI text.
            // Those items are gone: explain the empty clipboard instead of
            // dispatching another MOVE job for them.
            PasteAction::Explain("clip-empty")
        }
        PasteDecision::Internal { cut: true, .. } => match clipboard.borrow_mut().begin_cut() {
            Some(dispatch) => PasteAction::Move(dispatch),
            None => PasteAction::MoveRefused,
        },
        PasteDecision::Internal { uris, cut: false } => PasteAction::Copy(uris.clone()),
        PasteDecision::External { uris } => PasteAction::Copy(uris.clone()),
        PasteDecision::Unsupported => {
            let key = if text.is_none_or(|t| t.trim().is_empty()) {
                "clip-empty"
            } else {
                "clip-nofiles"
            };
            PasteAction::Explain(key)
        }
    }
}

/// Toast key for a copy/cut publish attempt. Success is declared only when
/// the provider really reached the system clipboard: a failed publish never
/// reports `Copied`/`Cut`, whatever the requested action was.
fn publish_toast_key(published: bool, cut: bool) -> &'static str {
    match (published, cut) {
        (true, false) => "toast-copied",
        (true, true) => "toast-cut",
        (false, _) => "clip-publish-failed",
    }
}

#[derive(Clone, Debug)]
enum ItemState {
    Succeeded,
    Failed(String),
    Cancelled(String),
}

/// Per-item result retained for a concise summary, detail view and precise
/// retries. `operation` is a Fluent key; source/destination remain URIs so
/// non-local and special-character paths are not lost.
#[derive(Clone, Debug)]
struct OperationItemResult {
    operation: &'static str,
    source: String,
    destination: Option<String>,
    state: ItemState,
    /// Stable MOVE source identity captured before the first mutation. Kept
    /// only for retry authorization; never included in user-facing details.
    retry_identity: Option<kito_core::FileIdentity>,
}

const MOVE_RETRY_IDENTITY_CHANGED_MARKER: &str =
    kito_core::MOVE_RETRY_IDENTITY_CHANGED_MARKER;
const MOVE_RETRY_IDENTITY_UNAVAILABLE_MARKER: &str =
    "retry withheld: MOVE source identity unavailable";

type RetryHandler = Rc<dyn Fn(Vec<OperationItemResult>)>;

/// Worker-side hook to the running F08 job. `submit_job` installs the
/// job for the duration of its worker thread; the closure returns false
/// when no job context is set (defensive: callers treat it as progress
/// without a registry entry).
static CURRENT_JOB: std::sync::Mutex<Option<Arc<crate::job_center::Job>>> =
    std::sync::Mutex::new(None);

struct CurrentJobGuard;

impl CurrentJobGuard {
    fn install(job: Arc<crate::job_center::Job>) -> Self {
        *CURRENT_JOB.lock().unwrap() = Some(job);
        Self
    }
}

impl Drop for CurrentJobGuard {
    fn drop(&mut self) {
        if let Ok(mut current) = CURRENT_JOB.lock() {
            *current = None;
        }
    }
}

fn job_report(f: impl FnOnce(&crate::job_center::Job)) {
    if let Ok(guard) = CURRENT_JOB.lock() {
        if let Some(job) = guard.as_ref() {
            f(job);
        }
    }
}

fn is_queued_cancellation(failure: &crate::operation_queue::WorkerFailure) -> bool {
    matches!(failure, crate::operation_queue::WorkerFailure::CancelledBeforeStart)
}

impl OperationItemResult {
    /// Registry view of one finished element: the job center keeps the
    /// same classification (B03) without duplicating the semantics.
    fn job_state(&self) -> JobItemState {
        match &self.state {
            ItemState::Succeeded => JobItemState::Succeeded,
            ItemState::Failed(error) => JobItemState::Failed(error.clone()),
            ItemState::Cancelled(error) => JobItemState::Cancelled(error.clone()),
        }
    }
}

/// Cut clipboard state must retain every unsuccessful source, independently
/// of whether the Operations dialog offers a safe retry for that outcome.
#[cfg(test)]
fn failed_cut_uris(results: &[OperationItemResult]) -> Vec<String> {
    results
        .iter()
        .filter(|result| !matches!(result.state, ItemState::Succeeded))
        .map(|result| result.source.clone())
        .collect()
}

/// Captures only the entries which passed the retry policy. MOVE identities
/// remain private to this transfer state and are checked again in the worker.
fn retry_transfer_capture(
    results: Vec<OperationItemResult>,
) -> (Vec<String>, HashMap<String, MoveIdentityGuard>) {
    let mut uris = Vec::new();
    let mut identities = HashMap::new();
    for item in results.into_iter().filter(OperationItemResult::needs_retry) {
        if item.operation == "operation-move" {
            if let Some(identity) = item.retry_identity {
                identities
                    .entry(item.source.clone())
                    .or_insert(MoveIdentityGuard::Stable(identity));
            }
        }
        uris.push(item.source);
    }
    (uris, identities)
}

fn operation_summary_args(
    operation: String,
    succeeded: usize,
    failed: usize,
    cancelled: usize,
    total: usize,
) -> kito_i18n::FluentArgs<'static> {
    let mut args = kito_i18n::FluentArgs::new();
    args.set("operation", operation);
    args.set("count", succeeded as u64);
    args.set("succeeded", succeeded as u64);
    args.set("failed", failed as u64);
    args.set("cancelled", cancelled as u64);
    args.set("total", total as u64);
    args
}

/// An inverse may only target an object whose backend identity survived the
/// operation. In particular, GIO may implement a cross-filesystem move as
/// copy+delete, so an undo record must not retain the pre-move identity.
fn unchanged_undo_identity(
    before: Option<kito_core::FileIdentity>,
    after: Option<kito_core::FileIdentity>,
) -> Option<kito_core::FileIdentity> {
    match (before, after) {
        (Some(before), Some(after)) if before == after => Some(after),
        _ => None,
    }
}

struct DropCompletion {
    drop: gdk::Drop,
    action: crate::dnd::TransferAction,
    finished: Cell<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedTransfer {
    uris: Vec<String>,
    destination: String,
    cut: bool,
    move_with_backend: bool,
    identity_guards: HashMap<String, MoveIdentityGuard>,
}

enum MoveIdentityAttempt {
    Authorized(Option<kito_core::FileIdentity>),
    Withheld(String),
}

/// Resolves the worker's MOVE authorization. Fresh moves capture the current
/// identity; retried moves use only the original guard, and an unavailable
/// original identity is never upgraded by querying the current path.
fn move_identity_attempt(
    uri: &str,
    guards: &HashMap<String, MoveIdentityGuard>,
    capture: impl FnOnce() -> Option<kito_core::FileIdentity>,
) -> MoveIdentityAttempt {
    match guards.get(uri) {
        Some(MoveIdentityGuard::Stable(identity)) => {
            MoveIdentityAttempt::Authorized(Some(identity.clone()))
        }
        Some(MoveIdentityGuard::Withheld(marker)) => {
            MoveIdentityAttempt::Withheld(marker.clone())
        }
        Some(MoveIdentityGuard::Unavailable) => {
            MoveIdentityAttempt::Withheld(MOVE_RETRY_IDENTITY_UNAVAILABLE_MARKER.to_string())
        }
        None => MoveIdentityAttempt::Authorized(capture()),
    }
}

fn worker_failure_has_unknown_outcome(failure: &crate::operation_queue::WorkerFailure) -> bool {
    matches!(
        failure,
        crate::operation_queue::WorkerFailure::Panicked
            | crate::operation_queue::WorkerFailure::ChannelClosed
    )
}

/// Provider identity is compared as a pointer value so this decision can be
/// tested without constructing or touching the process clipboard.
fn clipboard_provider_is_owned(current: Option<usize>, owned: Option<usize>) -> bool {
    matches!((current, owned), (Some(current), Some(owned)) if current == owned)
}

fn content_provider_identity(provider: Option<&gdk::ContentProvider>) -> Option<usize> {
    provider.map(|provider| provider.as_ptr() as usize)
}

impl DropCompletion {
    fn finish(&self, results: &[OperationItemResult]) {
        if self.finished.get() {
            return;
        }
        self.finished.set(true);
        self.drop.finish(ack_action(self.action, results));
    }
}

/// The acknowledgement sent to the drag source once the asynchronous
/// transfer settled. A MOVE may only be confirmed after every item
/// reached the destination: a partial or failed transfer leaves the
/// source data untouched. COPY and LINK confirm partial success.
fn ack_action(
    action: crate::dnd::TransferAction,
    results: &[OperationItemResult],
) -> gdk::DragAction {
    let succeeded = results
        .iter()
        .filter(|result| matches!(result.state, ItemState::Succeeded))
        .count();
    let all_succeeded = !results.is_empty()
        && succeeded == results.len()
        && results.iter().all(|result| !result.needs_retry());
    match action {
        crate::dnd::TransferAction::Copy if succeeded > 0 => gdk::DragAction::COPY,
        crate::dnd::TransferAction::Move if all_succeeded => gdk::DragAction::MOVE,
        crate::dnd::TransferAction::Link if succeeded > 0 => gdk::DragAction::LINK,
        _ => gdk::DragAction::empty(),
    }
}

impl Drop for DropCompletion {
    fn drop(&mut self) {
        if !self.finished.replace(true) {
            self.drop.finish(gdk::DragAction::empty());
        }
    }
}

impl OperationItemResult {
    fn from_result(
        operation: &'static str,
        source: String,
        destination: Option<String>,
        result: Result<(), glib::Error>,
    ) -> Self {
        let state = match result {
            Ok(()) => ItemState::Succeeded,
            Err(error) if error.matches(gio::IOErrorEnum::Cancelled) => {
                ItemState::Cancelled(error.to_string())
            }
            Err(error) => ItemState::Failed(error.to_string()),
        };
        Self {
            operation,
            source,
            destination,
            state,
            retry_identity: None,
        }
    }

    fn from_move_result(
        operation: &'static str,
        source: String,
        destination_on_error: String,
        retry_identity: Option<kito_core::FileIdentity>,
        result: Result<String, glib::Error>,
    ) -> Self {
        match result {
            Ok(destination) => Self::from_destination_result(
                operation,
                source,
                destination_on_error,
                Ok(destination),
            ),
            Err(error) => {
                let mut item = Self::from_result(
                    operation,
                    source,
                    Some(destination_on_error),
                    Err(error),
                );
                item.retry_identity = retry_identity;
                item
            }
        }
    }

    fn from_destination_result(
        operation: &'static str,
        source: String,
        destination_on_error: String,
        result: Result<String, glib::Error>,
    ) -> Self {
        match result {
            Ok(destination) => Self {
                operation,
                source,
                destination: Some(destination),
                state: ItemState::Succeeded,
                retry_identity: None,
            },
            Err(error) => {
                Self::from_result(operation, source, Some(destination_on_error), Err(error))
            }
        }
    }

    fn needs_retry(&self) -> bool {
        match &self.state {
            ItemState::Failed(error) | ItemState::Cancelled(error) => {
                let safe_error = !error.contains("partial copy output may remain at")
                    && !error.contains("partial move output may remain at")
                    && !error.contains(MOVE_RETRY_IDENTITY_CHANGED_MARKER);
                safe_error
                    && (self.operation != "operation-move" || self.retry_identity.is_some())
            }
            ItemState::Succeeded => false,
        }
    }
}

/// An external MOVE may already have removed its source at the destination.
/// Only that callback treats GIO NotFound as an already-completed item.
fn external_move_trash_result(
    uri: String,
    result: Result<(), glib::Error>,
    source_check: impl FnOnce() -> Result<(), glib::Error>,
) -> Option<OperationItemResult> {
    if result
        .as_ref()
        .is_err_and(|error| error.matches(gio::IOErrorEnum::NotFound))
    {
        match source_check() {
            Err(error) if error.matches(gio::IOErrorEnum::NotFound) => return None,
            // A cancelled check is the actionable outcome; uncertain checks
            // must not be used to infer that the external source is gone.
            Err(error) if error.matches(gio::IOErrorEnum::Cancelled) => {
                return Some(OperationItemResult::from_result(
                    "operation-trash",
                    uri,
                    Some(kito_core::TRASH_URI.to_string()),
                    Err(error),
                ));
            }
            _ => {}
        }
    }
    Some(OperationItemResult::from_result(
        "operation-trash",
        uri,
        Some(kito_core::TRASH_URI.to_string()),
        result,
    ))
}

/// Queries the source without following symlinks. Only an explicit GIO
/// NotFound proves the entry itself is absent; all other errors are uncertain.
fn external_move_source_check(
    uri: &str,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    gio::File::for_uri(uri)
        .query_info(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancellable),
        )
        .map(|_| ())
}

/// Clipboard tracker: internal entries stay valid only while they match
/// what Kito published. Ownership changes arrive via the GDK `changed`
/// notification (not text comparison); paste-time text matching is only
/// a safety net for missed notifications. GDK-free: fully testable.
#[derive(Default)]
pub(crate) struct ClipTracker {
    entries: Option<Clipboard>,
    published_text: Option<String>,
    generation: u64,
    own_change_pending: bool,
    /// Provider we published with: pointer identity (not text) proves
    /// a `changed` notification is ours.
    our_provider: Option<gdk::ContentProvider>,
    /// In-flight cut move, bound to its dispatch generation and URI subset:
    /// repeated pastes refuse to start a concurrent move, and completion may
    /// consume only the dispatched entries.
    cut_in_flight: Option<CutDispatch>,
    /// Identity policy for unresolved cut entries, private to this process.
    cut_identity_guards: HashMap<String, MoveIdentityGuard>,
}

impl ClipTracker {
    /// Internal copy/cut: store the entries, arm the own-change flag and
    /// return the text to publish. Bumps the generation so in-flight
    /// async reads go stale.
    fn publish(&mut self, entries: Clipboard) -> String {
        let text = entries.uris.join("\n");
        self.entries = Some(entries);
        self.published_text = Some(text.clone());
        self.own_change_pending = true;
        self.generation += 1;
        self.cut_in_flight = None;
        self.cut_identity_guards.clear();
        text
    }

    /// Notification for a change Kito itself published: keep the state.
    /// Idempotent: late duplicates are harmless.
    fn on_own_changed(&mut self) {
        self.own_change_pending = false;
    }

    /// Notification for a foreign change (plain text and unsupported
    /// content included): drop the internal state so nothing stale can
    /// be pasted. Bumps the generation to retire in-flight reads.
    fn on_external_changed(&mut self) {
        self.entries = None;
        self.published_text = None;
        self.own_change_pending = false;
        self.our_provider = None;
        self.generation += 1;
        self.cut_in_flight = None;
        self.cut_identity_guards.clear();
    }

    /// Starts a cut move for every current entry. Returns its generation,
    /// URI subset, and any stored identity guards; repeated pastes cannot
    /// duplicate the same generation while it is in flight.
    fn begin_cut(&mut self) -> Option<CutDispatch> {
        let uris = self.entries.as_ref()?.uris.clone();
        self.begin_cut_for(self.generation, &uris)
    }

    /// Starts a move subset only while the internal clipboard still belongs
    /// to the same cut generation and contains every requested URI. A newer
    /// clipboard can never be moved by an old completion dialog.
    fn begin_cut_for(&mut self, generation: u64, uris: &[String]) -> Option<CutDispatch> {
        if self.generation != generation
            || !self.entries.as_ref().is_some_and(|entries| {
                entries.cut && uris.iter().all(|uri| entries.uris.contains(uri))
            })
            || uris.is_empty()
            || self
                .cut_in_flight
                .as_ref()
                .is_some_and(|dispatch| dispatch.generation == generation)
        {
            return None;
        }
        let dispatch = CutDispatch {
            generation,
            uris: uris.to_vec(),
            identity_guards: uris
                .iter()
                .filter_map(|uri| {
                    self.cut_identity_guards
                        .get(uri)
                        .cloned()
                        .map(|guard| (uri.clone(), guard))
                })
                .collect(),
        };
        self.cut_in_flight = Some(dispatch.clone());
        Some(dispatch)
    }

    /// Retires a cut dispatch rejected before it starts, without consuming
    /// clipboard entries. Only the matching operation may clear the mark.
    fn cancel_cut(&mut self, op_generation: u64) {
        if self
            .cut_in_flight
            .as_ref()
            .is_some_and(|dispatch| dispatch.generation == op_generation)
        {
            self.cut_in_flight = None;
        }
    }

    /// Abandons exactly the dispatch captured by its completion guard. For an
    /// uncertain outcome, identity guards are invalidated before the in-flight
    /// mark is retired, but only while the same clipboard generation survives.
    fn abandon_cut_dispatch(
        &mut self,
        op_generation: u64,
        dispatched_uris: &[String],
        outcome_unknown: bool,
    ) {
        let matches_dispatch = self.cut_in_flight.as_ref().is_some_and(|dispatch| {
            dispatch.generation == op_generation && dispatch.uris == dispatched_uris
        });
        if !matches_dispatch {
            return;
        }
        if outcome_unknown
            && self.generation == op_generation
            && self.entries.as_ref().is_some_and(|entries| {
                entries.cut && dispatched_uris.iter().all(|uri| entries.uris.contains(uri))
            })
        {
            for uri in dispatched_uris {
                self.cut_identity_guards
                    .insert(uri.clone(), MoveIdentityGuard::Unavailable);
            }
        }
        self.cut_in_flight = None;
    }

    /// Applies a finished cut move. Always retires a matching in-flight
    /// mark; touches entries only when the clipboard still belongs to
    /// this operation (same generation): successful entries in its subset
    /// are consumed, failed entries retain their identity policy, and entries
    /// outside the subset remain unresolved. Never converts to copy.
    fn finish_cut(&mut self, op_generation: u64, results: &[OperationItemResult]) -> CutFinish {
        let Some(dispatch) = self
            .cut_in_flight
            .as_ref()
            .filter(|dispatch| dispatch.generation == op_generation)
            .cloned()
        else {
            return CutFinish::Untouched;
        };
        if self.generation != op_generation {
            self.cut_in_flight = None;
            return CutFinish::Untouched;
        }
        let Some(entries) = self.entries.as_ref() else {
            self.cut_in_flight = None;
            return CutFinish::Untouched;
        };
        let results_by_uri: HashMap<_, _> = results
            .iter()
            .map(|result| (result.source.as_str(), result))
            .collect();
        let mut unresolved = Vec::with_capacity(entries.uris.len());
        let mut next_identity_guards = self.cut_identity_guards.clone();
        for uri in &entries.uris {
            if !dispatch.uris.contains(uri) {
                unresolved.push(uri.clone());
                continue;
            }
            match results_by_uri.get(uri.as_str()).copied() {
                Some(result) if matches!(result.state, ItemState::Succeeded) => {
                    next_identity_guards.remove(uri);
                }
                Some(result) => {
                    let guard = match (result.needs_retry(), result.retry_identity.clone()) {
                        (true, Some(identity)) => MoveIdentityGuard::Stable(identity),
                        (false, Some(_)) => {
                            let marker = match &result.state {
                                ItemState::Failed(marker) | ItemState::Cancelled(marker) => {
                                    marker.clone()
                                }
                                ItemState::Succeeded => unreachable!("handled above"),
                            };
                            MoveIdentityGuard::Withheld(marker)
                        }
                        (_, None) => {
                            // A normal Ctrl+V may have been refused by an
                            // existing guard. Preserve that more specific
                            // policy instead of replacing it with Unavailable.
                            match (
                                next_identity_guards.get(uri),
                                &result.state,
                            ) {
                                (
                                    Some(MoveIdentityGuard::Withheld(existing)),
                                    ItemState::Failed(marker) | ItemState::Cancelled(marker),
                                ) if marker == existing => {
                                    MoveIdentityGuard::Withheld(existing.clone())
                                }
                                _ => MoveIdentityGuard::Unavailable,
                            }
                        }
                    };
                    next_identity_guards.insert(uri.clone(), guard);
                    unresolved.push(uri.clone());
                }
                None => {
                    // A missing worker result cannot prove success or provide
                    // a safe identity for a future MOVE.
                    next_identity_guards.insert(uri.clone(), MoveIdentityGuard::Unavailable);
                    unresolved.push(uri.clone());
                }
            }
        }
        if unresolved.is_empty() {
            // Keep a consumed internal tombstone for the published text. If
            // we interpreted it as foreign URI data on the next paste, a
            // completed cut could silently turn back into a copy.
            self.entries = Some(Clipboard {
                uris: Vec::new(),
                cut: true,
            });
            self.cut_identity_guards.clear();
            self.cut_in_flight = None;
            CutFinish::Consumed
        } else {
            self.entries
                .as_mut()
                .expect("entries checked above")
                .uris = unresolved;
            self.cut_identity_guards = next_identity_guards;
            self.cut_in_flight = None;
            CutFinish::Partial
        }
    }

    /// Synchronous paste hint for menu enablement: true only while internal
    /// entries are still valid (never stale: external changes clear them).
    /// Foreign-app content needs a format peek by the caller. No I/O here,
    /// never blocks.
    pub(crate) fn has_usable_entries(&self) -> bool {
        self.entries
            .as_ref()
            .is_some_and(|entries| !entries.uris.is_empty())
    }

    #[cfg(test)]
    fn resolve(&self, text: Option<&str>, read_generation: u64) -> PasteDecision {
        // Existing GDK-free tracker tests model the provider Kito published.
        self.resolve_with_provider_snapshot(text, read_generation, Some(1), Some(1), Some(1))
    }

    fn resolve_with_provider_snapshot(
        &self,
        text: Option<&str>,
        read_generation: u64,
        provider_at_read: Option<usize>,
        provider_current: Option<usize>,
        our_provider: Option<usize>,
    ) -> PasteDecision {
        if read_generation != self.generation || provider_at_read != provider_current {
            return PasteDecision::Stale;
        }
        match (&self.entries, &self.published_text, text) {
            (Some(entries), Some(published), Some(current))
                if clipboard_provider_is_owned(provider_current, our_provider)
                    && current == published =>
            {
                PasteDecision::Internal {
                    uris: entries.uris.clone(),
                    cut: entries.cut,
                }
            }
            (_, _, Some(current)) => {
                let uris = parse_external_uris(current);
                if uris.is_empty() {
                    PasteDecision::Unsupported
                } else {
                    PasteDecision::External { uris }
                }
            }
            _ => PasteDecision::Unsupported,
        }
    }
}

/// A worker callback owns this guard after a cut was marked in flight. If
/// delivery fails or a completion callback panics before applying the result,
/// dropping it releases the mark without consuming clipboard entries.
struct CutCompletionGuard {
    clipboard: Rc<RefCell<ClipTracker>>,
    generation: Option<u64>,
    dispatched_uris: Vec<String>,
    worker_started: Arc<AtomicBool>,
    worker_failure_unknown: Cell<Option<bool>>,
    completed: bool,
}

impl CutCompletionGuard {
    fn new(
        clipboard: Rc<RefCell<ClipTracker>>,
        generation: Option<u64>,
        dispatched_uris: Vec<String>,
        worker_started: Arc<AtomicBool>,
    ) -> Self {
        Self {
            clipboard,
            generation,
            dispatched_uris,
            worker_started,
            worker_failure_unknown: Cell::new(None),
            completed: false,
        }
    }

    fn worker_failed(&self, failure: &crate::operation_queue::WorkerFailure) {
        self.worker_failure_unknown
            .set(Some(worker_failure_has_unknown_outcome(failure)));
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for CutCompletionGuard {
    fn drop(&mut self) {
        if !self.completed {
            if let Some(generation) = self.generation {
                if let Ok(mut clipboard) = self.clipboard.try_borrow_mut() {
                    let outcome_unknown = self
                        .worker_failure_unknown
                        .get()
                        .unwrap_or_else(|| self.worker_started.load(Ordering::Acquire));
                    clipboard.abandon_cut_dispatch(
                        generation,
                        &self.dispatched_uris,
                        outcome_unknown,
                    );
                }
            }
        }
    }
}

/// `file://` lines from foreign clipboard text (interop with other
/// apps). Anything else is ignored by the caller.
fn parse_external_uris(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("file://"))
        .map(String::from)
        .collect()
}

#[derive(Clone)]
pub struct Ctx {
    pub window: adw::ApplicationWindow,
    pub manager: Rc<TabManager>,
    pub toast: adw::ToastOverlay,
    pub(crate) clipboard: Rc<RefCell<ClipTracker>>,
    pub(crate) operation_queue: Rc<OperationQueue>,
    /// F08: operation center shared by every window of the process.
    pub(crate) job_center: Rc<JobCenter>,
    pub preferences: Rc<PreferenceStore>,
    /// Shows the path entry (Ctrl+L). Set by main.
    pub focus_path: Rc<dyn Fn()>,
}

/// Whether an already-loaded current location is a bookmarkable GIO target.
/// Tabs reach this callback only after a successful load, so probing remote
/// filesystems synchronously here would add latency without improving safety.
pub(crate) fn current_location_bookmarkable(uri: &str) -> bool {
    !uri.is_empty()
        && !gio::File::for_uri(uri).equal(&gio::File::for_path(glib::home_dir()))
        && !matches!(
            uri,
            kito_core::FAVORITES_URI
                | kito_core::ADD_BOOKMARK_URI
                | kito_core::TRASH_URI
                | "network:///"
        )
        && !uri.starts_with("recent:")
        && !uri.starts_with("search:")
}

impl Ctx {
    /// GDK `changed` notification: our own publish keeps the state,
    /// anything else invalidates it. Ownership is the content provider
    /// identity (not text equality).
    pub(crate) fn on_clipboard_changed(&self) {
        let ours = (|| {
            let current = gdk::Display::default()?.clipboard().content()?;
            let owned = self.clipboard.borrow().our_provider.clone()?;
            Some(clipboard_provider_is_owned(
                Some(current.as_ptr() as usize),
                Some(owned.as_ptr() as usize),
            ))
        })()
        .unwrap_or(false);
        let mut tracker = self.clipboard.borrow_mut();
        if ours {
            tracker.on_own_changed();
        } else {
            tracker.on_external_changed();
        }
    }

    pub fn toast(&self, msg: &str) {
        self.toast.add_toast(adw::Toast::new(msg));
    }

    /// F08: submits a mutation as a registered, cancellable job. The job
    /// captures sources/destination at enqueue time; the worker propagates
    /// the job's GIO `Cancellable` to every cancellable core call and the
    /// completion records per-element outcomes. Sources/destination and
    /// later tab or selection changes stay independent (captured here),
    /// and closing a tab or window never invalidates the running worker.
    /// `job_worker` receives the cancellable. `done` keeps the same
    /// engine contract as `OperationQueue::submit`.
    fn submit_job<F, R>(
        self: &Rc<Self>,
        operation: &'static str,
        sources: Vec<String>,
        destination: Option<String>,
        total_known: Option<u64>,
        job_worker: F,
        done: impl FnOnce(
            Rc<Self>,
            Arc<crate::job_center::Job>,
            Result<R, crate::operation_queue::WorkerFailure>,
        ) + 'static,
    ) where
        F: FnOnce(&gio::Cancellable) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (job, cancellable) = self
            .job_center
            .begin_job(operation, sources.clone(), destination.clone());
        let job_center = self.job_center.clone();
        let this = self.clone();
        let worker_job = job.clone();
        let start_job = job.clone();
        let queued_cancellable = cancellable.clone();
        let cancelled_job = job.clone();
        let cancelled_center = job_center.clone();
        self.operation_queue.clone().submit_cancellable_with_start(
            move || {
                let _job_guard = CurrentJobGuard::install(worker_job.clone());
                job_worker(&cancellable)
            },
            move || queued_cancellable.is_cancelled(),
            move || start_job.start(total_known),
            move |outcome| {
                let _completion_guard = JobCompletionGuard::new(job_center.clone(), job.clone());
                let cancelled_before_start = matches!(
                    outcome.as_ref(),
                    Err(crate::operation_queue::WorkerFailure::CancelledBeforeStart)
                );
                if cancelled_before_start {
                    cancelled_center.finish_items(
                        &cancelled_job,
                        cancelled_job
                            .sources
                            .iter()
                            .map(|source| {
                                (
                                    source.clone(),
                                    JobItemState::Cancelled(crate::l10n::tr(
                                        "jobs-cancelled-before-start",
                                    )),
                                )
                            })
                            .collect(),
                    );
                }
                if outcome.is_err() {
                    if !cancelled_before_start {
                        job_center.finish_worker_failure(&job);
                    }
                }
                done(this, job, outcome);
            },
        );
    }

    /// Starts a drag-and-drop transfer using the action selected by GDK.
    /// This path uses the operation engine directly and never changes the clipboard.
    pub fn transfer_uris(
        &self,
        uris: Vec<String>,
        destination: String,
        action: crate::dnd::TransferAction,
        internal: bool,
        drop: Option<gdk::Drop>,
    ) {
        let completion = drop.map(|drop| DropCompletion {
            drop,
            action,
            finished: Cell::new(false),
        });
        if destination == kito_core::ADD_BOOKMARK_URI {
            let mut results = Vec::new();
            let mut added = false;
            for uri in uris {
                let name = gio::File::for_uri(&uri)
                    .basename()
                    .map(|name| name.to_string_lossy().to_string())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| kito_core::uri_to_display(&uri));
                let result = if matches!(
                    action,
                    crate::dnd::TransferAction::Copy | crate::dnd::TransferAction::Link
                ) {
                    kito_core::bookmarks::pin(&name, &uri)
                } else {
                    Err(glib::Error::new(
                        gio::IOErrorEnum::InvalidArgument,
                        "Bookmark drop requires COPY or LINK",
                    ))
                };
                added |= result.is_ok();
                if let Err(error) = &result {
                    self.toast(&error.to_string());
                }
                results.push(OperationItemResult::from_result(
                    "bookmark", uri, None, result,
                ));
            }
            if let Some(completion) = completion {
                completion.finish(&results);
            }
            if added {
                self.preferences.notify_bookmarks_changed();
                self.toast(&crate::l10n::tr("side-bookmark-added"));
            }
            return;
        }
        if destination == kito_core::FAVORITES_URI {
            if matches!(
                action,
                crate::dnd::TransferAction::Copy | crate::dnd::TransferAction::Link
            ) {
                let mut results = Vec::new();
                for uri in uris {
                    match self.preferences.add_favorite(&uri) {
                        Ok(()) => results.push(OperationItemResult::from_result(
                            "favorite",
                            uri,
                            None,
                            Ok(()),
                        )),
                        Err(error) => results.push(OperationItemResult::from_result(
                            "favorite",
                            uri,
                            None,
                            Err(glib::Error::new(
                                gio::IOErrorEnum::Failed,
                                &error.to_string(),
                            )),
                        )),
                    }
                }
                if let Some(completion) = completion {
                    completion.finish(&results);
                }
            } else {
                self.toast(&crate::l10n::tr("favorites-not-a-folder"));
                if let Some(completion) = completion {
                    completion.finish(&[]);
                }
            }
            return;
        }
        if uris.is_empty() {
            if let Some(completion) = completion {
                completion.finish(&[]);
            }
            return;
        }
        match (destination.as_str(), action) {
            (kito_core::TRASH_URI, crate::dnd::TransferAction::Move) => {
                self.trash_uris(uris, kito_core::TRASH_URI.to_string(), completion, false);
            }
            (kito_core::TRASH_URI, crate::dnd::TransferAction::Copy) => {
                self.toast(&crate::l10n::tr("drop-trash-copy-unsupported"));
                if let Some(completion) = completion {
                    completion.finish(&[]);
                }
            }
            (_, action) => {
                self.paste_uris_with_drop(
                    uris,
                    action == crate::dnd::TransferAction::Move,
                    destination,
                    None,
                    completion,
                    internal && action == crate::dnd::TransferAction::Move,
                    HashMap::new(),
                );
            }
        }
    }

    /// A completed external MOVE asks the source to delete its data. Route
    /// that deletion to the recoverable trash and retain the original URIs.
    pub fn trash_external_move(&self, uris: Vec<String>) {
        if uris.is_empty() {
            return;
        }
        let folder = gio::File::for_uri(&uris[0])
            .parent()
            .map(|parent| parent.uri().to_string())
            .unwrap_or_default();
        self.trash_uris(uris, folder, None, true);
    }

    /// Ctrl+L: switches to path writing mode.
    pub fn focus_path(&self) {
        (self.focus_path)();
    }

    fn error_dialog(&self, heading: &str, body: String) {
        let dialog = adw::AlertDialog::builder()
            .heading(heading)
            .body(body)
            .build();
        dialog.add_response("ok", &crate::l10n::tr("dialog-ok"));
        dialog.present(Some(&self.window));
    }

    fn operation_worker_failed(&self, failure: &crate::operation_queue::WorkerFailure) {
        self.toast(&crate::l10n::tr(if matches!(
            failure,
            crate::operation_queue::WorkerFailure::QueueFull
        ) {
            "operation-queue-full"
        } else {
            "operation-worker-failed"
        }));
    }

    /// Reports one batch once, with expandable per-item details. A retry
    /// receives only failed entries; successful entries are never replayed.
    fn report_operation(
        &self,
        operation: &'static str,
        results: Vec<OperationItemResult>,
        retry: Option<RetryHandler>,
    ) {
        if results.is_empty() {
            return;
        }
        // Item operations act on real targets; refresh virtual Favorites so
        // renamed, moved, deleted or restored targets expose current metadata.
        self.manager.reload_if_current(kito_core::FAVORITES_URI);
        let succeeded = results
            .iter()
            .filter(|result| matches!(result.state, ItemState::Succeeded))
            .count();
        let failed = results
            .iter()
            .filter(|result| matches!(result.state, ItemState::Failed(_)))
            .count();
        let cancelled = results
            .iter()
            .filter(|result| matches!(result.state, ItemState::Cancelled(_)))
            .count();
        let total = results.len();
        let args = operation_summary_args(
            crate::l10n::tr(operation),
            succeeded,
            failed,
            cancelled,
            total,
        );
        let summary_key = if failed == 0 && cancelled == 0 {
            "operation-result-success"
        } else if succeeded == 0 && cancelled == 0 {
            "operation-result-failed"
        } else if cancelled > 0 {
            "operation-result-cancelled"
        } else {
            "operation-result-partial"
        };
        let summary = crate::l10n::tr_with(summary_key, &args);
        if failed == 0 && cancelled == 0 {
            self.toast(&summary);
            return;
        }

        let details = results
            .iter()
            .map(|result| {
                let mut lines = vec![crate::l10n::tr_with_one(
                    "operation-detail-operation",
                    "operation",
                    &crate::l10n::tr(result.operation),
                )];
                lines.push(crate::l10n::tr_with_one(
                    "operation-detail-source",
                    "source",
                    &kito_core::uri_to_display(&result.source),
                ));
                if let Some(destination) = &result.destination {
                    lines.push(crate::l10n::tr_with_one(
                        "operation-detail-destination",
                        "destination",
                        &kito_core::uri_to_display(destination),
                    ));
                }
                match &result.state {
                    ItemState::Succeeded => lines.push(crate::l10n::tr("operation-item-succeeded")),
                    ItemState::Failed(error) => lines.push(crate::l10n::tr_with_one(
                        "operation-item-failed",
                        "error",
                        error,
                    )),
                    ItemState::Cancelled(error) => lines.push(crate::l10n::tr_with_one(
                        "operation-item-cancelled",
                        "error",
                        error,
                    )),
                }
                lines.join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let dialog = adw::Dialog::builder()
            .title(crate::l10n::tr("operation-result-title"))
            .content_width(560)
            .build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_start(20)
            .margin_end(20)
            .margin_top(20)
            .margin_bottom(20)
            .build();
        content.append(
            &gtk::Label::builder()
                .label(summary)
                .halign(gtk::Align::Start)
                .wrap(true)
                .build(),
        );
        let details_label = gtk::Label::builder()
            .label(details)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .selectable(true)
            .wrap(true)
            .build();
        let details_scroll = gtk::ScrolledWindow::builder()
            .min_content_height(120)
            .max_content_height(280)
            .child(&details_label)
            .build();
        let expander = gtk::Expander::builder()
            .label(crate::l10n::tr("operation-details"))
            .child(&details_scroll)
            .build();
        content.append(&expander);

        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::End)
            .build();
        let close = gtk::Button::builder()
            .label(crate::l10n::tr("operation-close"))
            .build();
        buttons.append(&close);
        if let Some(retry) = retry {
            let failed_items: Vec<_> = results
                .into_iter()
                .filter(OperationItemResult::needs_retry)
                .collect();
            if !failed_items.is_empty() {
                let retry_button = gtk::Button::builder()
                    .label(crate::l10n::tr("operation-retry-failed"))
                    .css_classes(["suggested-action"])
                    .build();
                retry_button.connect_clicked({
                    let dialog = dialog.clone();
                    move |_| {
                        dialog.close();
                        retry(failed_items.clone());
                    }
                });
                buttons.append(&retry_button);
            }
        }
        content.append(&buttons);
        close.connect_clicked({
            let dialog = dialog.clone();
            move |_| {
                dialog.close();
            }
        });
        dialog.set_child(Some(&content));
        dialog.present(Some(&self.window));
    }

    /// Runs `job` in a thread; `done` runs on the main loop.
    /// `done` touches GTK widgets, so it stays on the main thread: the worker
    /// sends the result through a bounded async channel and a main-loop future
    /// delivers it. The main loop sleeps until the result arrives (no polling,
    /// no sleeps on the UI thread). Dropping without delivery (worker panic)
    /// simply runs nothing, as before.
    fn run_in_thread<F, R>(job: F, done: impl FnOnce(R) + 'static)
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = async_channel::bounded::<R>(1);
        std::thread::spawn(move || {
            let result = job();
            let _ = tx.send_blocking(result);
        });
        glib::spawn_future_local(async move {
            if let Ok(result) = rx.recv().await {
                done(result);
            }
        });
    }

    pub fn open_selected(&self) {
        let Some(tab) = self.manager.selected() else {
            return;
        };
        let objs = tab.selected_objects();
        if objs.is_empty() {
            self.toast(&crate::l10n::tr("toast-nothing"));
            return;
        }
        if objs.len() == 1 {
            tab.activate_entry(&objs[0].uri(), objs[0].is_dir());
            return;
        }
        for obj in objs {
            if obj.is_dir() {
                self.manager.open_tab_from_context(&obj.uri());
            } else {
                tab.activate_entry(&obj.uri(), false);
            }
        }
    }

    pub fn open_selected_in_new_tabs(&self) {
        let selected = self.manager.selected_objects();
        if selected.is_empty() || selected.iter().any(|obj| !obj.is_dir()) {
            self.toast(&crate::l10n::tr("open-folders-only"));
            return;
        }
        let uris: Vec<String> = selected.iter().map(|obj| obj.uri()).collect();
        self.manager.open_folders_in_tabs_from_context(&uris);
    }

    pub fn open_selected_in_new_window(&self) {
        let selected = self.manager.selected_objects();
        if selected.is_empty() || selected.iter().any(|obj| !obj.is_dir()) {
            self.toast(&crate::l10n::tr("open-folders-only"));
            return;
        }
        let uris: Vec<String> = selected.iter().map(|obj| obj.uri()).collect();
        self.manager.open_folders_in_new_window(&uris);
    }

    pub fn go_up(&self) {
        if self.manager.selected_uri().as_deref() == Some(kito_core::FAVORITES_URI) {
            self.manager.go_back();
            return;
        }
        if let Some(uri) = self.manager.selected_uri() {
            if let Some(parent) = gio::File::for_uri(&uri).parent() {
                self.manager.load_selected(&parent.uri());
            }
        }
    }

    fn change_zoom(&self, zoom: u8) {
        if let Err(error) = self.preferences.set_icon_zoom(zoom) {
            self.toast(&error.to_string());
        }
        self.manager.refresh_display_preferences();
    }

    pub fn zoom_in(&self) {
        let zoom = self.preferences.snapshot().icon_zoom;
        self.change_zoom(zoom.saturating_add(crate::preferences::model::ICON_ZOOM_STEP));
    }

    pub fn zoom_out(&self) {
        let zoom = self.preferences.snapshot().icon_zoom;
        self.change_zoom(zoom.saturating_sub(crate::preferences::model::ICON_ZOOM_STEP));
    }

    pub fn zoom_reset(&self) {
        self.change_zoom(100);
    }

    /// Ctrl+C / Ctrl+X: internal state plus a system publish as a
    /// newline-separated URI list. The provider wraps a plain string, so
    /// other apps read the URIs from its text; Kito does not advertise
    /// `text/uri-list` and never claims a format it does not publish.
    /// Success is reported only when GDK accepts the provider.
    pub fn copy_selected(&self, cut: bool) {
        let uris: Vec<String> = self
            .manager
            .selected_objects()
            .iter()
            .map(|o| o.uri())
            .collect();
        if uris.is_empty() {
            self.toast(&crate::l10n::tr("toast-nothing"));
            return;
        }
        // Publish internally first, then to the system clipboard (plain
        // text stays readable by other apps, as before).
        let text = self.clipboard.borrow_mut().publish(Clipboard { uris, cut });
        let mut published = false;
        if let Some(display) = gdk::Display::default() {
            let clipboard = display.clipboard();
            // A GValue string provider has the same interoperable text
            // serializers as `Clipboard::set_text`, while letting us record
            // its identity before GDK emits the changed notification.
            let provider = gdk::ContentProvider::for_value(&text.to_value());
            self.clipboard.borrow_mut().our_provider = Some(provider.clone());
            published = clipboard.set_content(Some(&provider)).is_ok();
            if !published {
                // Nothing reached the clipboard: drop the internal state as
                // well, so it cannot describe content that was never
                // published. The foreign clipboard stays untouched.
                let mut tracker = self.clipboard.borrow_mut();
                tracker.on_external_changed();
                tracker.our_provider = None;
            }
        } else {
            // No system clipboard to publish to: no owned provider, and
            // nothing to declare success about.
            self.clipboard.borrow_mut().on_external_changed();
        }
        self.toast(&crate::l10n::tr(publish_toast_key(published, cut)));
    }

    pub fn paste(&self) {
        let Some(dest) = self.manager.selected_uri() else {
            return;
        };
        self.paste_at(&dest);
    }

    /// Pastes into an explicit folder: same guards as [`Self::paste`], but
    /// the destination is fixed (background menu) instead of following the
    /// selected tab.
    pub fn paste_at(&self, dest: &str) {
        let dest = dest.to_string();
        let Some(display) = gdk::Display::default() else {
            self.toast(&crate::l10n::tr("clip-empty"));
            return;
        };
        // The system clipboard decides: read it first, then resolve
        // against the internal state captured now. A change in between
        // retires this read instead of pasting stale entries.
        let generation = self.clipboard.borrow().generation;
        let clipboard = display.clipboard();
        let provider_at_read = clipboard.content();
        let provider_at_read_id = content_provider_identity(provider_at_read.as_ref());
        let our_provider_id =
            content_provider_identity(self.clipboard.borrow().our_provider.as_ref());
        // Retain the provider across the async read so its pointer identity
        // cannot be recycled before the callback compares current content.
        let provider_snapshot = provider_at_read;
        let clipboard_for_callback = clipboard.clone();
        let this = self.clone();
        clipboard.read_text_async(gio::Cancellable::NONE, move |result| {
            let _provider_snapshot = provider_snapshot;
            let provider_current = clipboard_for_callback.content();
            let provider_current_id = content_provider_identity(provider_current.as_ref());
            let text = result.ok().flatten().map(|s| s.to_string());
            // The decision is bound first: the tracker borrow ends with
            // this statement, so the action below can start (or refuse)
            // a move without holding a shared borrow.
            let decision = this.clipboard.borrow().resolve_with_provider_snapshot(
                text.as_deref(),
                generation,
                provider_at_read_id,
                provider_current_id,
                our_provider_id,
            );
            match paste_action(&this.clipboard, &decision, text.as_deref()) {
                PasteAction::Explain(key) => this.toast(&crate::l10n::tr(key)),
                PasteAction::Copy(uris) => this.paste_uris(uris, false, dest, None),
                // Cuts move asynchronously: the tracker keeps the state
                // until completion and refuses a concurrent duplicate.
                PasteAction::Move(dispatch) => this.paste_uris_with_identity_guards(
                    dispatch.uris,
                    true,
                    dest,
                    Some(dispatch.generation),
                    dispatch.identity_guards,
                ),
                PasteAction::MoveRefused => {
                    this.toast(&crate::l10n::tr("clip-moving"));
                }
            }
        });
    }

    /// Copies (or moves, for a cut) `uris` into `dest`. `cut_op` is the
    /// dispatch generation of a cut move, applied at completion.
    fn paste_uris(&self, uris: Vec<String>, cut: bool, dest: String, cut_op: Option<u64>) {
        self.paste_uris_with_identity_guards(uris, cut, dest, cut_op, HashMap::new());
    }

    fn paste_uris_with_identity_guards(
        &self,
        uris: Vec<String>,
        cut: bool,
        dest: String,
        cut_op: Option<u64>,
        identity_guards: HashMap<String, MoveIdentityGuard>,
    ) {
        self.paste_uris_with_drop(
            uris,
            cut,
            dest,
            cut_op,
            None,
            cut,
            identity_guards,
        );
    }

    fn paste_uris_with_drop(
        &self,
        uris: Vec<String>,
        cut: bool,
        dest: String,
        cut_op: Option<u64>,
        drop_completion: Option<DropCompletion>,
        move_with_backend: bool,
        identity_guards: HashMap<String, MoveIdentityGuard>,
    ) {
        if dest == kito_core::FAVORITES_URI {
            self.toast(&crate::l10n::tr("favorites-not-a-folder"));
            if let Some(op_generation) = cut_op {
                self.clipboard.borrow_mut().cancel_cut(op_generation);
            }
            if let Some(completion) = drop_completion {
                completion.finish(&[]);
            }
            return;
        }
        if uris.is_empty() {
            if let Some(completion) = drop_completion {
                completion.finish(&[]);
            } else {
                self.toast(&crate::l10n::tr("clip-empty"));
            }
            return;
        }
        let transfer = CapturedTransfer {
            uris,
            destination: dest,
            cut,
            move_with_backend: cut && move_with_backend,
            identity_guards,
        };
        let result_operation = if transfer.cut {
            "operation-move"
        } else {
            "operation-copy"
        };
        let is_cut = transfer.cut;
        let retry_dest = transfer.destination.clone();
        // F08: copy keeps the source by definition. A backend move can be a
        // copy+delete across filesystems, so failure may leave both source
        // and a partial destination; core reports that output and disables
        // blind retry without attempting unsafe cleanup.
        let this = Rc::new(self.clone());
        let this_rc = this.clone();
        let worker_started = Arc::new(AtomicBool::new(false));
        let worker_started_by_job = worker_started.clone();
        let dispatched_uris = if cut_op.is_some() {
            transfer.uris.clone()
        } else {
            Vec::new()
        };
        // Capture before submission: this guard also runs if the queue's
        // completion wrapper panics before invoking the UI `done` callback.
        let completion_guard = CutCompletionGuard::new(
            this.clipboard.clone(),
            cut_op,
            dispatched_uris,
            worker_started,
        );
        Ctx::submit_job(
            &this_rc,
            result_operation,
            transfer.uris.clone(),
            Some(transfer.destination.clone()),
            Some(transfer.uris.len() as u64),
            move |cancellable| {
                worker_started_by_job.store(true, Ordering::Release);
                let mut results = Vec::with_capacity(transfer.uris.len());
                let mut undo = Vec::new();
                for uri in &transfer.uris {
                    let outcome: OperationItemResult = if transfer.move_with_backend {
                        match move_identity_attempt(uri, &transfer.identity_guards, || {
                            kito_core::file_identity_cancellable(uri, cancellable).ok()
                        }) {
                            MoveIdentityAttempt::Withheld(marker) => {
                                OperationItemResult::from_move_result(
                                    result_operation,
                                    uri.clone(),
                                    transfer.destination.clone(),
                                    None,
                                    Err(glib::Error::new(gio::IOErrorEnum::Failed, &marker)),
                                )
                            }
                            MoveIdentityAttempt::Authorized(retry_identity) => {
                                match kito_core::move_to_exact_result_cancellable_if_identity(
                                    uri,
                                    &transfer.destination,
                                    cancellable,
                                    retry_identity.as_ref(),
                                ) {
                                    Ok(target) => {
                                        let identity = unchanged_undo_identity(
                                            retry_identity.clone(),
                                            kito_core::file_identity(&target).ok(),
                                        );
                                        if let Some(identity) = identity {
                                            undo.push(UndoRecord::Move {
                                                source: uri.clone(),
                                                target: target.clone(),
                                                identity,
                                            });
                                        }
                                        OperationItemResult::from_move_result(
                                            result_operation,
                                            uri.clone(),
                                            transfer.destination.clone(),
                                            retry_identity.clone(),
                                            Ok(target),
                                        )
                                    }
                                    Err(error) => OperationItemResult::from_move_result(
                                        result_operation,
                                        uri.clone(),
                                        transfer.destination.clone(),
                                        retry_identity,
                                        Err(error),
                                    ),
                                }
                            }
                        }
                    } else {
                        let report_bytes = gio::File::for_uri(uri).query_file_type(
                            gio::FileQueryInfoFlags::NONE,
                            Some(cancellable),
                        ) != gio::FileType::Directory;
                        match kito_core::copy_to_cancellable(
                            uri,
                            &transfer.destination,
                            cancellable,
                            |bytes_current, bytes_current_total| {
                                if report_bytes {
                                    job_report(move |job| {
                                        job.report_current_file(
                                            bytes_current as u64,
                                            bytes_current_total,
                                        );
                                    });
                                }
                            },
                        ) {
                            Ok(()) => OperationItemResult::from_destination_result(
                                result_operation,
                                uri.clone(),
                                transfer.destination.clone(),
                                Ok(transfer.destination.clone()),
                            ),
                            Err(error) => OperationItemResult::from_result(
                                result_operation,
                                uri.clone(),
                                Some(transfer.destination.clone()),
                                Err(error),
                            ),
                        }
                    };
                    // Every really attempted element (success or terminal
                    // non-cancelled failure) advances the determinate bar.
                    // A cancelled item keeps the batch at its last real value.
                    if !matches!(outcome.state, ItemState::Cancelled(_)) {
                        job_report(move |job| job.record_attempt());
                    }
                    results.push(outcome);
                }
                (results, undo)
            },
            move |this, finished_job, outcome| {
                let mut cut_guard = completion_guard;
                let (results, undo): (Vec<OperationItemResult>, Vec<UndoRecord>) = match outcome {
                    Ok(result) => result,
                    Err(failure) => {
                        cut_guard.worker_failed(&failure);
                        drop(cut_guard);
                        this.manager.reload_if_current(&retry_dest);
                        if let Some(completion) = &drop_completion {
                            completion.finish(&[]);
                        }
                        if !is_queued_cancellation(&failure) {
                            this.operation_worker_failed(&failure);
                        }
                        return;
                    }
                };
                for record in undo {
                    this.operation_queue.push_undo(record);
                }
                // F08: registry classification for the Operations view.
                this.job_center.finish_items(
                    &finished_job,
                    results
                        .iter()
                        .map(|result| (result.source.clone(), result.job_state()))
                        .collect(),
                );
                let cut_finish = cut_op.map(|op_generation| {
                    this.clipboard
                        .borrow_mut()
                        .finish_cut(op_generation, &results)
                });
                cut_guard.complete();
                this.manager.reload_if_current(&retry_dest);
                if let Some(completion) = &drop_completion {
                    completion.finish(&results);
                }
                let retry: Option<RetryHandler> = if is_cut {
                    match (cut_op, cut_finish) {
                        (Some(op_generation), Some(CutFinish::Partial)) => {
                            let this = this.clone();
                            Some(Rc::new(move |failed: Vec<OperationItemResult>| {
                                let (uris, _) = retry_transfer_capture(failed);
                                let next_op = this
                                    .clipboard
                                    .borrow_mut()
                                    .begin_cut_for(op_generation, &uris);
                                if let Some(dispatch) = next_op {
                                    this.paste_uris_with_identity_guards(
                                        dispatch.uris,
                                        true,
                                        retry_dest.clone(),
                                        Some(dispatch.generation),
                                        dispatch.identity_guards,
                                    );
                                } else {
                                    this.toast(&crate::l10n::tr("clip-changed"));
                                }
                            }))
                        }
                        (None, _) => {
                            let this = this.clone();
                            Some(Rc::new(move |failed: Vec<OperationItemResult>| {
                                let (uris, retry_identities) =
                                    retry_transfer_capture(failed);
                                this.paste_uris_with_identity_guards(
                                    uris,
                                    true,
                                    retry_dest.clone(),
                                    None,
                                    retry_identities,
                                );
                            }))
                        }
                        _ => None,
                    }
                } else {
                    let this = this.clone();
                    Some(Rc::new(move |failed: Vec<OperationItemResult>| {
                        let uris = failed.into_iter().map(|item| item.source).collect();
                        this.paste_uris(uris, false, retry_dest.clone(), None);
                    }))
                };
                this.report_operation(result_operation, results, retry);
            },
        );
    }

    pub fn trash_selected(&self) {
        let uris: Vec<String> = self
            .manager
            .selected_objects()
            .iter()
            .map(|o| o.uri())
            .collect();
        if uris.is_empty() {
            self.toast(&crate::l10n::tr("toast-nothing"));
            return;
        }
        let folder = self.manager.selected_uri().unwrap_or_default();
        self.trash_uris(uris, folder, None, false);
    }

    fn trash_uris(
        &self,
        uris: Vec<String>,
        folder: String,
        drop_completion: Option<DropCompletion>,
        external_move: bool,
    ) {
        let this = Rc::new(self.clone());
        let retry_folder2 = folder.clone();
        Ctx::submit_job(
            &this,
            "operation-trash",
            uris.clone(),
            Some(kito_core::TRASH_URI.to_string()),
            Some(uris.len() as u64),
            move |cancellable| {
                let mut results = Vec::new();
                let mut undo = Vec::new();
                for uri in uris {
                    let result_count = results.len();
                    match kito_core::trash_with_identity_cancellable(&uri, cancellable) {
                        Ok(target) => {
                            if let Some((target, identity)) = target {
                                undo.push(UndoRecord::Trash {
                                    source: uri.clone(),
                                    target,
                                    identity,
                                });
                            }
                            results.push(OperationItemResult::from_result(
                                "operation-trash",
                                uri,
                                Some(kito_core::TRASH_URI.to_string()),
                                Ok(()),
                            ));
                        }
                        Err(error) => {
                            let result = if external_move {
                                let check_uri = uri.clone();
                                external_move_trash_result(uri, Err(error), || {
                                    external_move_source_check(&check_uri, cancellable)
                                })
                            } else {
                                Some(OperationItemResult::from_result(
                                    "operation-trash",
                                    uri,
                                    Some(kito_core::TRASH_URI.to_string()),
                                    Err(error),
                                ))
                            };
                            if let Some(result) = result {
                                results.push(result);
                            }
                        }
                    }
                    if results.len() > result_count
                        && results.last().is_some_and(|result| !matches!(result.state, ItemState::Cancelled(_)))
                        || (results.len() == result_count
                            && external_move
                            && !cancellable.is_cancelled())
                    {
                        job_report(move |job| job.record_attempt());
                    }
                }
                (results, undo)
            },
            move |this, finished_job, outcome| {
                let (results, undo) = match outcome {
                    Ok(result) => result,
                    Err(failure) => {
                        this.manager.reload_if_current(&folder);
                        if let Some(completion) = &drop_completion {
                            completion.finish(&[]);
                        }
                        if !is_queued_cancellation(&failure) {
                            this.operation_worker_failed(&failure);
                        }
                        return;
                    }
                };
                for record in undo {
                    this.operation_queue.push_undo(record);
                }
                this.job_center.finish_items(
                    &finished_job,
                    results
                        .iter()
                        .map(|result| (result.source.clone(), result.job_state()))
                        .collect(),
                );
                this.manager.reload_if_current(&folder);
                if let Some(completion) = &drop_completion {
                    completion.finish(&results);
                }
                if external_move && results.is_empty() {
                    this.manager
                        .reload_if_current(kito_core::FAVORITES_URI);
                }
                let retry_this = this.clone();
                let retry_folder = retry_folder2.clone();
                let retry: RetryHandler = Rc::new(move |failed| {
                    let uris = failed.into_iter().map(|item| item.source).collect();
                    retry_this.trash_uris(uris, retry_folder.clone(), None, false);
                });
                this.report_operation("operation-trash", results, Some(retry));
            },
        );
    }

    /// From the trash: restores the selected entries to the original location.
    pub fn restore_selected(&self) {
        let uris: Vec<String> = self
            .manager
            .selected_objects()
            .iter()
            .map(|o| o.uri())
            .collect();
        if uris.is_empty() {
            self.toast(&crate::l10n::tr("toast-nothing"));
            return;
        }
        let trash_uri = self.manager.selected_uri().unwrap_or_default();
        self.restore_uris(uris, trash_uri);
    }

    fn restore_uris(&self, uris: Vec<String>, trash_uri: String) {
        let this = Rc::new(self.clone());
        let retry_trash_uri = trash_uri.clone();
        Ctx::submit_job(
            &this,
            "operation-restore",
            uris.clone(),
            None,
            Some(uris.len() as u64),
            move |cancellable| {
                uris.into_iter()
                    .map(|uri| match kito_core::restore_cancellable(&uri, cancellable) {
                        Ok(destination) => {
                            job_report(move |job| job.record_attempt());
                            OperationItemResult {
                                operation: "operation-restore",
                                source: uri,
                                destination: Some(destination),
                                state: ItemState::Succeeded,
                                retry_identity: None,
                            }
                        }
                        Err(error) => {
                            if error.matches(gio::IOErrorEnum::Cancelled) {
                                OperationItemResult::from_result(
                                    "operation-restore",
                                    uri,
                                    None,
                                    Err(error),
                                )
                            } else {
                                job_report(move |job| job.record_attempt());
                                OperationItemResult::from_result(
                                    "operation-restore",
                                    uri,
                                    None,
                                    Err(error),
                                )
                            }
                        }
                    })
                    .collect::<Vec<_>>()
            },
            move |this, finished_job, outcome| {
                let results = match outcome {
                    Ok(results) => results,
                    Err(failure) => {
                        this.manager.reload_if_current(&trash_uri);
                        if !is_queued_cancellation(&failure) {
                            this.operation_worker_failed(&failure);
                        }
                        return;
                    }
                };
                this.job_center.finish_items(
                    &finished_job,
                    results
                        .iter()
                        .map(|result| (result.source.clone(), result.job_state()))
                        .collect(),
                );
                this.manager.reload_if_current(&trash_uri);
                let retry_this = this.clone();
                let retry: RetryHandler = Rc::new(move |failed| {
                    let uris = failed.into_iter().map(|item| item.source).collect();
                    retry_this.restore_uris(uris, retry_trash_uri.clone());
                });
                this.report_operation("operation-restore", results, Some(retry));
            },
        );
    }

    /// Empties the trash: confirm, then delete in background.
    pub fn empty_trash(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading(crate::l10n::tr("trash-empty-title"))
            .body(crate::l10n::tr("trash-empty-body"))
            .build();
        dialog.add_response("cancel", &crate::l10n::tr("dialog-cancel"));
        dialog.add_response("empty", &crate::l10n::tr("trash-empty-confirm"));
        dialog.set_response_appearance("empty", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        let this = self.clone();
        dialog.connect_response(None, move |dlg, response| {
            dlg.close();
            if response != "empty" {
                return;
            }
            let this = Rc::new(this.clone());
            Ctx::submit_job(
                &this,
                "operation-empty-trash",
                vec![kito_core::TRASH_URI.to_string()],
                None,
                None,
                move |cancellable| {
                    let outcome: Vec<OperationItemResult> = match
                        kito_core::empty_dir_detailed_cancellable(
                            kito_core::TRASH_URI,
                            cancellable,
                        ) {
                        Ok(items) => items
                            .into_iter()
                            .map(|item| {
                                let result = OperationItemResult::from_result(
                                    "operation-empty-trash",
                                    item.uri.clone(),
                                    None,
                                    item.error.map_or(Ok(()), Err),
                                );
                                if !matches!(result.state, ItemState::Cancelled(_)) {
                                    job_report(move |job| job.record_attempt());
                                }
                                result
                            })
                            .collect(),
                        Err(error) => vec![OperationItemResult::from_result(
                            "operation-empty-trash",
                            kito_core::TRASH_URI.to_string(),
                            None,
                            Err(error),
                        )],
                    };
                    outcome
                },
                move |this, finished_job, outcome| {
                    let results = match outcome {
                        Ok(results) => results,
                        Err(failure) => {
                            this.manager.reload_if_current(kito_core::TRASH_URI);
                            if !is_queued_cancellation(&failure) {
                                this.operation_worker_failed(&failure);
                            }
                            return;
                        }
                    };
                    this.job_center.finish_items(
                        &finished_job,
                        results
                            .iter()
                            .map(|result| (result.source.clone(), result.job_state()))
                            .collect(),
                    );
                    this.manager.reload_if_current(kito_core::TRASH_URI);
                    if results.is_empty() {
                        this.toast(&crate::l10n::tr("trash-already-empty"));
                    } else {
                        this.report_operation("operation-empty-trash", results, None);
                    }
                },
            );
        });
        dialog.present(Some(&self.window));
    }

    /// Shift+Delete: confirm and then really delete, recursively.
    pub fn delete_selected(&self) {
        let uris: Vec<String> = self
            .manager
            .selected_objects()
            .iter()
            .map(|o| o.uri())
            .collect();
        if uris.is_empty() {
            self.toast(&crate::l10n::tr("toast-nothing"));
            return;
        }
        let folder = self.manager.selected_uri().unwrap_or_default();
        self.confirm_delete(uris, folder);
    }

    fn confirm_delete(&self, uris: Vec<String>, folder: String) {
        let dialog = adw::AlertDialog::builder()
            .heading(crate::l10n::tr("delete-title"))
            .body(crate::l10n::tr_num("delete-body", uris.len() as u64))
            .build();
        dialog.add_response("cancel", &crate::l10n::tr("dialog-cancel"));
        dialog.add_response("delete", &crate::l10n::tr("delete-confirm"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        let this = self.clone();
        dialog.connect_response(None, move |dlg, response| {
            dlg.close();
            if response != "delete" {
                return;
            }
            let uris = uris.clone();
            let this = Rc::new(this.clone());
            let retry_folder = folder.clone();
            let completion_folder = folder.clone();
            Ctx::submit_job(
                &this,
                "operation-delete",
                uris.clone(),
                None,
                Some(uris.len() as u64),
                move |cancellable| {
                    uris.into_iter()
                        .map(|uri| {
                            let result = OperationItemResult::from_result(
                                "operation-delete",
                                uri.clone(),
                                None,
                                kito_core::delete_recursive_cancellable(&uri, cancellable),
                            );
                            if !matches!(result.state, ItemState::Cancelled(_)) {
                                job_report(move |job| job.record_attempt());
                            }
                            result
                        })
                        .collect::<Vec<_>>()
                },
                move |this, finished_job, outcome| {
                    let results = match outcome {
                        Ok(results) => results,
                        Err(failure) => {
                            this.manager.reload_if_current(&completion_folder);
                            if !is_queued_cancellation(&failure) {
                                this.operation_worker_failed(&failure);
                            }
                            return;
                        }
                    };
                    this.job_center.finish_items(
                        &finished_job,
                        results
                            .iter()
                            .map(|result| (result.source.clone(), result.job_state()))
                            .collect(),
                    );
                    this.manager.reload_if_current(&completion_folder);
                    let retry_this = this.clone();
                    let retry: RetryHandler = Rc::new(move |failed| {
                        let uris = failed.into_iter().map(|item| item.source).collect();
                        retry_this.confirm_delete(uris, retry_folder.clone());
                    });
                    this.report_operation("operation-delete", results, Some(retry));
                },
            );
        });
        dialog.present(Some(&self.window));
    }

    /// Pin/unpin of the single selected file or folder (context menu).
    /// Writes to `~/.config/gtk-3.0/bookmarks`: the sidebar reloads
    /// on its own via monitor (also applies to pins made by Nautilus).
    pub fn toggle_pin(&self) {
        let objs = self.manager.selected_objects();
        if objs.len() != 1 {
            self.toast(&crate::l10n::tr("pin-select-item"));
            return;
        }
        let uri = objs[0].uri();
        if kito_core::bookmarks::is_pinned(&uri) {
            match kito_core::bookmarks::unpin(&uri) {
                Ok(_) => {
                    self.preferences.notify_bookmarks_changed();
                    self.toast(&crate::l10n::tr("pin-unpinned"));
                    self.manager.reload_if_current(kito_core::FAVORITES_URI);
                }
                Err(e) => self.error_dialog(&crate::l10n::tr("error-unpin"), e.to_string()),
            }
        } else {
            match kito_core::bookmarks::pin(&objs[0].name(), &uri) {
                Ok(_) => {
                    self.preferences.notify_bookmarks_changed();
                    self.toast(&crate::l10n::tr("pin-pinned"));
                    self.manager.reload_if_current(kito_core::FAVORITES_URI);
                }
                Err(e) => self.error_dialog(&crate::l10n::tr("error-pin"), e.to_string()),
            }
        }
    }

    /// Adds the active real local folder to the shared Freedesktop bookmark
    /// list. This is deliberately distinct from pinning the selected item.
    pub fn bookmark_current_folder(&self) {
        let Some(uri) = self.manager.selected_uri() else {
            return;
        };
        if !current_location_bookmarkable(&uri) {
            self.toast(&crate::l10n::tr("side-bookmark-unavailable"));
            return;
        }
        let file = gio::File::for_uri(&uri);
        if kito_core::bookmarks::is_pinned(&uri) {
            self.toast(&crate::l10n::tr("side-bookmark-exists"));
            return;
        }
        let name = file
            .basename()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| kito_core::uri_to_display(&uri));
        match kito_core::bookmarks::pin(&name, &uri) {
            Ok(_) => {
                self.preferences.notify_bookmarks_changed();
                self.toast(&crate::l10n::tr("side-bookmark-added"));
            }
            Err(error) => {
                self.error_dialog(&crate::l10n::tr("side-bookmark-error"), error.to_string())
            }
        }
    }

    pub fn toggle_favorite(&self) {
        let objs = self.manager.selected_objects();
        if objs.len() != 1 {
            self.toast(&crate::l10n::tr("pin-select-item"));
            return;
        }
        let uri = objs[0].uri();
        match self.preferences.toggle_favorite(&uri) {
            Ok(added) => {
                self.toast(&crate::l10n::tr(if added {
                    "favorite-added"
                } else {
                    "favorite-removed"
                }));
            }
            Err(error) => self.error_dialog(&crate::l10n::tr("error-favorite"), error.to_string()),
        }
    }

    /// Dialog with a text field: `on_ok` receives the written value.
    /// The initial text stays selected, just type over it.
    fn name_dialog<F>(&self, title: &str, placeholder: &str, initial: &str, confirm: &str, on_ok: F)
    where
        F: Fn(String) + 'static,
    {
        let dialog = adw::Dialog::builder()
            .title(title)
            .content_width(360)
            .build();
        let entry = gtk::Entry::builder()
            .text(initial)
            .placeholder_text(placeholder)
            .build();
        let confirm_button = gtk::Button::builder()
            .label(confirm)
            .css_classes(["suggested-action"])
            .build();
        let cancel_button = gtk::Button::builder()
            .label(crate::l10n::tr("dialog-cancel"))
            .build();
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::End)
            .build();
        buttons.append(&cancel_button);
        buttons.append(&confirm_button);
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_start(16)
            .margin_end(16)
            .margin_top(16)
            .margin_bottom(16)
            .build();
        content.append(&entry);
        content.append(&buttons);
        dialog.set_child(Some(&content));

        let ok: Rc<dyn Fn()> = Rc::new({
            let dialog = dialog.clone();
            let entry = entry.clone();
            move || {
                let text = entry.text().to_string();
                dialog.close();
                on_ok(text);
            }
        });
        cancel_button.connect_clicked({
            let dialog = dialog.clone();
            move |_| {
                dialog.close();
            }
        });
        confirm_button.connect_clicked({
            let ok = ok.clone();
            move |_| ok()
        });
        entry.connect_activate({
            let ok = ok.clone();
            move |_| ok()
        });
        dialog.present(Some(&self.window));
        // Text selection exists only on a focused widget.
        let entry = entry.clone();
        glib::idle_add_local_once(move || {
            entry.grab_focus();
            entry.select_region(0, -1);
        });
    }

    /// F2: renames the single selected item.
    pub fn rename_selected(&self) {
        let objs = self.manager.selected_objects();
        if objs.len() != 1 {
            self.toast(&crate::l10n::tr("rename-select"));
            return;
        }
        let uri = objs[0].uri();
        let name = objs[0].name();
        let parent = uri
            .rsplit_once('/')
            .map(|(parent, _)| parent.to_string())
            .unwrap_or_default();
        let this = self.clone();
        self.name_dialog(
            &crate::l10n::tr("rename-title"),
            &crate::l10n::tr("rename-placeholder"),
            &name,
            &crate::l10n::tr("rename-confirm"),
            move |text| {
                let source = uri.clone();
                let operation_uri = uri.clone();
                let destination_parent = parent.clone();
                let this = this.clone();
                this.operation_queue.clone().submit(
                    move || {
                        let before = kito_core::file_identity(&operation_uri).ok();
                        let result = kito_core::rename(&operation_uri, &text);
                        let after = result
                            .as_ref()
                            .ok()
                            .and_then(|target| kito_core::file_identity(target).ok());
                        (result, unchanged_undo_identity(before, after))
                    },
                    move |outcome| {
                        let (result, identity) = match outcome {
                            Ok(result) => result,
                            Err(failure) => {
                                this.manager.reload_if_current(&destination_parent);
                                this.operation_worker_failed(&failure);
                                return;
                            }
                        };
                        this.manager.reload_if_current(&destination_parent);
                        if let (Ok(target), Some(identity)) = (&result, identity) {
                            this.operation_queue.push_undo(UndoRecord::Move {
                                source: source.clone(),
                                target: target.clone(),
                                identity,
                            });
                        }
                        let item = OperationItemResult::from_destination_result(
                            "operation-rename",
                            source,
                            destination_parent,
                            result,
                        );
                        this.report_operation("operation-rename", vec![item], None);
                    },
                );
            },
        );
    }

    /// Undo one confirmed operation. A failed/stale inverse remains at the
    /// top of history for a safe retry. Undo is refused while any queued or
    /// running mutation could make the captured inverse stale.
    pub fn undo(&self) {
        let record = match self.operation_queue.begin_undo() {
            Ok(record) => record,
            Err(UndoStartError::Busy | UndoStartError::InProgress) => {
                self.toast(&crate::l10n::tr("undo-in-progress"));
                return;
            }
            Err(UndoStartError::Empty) => {
                self.toast(&crate::l10n::tr("undo-empty"));
                return;
            }
        };
        let this = self.clone();
        let worker_record = record.record.clone();
        this.operation_queue.clone().submit(
            move || match &worker_record {
                UndoRecord::Move {
                    source,
                    target,
                    identity,
                } => kito_core::move_exact_checked(target, source, identity),
                UndoRecord::Trash {
                    source,
                    target,
                    identity,
                } => kito_core::restore_exact_checked(target, source, identity),
            },
            move |outcome| {
                let (source, target) = match &record.record {
                    UndoRecord::Move { source, target, .. }
                    | UndoRecord::Trash { source, target, .. } => (source.clone(), target.clone()),
                };
                let result = match outcome {
                    Ok(result) => result,
                    Err(failure) => {
                        this.operation_queue.finish_undo(record.id, false);
                        if let Some(parent) = gio::File::for_uri(&source).parent() {
                            this.manager.reload_if_current(&parent.uri());
                        }
                        if let Some(parent) = gio::File::for_uri(&target).parent() {
                            this.manager.reload_if_current(&parent.uri());
                        }
                        this.manager.reload_if_current(kito_core::TRASH_URI);
                        this.manager.reload_if_current(kito_core::FAVORITES_URI);
                        this.operation_worker_failed(&failure);
                        return;
                    }
                };
                this.operation_queue.finish_undo(record.id, result.is_ok());
                if let Some(parent) = gio::File::for_uri(&source).parent() {
                    this.manager.reload_if_current(&parent.uri());
                }
                if let Some(parent) = gio::File::for_uri(&target).parent() {
                    this.manager.reload_if_current(&parent.uri());
                }
                this.manager.reload_if_current(kito_core::TRASH_URI);
                if result.is_ok() {
                    this.toast(&crate::l10n::tr("undo-success"));
                } else {
                    this.toast(&crate::l10n::tr("undo-conflict"));
                }
                this.manager.reload_if_current(kito_core::FAVORITES_URI);
            },
        );
    }

    /// New folder with an explicit destination (background menu).
    /// [`Self::new_folder`] keeps the selected-tab behavior for the shared
    /// action and shortcuts.
    pub fn new_folder_at(&self, dest: &str) {
        if dest == kito_core::FAVORITES_URI {
            self.toast(&crate::l10n::tr("favorites-not-a-folder"));
            return;
        }
        let dest = dest.to_string();
        let this = self.clone();
        self.name_dialog(
            &crate::l10n::tr("new-folder-title"),
            &crate::l10n::tr("new-folder-placeholder"),
            &crate::l10n::tr("new-folder-initial"),
            &crate::l10n::tr("new-folder-confirm"),
            move |text| {
                let destination = dest.clone();
                let parent = dest.clone();
                let operation_dest = dest.clone();
                let this = this.clone();
                this.operation_queue.clone().submit(
                    move || kito_core::mkdir(&operation_dest, &text),
                    move |outcome| {
                        let result = match outcome {
                            Ok(result) => result,
                            Err(failure) => {
                                this.manager.reload_if_current(&parent);
                                this.operation_worker_failed(&failure);
                                return;
                            }
                        };
                        this.manager.reload_if_current(&parent);
                        let item = OperationItemResult::from_destination_result(
                            "operation-create-folder",
                            destination,
                            parent,
                            result,
                        );
                        this.report_operation("operation-create-folder", vec![item], None);
                    },
                );
            },
        );
    }

    /// New folder in the current folder (shared action, Ctrl+Shift+N).
    pub fn new_folder(&self) {
        let Some(dest) = self.manager.selected_uri() else {
            return;
        };
        self.new_folder_at(&dest);
    }

    /// New file with an explicit destination (background menu).
    /// [`Self::new_file`] keeps the selected-tab behavior for the shared
    /// action.
    pub fn new_file_at(&self, dest: &str, template: String) {
        if dest == kito_core::FAVORITES_URI {
            self.toast(&crate::l10n::tr("favorites-not-a-folder"));
            return;
        }
        if !dest.starts_with("file://") {
            self.error_dialog(
                &crate::l10n::tr("term-cannot-here"),
                crate::l10n::tr("term-local-only"),
            );
            return;
        }
        let dest = dest.to_string();
        let this = self.clone();
        self.name_dialog(
            &crate::l10n::tr("new-file-title"),
            &crate::l10n::tr("new-file-placeholder"),
            &template,
            &crate::l10n::tr("new-file-confirm"),
            move |text| {
                let destination = dest.clone();
                let parent = dest.clone();
                let operation_dest = dest.clone();
                let this = this.clone();
                this.operation_queue.clone().submit(
                    move || kito_core::create_file(&operation_dest, &text),
                    move |outcome| {
                        let result = match outcome {
                            Ok(result) => result,
                            Err(failure) => {
                                this.manager.reload_if_current(&parent);
                                this.operation_worker_failed(&failure);
                                return;
                            }
                        };
                        this.manager.reload_if_current(&parent);
                        let item = OperationItemResult::from_destination_result(
                            "operation-create-file",
                            destination,
                            parent,
                            result,
                        );
                        this.report_operation("operation-create-file", vec![item], None);
                    },
                );
            },
        );
    }

    /// New file: the chosen type is only the initial suggested name, the
    /// real name is always written by the user.
    pub fn new_file(&self, template: String) {
        let Some(dest) = self.manager.selected_uri() else {
            return;
        };
        self.new_file_at(&dest, template);
    }

    /// Opens the system terminal in the current folder.
    pub fn open_terminal(&self) {
        self.launch_terminal(false);
    }

    /// Opens the terminal with a root shell (`sudo -i` inside the terminal).
    pub fn open_terminal_root(&self) {
        self.launch_terminal(true);
    }

    /// Terminal in an explicit folder (background menu). Same guards as
    /// [`Self::launch_terminal`]: local paths only, configured emulator,
    /// errors reported at operation time.
    pub fn open_terminal_at(&self, dest_uri: &str, root: bool) {
        // Real local path via GIO (decoded: spaces, Unicode, `%`, `#`...).
        // `None` on non-local locations: refuse with a clear message.
        let Some(path) = kito_core::uri_to_path(dest_uri) else {
            self.error_dialog(
                &crate::l10n::tr("term-cannot-here"),
                crate::l10n::tr("term-local-only"),
            );
            return;
        };
        let heading = if root {
            "error-terminal-root"
        } else {
            "error-terminal"
        };
        let choice = self.preferences.snapshot().terminal;
        if let Err(e) = crate::terminal::open(&path, root, &choice) {
            self.error_dialog(&crate::l10n::tr(heading), e.user_message());
        }
    }

    fn launch_terminal(&self, root: bool) {
        let Some(uri) = self.manager.selected_uri() else {
            return;
        };
        self.open_terminal_at(&uri, root);
    }

    /// Properties: the selected entry (if just one) or the folder.
    pub fn show_properties(&self) {
        let selected = self.manager.selected_objects();
        if selected.len() > 1 {
            self.toast(&crate::l10n::tr("properties-select-one"));
            return;
        }
        let uri = if selected.len() == 1 {
            selected[0].uri()
        } else {
            self.manager.selected_uri().unwrap_or_default()
        };
        if uri.is_empty() {
            return;
        }
        self.show_properties_for(&uri);
    }

    /// Properties of an explicit folder URI (background menu): ignores any
    /// selected file without clearing the selection.
    pub fn show_folder_properties(&self, dest_uri: &str) {
        if dest_uri.is_empty() {
            return;
        }
        if dest_uri == kito_core::FAVORITES_URI {
            self.toast(&crate::l10n::tr("favorites-not-a-folder"));
            return;
        }
        self.show_properties_for(dest_uri);
    }

    fn show_properties_for(&self, uri: &str) {
        let uri = uri.to_string();
        let this = self.clone();
        Self::run_in_thread(
            move || {
                let info = kito_core::props(&uri).map_err(|error| error.to_string())?;
                let child_count = info
                    .is_dir
                    .then(|| {
                        kito_core::list_dir(&uri, true)
                            .ok()
                            .map(|entries| entries.len())
                    })
                    .flatten();
                Ok::<_, String>((info, child_count))
            },
            move |result| match result {
                Ok((info, child_count)) => this.present_properties(info, child_count),
                Err(error) => this.error_dialog(&crate::l10n::tr("error-props"), error),
            },
        );
    }

    fn present_properties(&self, info: kito_core::Props, child_count: Option<usize>) {
        let dialog = adw::Dialog::builder()
            .title(crate::l10n::tr("props-title"))
            .content_width(420)
            .build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_start(20)
            .margin_end(20)
            .margin_top(20)
            .margin_bottom(20)
            .build();

        // Header: big icon + name + type.
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(14)
            .build();
        let icon_name = if info.is_dir {
            "folder".to_string()
        } else {
            info.content_type
                .as_deref()
                .map(gio::content_type_get_icon)
                .and_then(|icon| icon.downcast::<gio::ThemedIcon>().ok())
                .and_then(|themed| themed.names().first().map(|n| n.to_string()))
                .unwrap_or_else(|| "text-x-generic".to_string())
        };
        header.append(&gtk::Image::from_gicon(&gio::ThemedIcon::from_names(&[
            &icon_name,
            "text-x-generic",
        ])));
        let titles = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .valign(gtk::Align::Center)
            .build();
        titles.append(
            &gtk::Label::builder()
                .label(&info.name)
                .halign(gtk::Align::Start)
                .css_classes(["title-2"])
                .build(),
        );
        titles.append(
            &gtk::Label::builder()
                .label(crate::l10n::tr(if info.is_dir {
                    "props-folder"
                } else {
                    "props-file"
                }))
                .halign(gtk::Align::Start)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        header.append(&titles);
        content.append(&header);

        // Name/value rows.
        let grid = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build();
        let type_label = match &info.content_type {
            Some(ct) if !info.is_dir => ct.clone(),
            _ => crate::l10n::tr("props-folder"),
        };
        grid.append(&prop_row(&crate::l10n::tr("props-type"), &type_label));
        let size = if info.is_dir {
            child_count
                .map(|count| crate::l10n::tr_num("props-size-items", count as u64))
                .unwrap_or_else(|| "—".to_string())
        } else {
            crate::file_list::human_size(info.size)
        };
        grid.append(&prop_row(&crate::l10n::tr("props-size"), &size));
        let location = info
            .uri
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or(&info.uri);
        grid.append(&prop_row(&crate::l10n::tr("props-location"), location));
        let modified = info
            .modified
            .and_then(|t| glib::DateTime::from_unix_local(t).ok())
            .and_then(|dt| dt.format("%Y-%m-%d %H:%M").ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "—".to_string());
        grid.append(&prop_row(&crate::l10n::tr("props-modified"), &modified));
        content.append(&grid);

        let close = gtk::Button::builder()
            .label(crate::l10n::tr("props-close"))
            .halign(gtk::Align::End)
            .css_classes(["suggested-action"])
            .build();
        close.connect_clicked({
            let dialog = dialog.clone();
            move |_| {
                dialog.close();
            }
        });
        content.append(&close);
        dialog.set_child(Some(&content));
        dialog.present(Some(&self.window));
    }
}

/// Label/value row for the Properties window.
fn prop_row(label: &str, value: &str) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .build();
    row.append(
        &gtk::Label::builder()
            .label(label)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .css_classes(["dim-label"])
            .build(),
    );
    row.append(
        &gtk::Label::builder()
            .label(value)
            .halign(gtk::Align::End)
            .selectable(true)
            .build(),
    );
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_operation_summary_supplies_fluent_plural_count() {
        let args = operation_summary_args(
            "Copy".to_string(),
            1,
            0,
            0,
            1,
        );
        let i18n = kito_i18n::I18n::new(kito_i18n::AppLang::English, None);

        assert_eq!(
            i18n.tr_with("operation-result-success", &args),
            "Copy: completed one item."
        );
    }

    fn move_result(source: &str, result: Result<(), glib::Error>) -> OperationItemResult {
        OperationItemResult::from_result(
            "operation-move",
            source.to_string(),
            Some("file:///dest".to_string()),
            result,
        )
    }

    fn failed_error() -> glib::Error {
        glib::Error::new(gio::IOErrorEnum::Failed, "boom")
    }

    fn isolated_fixture() -> (tempfile::TempDir, std::path::PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        let uri = gio::File::for_path(&path).uri().to_string();
        (dir, path, uri)
    }

    #[test]
    fn successful_move_keeps_undo_identity_when_cancelled_afterward() {
        let tmp = tempfile::tempdir().unwrap();
        let source_dir = tmp.path().join("source");
        let destination_dir = tmp.path().join("destination");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::create_dir(&destination_dir).unwrap();
        let source = source_dir.join("item.txt");
        std::fs::write(&source, b"temporary fixture").unwrap();
        let source_uri = gio::File::for_path(&source).uri().to_string();
        let destination_uri = gio::File::for_path(&destination_dir).uri().to_string();
        let before = kito_core::file_identity(&source_uri).unwrap();
        let cancellable = gio::Cancellable::new();

        let target = kito_core::move_to_exact_result_cancellable(
            &source_uri,
            &destination_uri,
            &cancellable,
        )
        .unwrap();
        cancellable.cancel();

        let after = kito_core::file_identity(&target).unwrap();
        let identity = unchanged_undo_identity(Some(before.clone()), Some(after))
            .expect("completed move should retain its undo identity");
        let undo = UndoRecord::Move {
            source: source_uri.clone(),
            target: target.clone(),
            identity,
        };
        assert!(matches!(undo, UndoRecord::Move { identity, .. } if identity == before));
        assert!(!source.exists());
        assert!(gio::File::for_uri(&target).query_exists(gio::Cancellable::NONE));
    }

    #[test]
    fn external_move_ignores_only_missing_sources_without_fabricating_success() {
        let (_dir, path, uri) = isolated_fixture();
        std::fs::write(&path, "temporary fixture").unwrap();
        std::fs::remove_file(&path).unwrap();

        // Model the exact GIO error returned when the destination has already
        // removed this fixture. No trash operation is invoked by this test.
        let result = external_move_trash_result(
            uri.clone(),
            Err(glib::Error::new(gio::IOErrorEnum::NotFound, "missing")),
            || external_move_source_check(&uri, &gio::Cancellable::new()),
        );
        assert!(result.is_none());
        // Empty results are intentionally not reported (and hence produce no
        // success toast) by report_operation.
        assert!(std::fs::metadata(&path).is_err());
    }

    #[test]
    fn external_move_keeps_other_trash_errors_visible_and_retryable() {
        let (_dir, path, uri) = isolated_fixture();
        std::fs::write(&path, "temporary fixture").unwrap();

        let result = external_move_trash_result(
            uri.clone(),
            Err(glib::Error::new(
                gio::IOErrorEnum::PermissionDenied,
                "permission denied",
            )),
            || external_move_source_check(&uri, &gio::Cancellable::new()),
        )
        .expect("non-NotFound errors remain reportable");
        assert_eq!(result.source, uri);
        assert!(matches!(result.state, ItemState::Failed(_)));
        assert!(result.needs_retry());
        assert!(path.exists());
    }

    #[test]
    fn external_move_keeps_not_found_when_source_still_exists() {
        let (_dir, path, uri) = isolated_fixture();
        std::fs::write(&path, "temporary fixture").unwrap();

        let result = external_move_trash_result(
            uri.clone(),
            Err(glib::Error::new(gio::IOErrorEnum::NotFound, "trash backend")),
            || external_move_source_check(&uri, &gio::Cancellable::new()),
        )
        .expect("existing source means NotFound came from the trash backend");
        assert_eq!(result.source, uri);
        assert!(matches!(result.state, ItemState::Failed(_)));
        assert!(result.needs_retry());
        assert!(path.exists());
    }

    #[test]
    fn external_move_keeps_not_found_when_source_check_is_uncertain() {
        let (_dir, path, uri) = isolated_fixture();
        std::fs::write(&path, "temporary fixture").unwrap();

        let result = external_move_trash_result(
            uri.clone(),
            Err(glib::Error::new(gio::IOErrorEnum::NotFound, "trash backend")),
            || {
                Err(glib::Error::new(
                    gio::IOErrorEnum::PermissionDenied,
                    "cannot check source",
                ))
            },
        )
        .expect("an inconclusive source check must preserve the trash error");
        assert_eq!(result.source, uri);
        assert!(matches!(result.state, ItemState::Failed(_)));
        assert!(result.needs_retry());
        assert!(path.exists());
    }

    #[test]
    fn external_move_reports_cancelled_source_check_instead_of_inferring_absence() {
        let (_dir, path, uri) = isolated_fixture();
        std::fs::write(&path, "temporary fixture").unwrap();
        let cancellable = gio::Cancellable::new();
        cancellable.cancel();

        let result = external_move_trash_result(
            uri.clone(),
            Err(glib::Error::new(gio::IOErrorEnum::NotFound, "trash result")),
            || external_move_source_check(&uri, &cancellable),
        )
        .expect("a cancelled check must remain visible, not infer source absence");
        assert_eq!(result.source, uri);
        assert!(matches!(result.state, ItemState::Cancelled(_)));
        assert!(path.exists());
    }

    #[test]
    fn external_move_batch_does_not_lose_other_item_outcomes() {
        let (_removed_dir, removed_path, removed_uri) = isolated_fixture();
        std::fs::write(&removed_path, "temporary fixture").unwrap();
        std::fs::remove_file(&removed_path).unwrap();
        let missing = external_move_trash_result(
            removed_uri.clone(),
            Err(glib::Error::new(gio::IOErrorEnum::NotFound, "missing")),
            || external_move_source_check(&removed_uri, &gio::Cancellable::new()),
        );

        let (_error_dir, error_path, error_uri) = isolated_fixture();
        std::fs::write(&error_path, "temporary fixture").unwrap();
        let failed = external_move_trash_result(
            error_uri.clone(),
            Err(glib::Error::new(gio::IOErrorEnum::PermissionDenied, "denied")),
            || external_move_source_check(&error_uri, &gio::Cancellable::new()),
        );

        let (_success_dir, success_path, success_uri) = isolated_fixture();
        std::fs::write(&success_path, "temporary fixture").unwrap();
        let succeeded = external_move_trash_result(success_uri.clone(), Ok(()), || {
            external_move_source_check(&success_uri, &gio::Cancellable::new())
        });

        let outcomes: Vec<_> = [missing, failed, succeeded].into_iter().flatten().collect();
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].source, error_uri);
        assert!(matches!(outcomes[0].state, ItemState::Failed(_)));
        assert_eq!(outcomes[1].source, success_uri);
        assert!(matches!(outcomes[1].state, ItemState::Succeeded));
    }

    #[test]
    fn move_ack_is_sent_only_after_complete_success() {
        use crate::dnd::TransferAction;
        // Full success: the external source may delete its data.
        assert_eq!(
            ack_action(
                TransferAction::Move,
                &[
                    move_result("file:///a", Ok(())),
                    move_result("file:///b", Ok(())),
                ],
            ),
            gdk::DragAction::MOVE
        );
        // Partial transfer: no ack, so the source deletes nothing and the
        // items that did not arrive stay on the source side.
        assert_eq!(
            ack_action(
                TransferAction::Move,
                &[
                    move_result("file:///a", Ok(())),
                    move_result("file:///b", Err(failed_error())),
                ],
            ),
            gdk::DragAction::empty()
        );
        // All failed, cancelled or empty batches never confirm a move.
        assert_eq!(
            ack_action(
                TransferAction::Move,
                &[move_result("file:///a", Err(failed_error()))],
            ),
            gdk::DragAction::empty()
        );
        assert_eq!(
            ack_action(
                TransferAction::Move,
                &[move_result(
                    "file:///a",
                    Err(glib::Error::new(gio::IOErrorEnum::Cancelled, "stopped")),
                )],
            ),
            gdk::DragAction::empty()
        );
        assert_eq!(
            ack_action(TransferAction::Move, &[]),
            gdk::DragAction::empty()
        );
    }

    #[test]
    fn copy_and_link_ack_partial_success() {
        use crate::dnd::TransferAction;
        let results = [
            move_result("file:///a", Ok(())),
            move_result("file:///b", Err(failed_error())),
        ];
        assert_eq!(
            ack_action(TransferAction::Copy, &results),
            gdk::DragAction::COPY
        );
        assert_eq!(
            ack_action(TransferAction::Link, &results),
            gdk::DragAction::LINK
        );
        let failed = [move_result("file:///a", Err(failed_error()))];
        assert_eq!(
            ack_action(TransferAction::Copy, &failed),
            gdk::DragAction::empty()
        );
        assert_eq!(
            ack_action(TransferAction::Copy, &[]),
            gdk::DragAction::empty()
        );
    }

    #[test]
    fn undo_records_require_identity_to_survive_the_move() {
        let before = kito_core::FileIdentity {
            filesystem: "fs".into(),
            file: "inode-before".into(),
        };
        let same = before.clone();
        assert_eq!(
            unchanged_undo_identity(Some(before.clone()), Some(same)),
            Some(before.clone())
        );

        let copied = kito_core::FileIdentity {
            filesystem: "other-fs".into(),
            file: "new-inode".into(),
        };
        assert_eq!(unchanged_undo_identity(Some(before), Some(copied)), None);
        assert_eq!(unchanged_undo_identity(None, None), None);
    }

    #[test]
    fn current_bookmark_destination_allows_local_and_remote_gio_locations_only() {
        for uri in ["file:///tmp", "smb://server/share", "sftp://host/home/user"] {
            assert!(current_location_bookmarkable(uri), "{uri}");
        }
        for uri in [
            "",
            kito_core::FAVORITES_URI,
            kito_core::ADD_BOOKMARK_URI,
            kito_core::TRASH_URI,
            "network:///",
            "recent:///",
            "search:///",
        ] {
            assert!(!current_location_bookmarkable(uri), "{uri}");
        }
    }

    /// Delivery through the channel future. One test (not two) so parallel
    /// test threads never contend for default-context ownership.
    #[test]
    fn run_in_thread_delivers_values_and_errors_on_the_main_loop() {
        use std::time::{Duration, Instant};

        let context = glib::MainContext::default();
        let pump = |slot: &Rc<RefCell<Option<String>>>| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while slot.borrow().is_none() {
                assert!(
                    Instant::now() < deadline,
                    "worker result was never delivered"
                );
                context.iteration(true);
            }
        };

        let done = Rc::new(RefCell::new(None));
        let done2 = done.clone();
        Ctx::run_in_thread(
            || 41 + 1,
            move |value: i32| *done2.borrow_mut() = Some(value.to_string()),
        );
        pump(&done);
        assert_eq!(done.borrow().as_deref(), Some("42"));

        let failed = Rc::new(RefCell::new(None));
        let failed2 = failed.clone();
        Ctx::run_in_thread(
            || "boom".to_string(),
            move |error: String| *failed2.borrow_mut() = Some(error),
        );
        pump(&failed);
        assert_eq!(failed.borrow().as_deref(), Some("boom"));
    }

    fn entries(uris: &[&str], cut: bool) -> Clipboard {
        Clipboard {
            uris: uris.iter().map(|s| s.to_string()).collect(),
            cut,
        }
    }

    const A: &str = "file:///tmp/A.txt";
    const B: &str = "file:///tmp/B.txt";

    fn finish_cut_with_failed_uris(
        clip: &mut ClipTracker,
        op: u64,
        failed_uris: &[String],
    ) -> CutFinish {
        let dispatch = clip.cut_in_flight.as_ref().unwrap().clone();
        let results = dispatch
            .uris
            .iter()
            .map(|uri| {
                if failed_uris.contains(uri) {
                    OperationItemResult::from_move_result(
                        "operation-move",
                        uri.clone(),
                        "file:///tmp/destination".into(),
                        None,
                        Err(glib::Error::new(gio::IOErrorEnum::PermissionDenied, "test failure")),
                    )
                } else {
                    OperationItemResult::from_result(
                        "operation-move",
                        uri.clone(),
                        Some("file:///tmp/destination".into()),
                        Ok(()),
                    )
                }
            })
            .collect::<Vec<_>>();
        clip.finish_cut(op, &results)
    }

    #[test]
    fn captured_group_transfer_is_independent_of_later_selection_changes() {
        let mut selection = vec![A.to_string(), B.to_string()];
        let transfer = CapturedTransfer {
            uris: selection.clone(),
            destination: "file:///tmp/destination".into(),
            cut: true,
            move_with_backend: true,
            identity_guards: HashMap::new(),
        };
        selection.clear();
        selection.push("file:///tmp/other.txt".into());

        assert_eq!(transfer.uris, vec![A.to_string(), B.to_string()]);
        assert_eq!(transfer.destination, "file:///tmp/destination");
        assert!(transfer.cut && transfer.move_with_backend);
    }

    #[test]
    fn external_copy_replaces_internal() {
        let mut clip = ClipTracker::default();
        let published_a = clip.publish(entries(&[A], false));
        clip.on_own_changed();
        // Another app publishes B afterwards.
        clip.on_external_changed();
        let published_b = format!("{B}\n");
        assert_eq!(
            clip.resolve(Some(&published_b), clip.generation),
            PasteDecision::External {
                uris: vec![B.to_string()]
            }
        );
        // A is gone: never pasted from stale state.
        assert_ne!(published_a, published_b);
    }

    #[test]
    fn external_plain_text_pastes_nothing() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], false));
        clip.on_own_changed();
        clip.on_external_changed();
        assert_eq!(
            clip.resolve(Some("just some text"), clip.generation),
            PasteDecision::Unsupported
        );
        assert_eq!(
            clip.resolve(None, clip.generation),
            PasteDecision::Unsupported
        );
    }

    #[test]
    fn foreign_clipboard_provider_is_not_authorized_for_mutation() {
        let our_provider = 0x10usize;
        let foreign_provider = 0x20usize;

        assert!(clipboard_provider_is_owned(
            Some(our_provider),
            Some(our_provider)
        ));
        assert!(!clipboard_provider_is_owned(
            Some(foreign_provider),
            Some(our_provider)
        ));
        assert!(!clipboard_provider_is_owned(None, Some(our_provider)));
        assert!(!clipboard_provider_is_owned(Some(our_provider), None));

        // Model the decision without constructing a display or mutating the
        // process clipboard: a foreign provider must leave its text untouched.
        let foreign_text = B.to_string();
        let mut simulated_clipboard_text = foreign_text.clone();
        if clipboard_provider_is_owned(Some(foreign_provider), Some(our_provider)) {
            simulated_clipboard_text.clear();
        }
        assert_eq!(simulated_clipboard_text, foreign_text);
    }

    #[test]
    fn paste_resolution_requires_the_same_owned_provider_as_the_async_read() {
        let mut tracker = ClipTracker::default();
        let published = tracker.publish(entries(&[A], true));
        let generation = tracker.generation;
        let owned_provider = 0x10usize;

        let internal = tracker.resolve_with_provider_snapshot(
            Some(&published),
            generation,
            Some(owned_provider),
            Some(owned_provider),
            Some(owned_provider),
        );
        assert_eq!(
            internal,
            PasteDecision::Internal {
                uris: vec![A.to_string()],
                cut: true,
            }
        );

        // Equal URI text from a foreign provider remains an external COPY;
        // it cannot dispatch a MOVE against the internal cut tracker.
        let foreign_provider = 0x20usize;
        let external = tracker.resolve_with_provider_snapshot(
            Some(&published),
            generation,
            Some(foreign_provider),
            Some(foreign_provider),
            Some(owned_provider),
        );
        assert_eq!(
            external,
            PasteDecision::External {
                uris: vec![A.to_string()],
            }
        );
        let shared_tracker = RefCell::new(tracker);
        assert_eq!(
            paste_action(&shared_tracker, &external, Some(&published)),
            PasteAction::Copy(vec![A.to_string()])
        );
        assert!(shared_tracker.borrow().cut_in_flight.is_none());

        // If the provider changes after read_text_async starts, discard its
        // text entirely, even if that text equals the formerly owned cut.
        let stale = shared_tracker.borrow().resolve_with_provider_snapshot(
            Some(&published),
            generation,
            Some(owned_provider),
            Some(foreign_provider),
            Some(owned_provider),
        );
        assert_eq!(stale, PasteDecision::Stale);
        assert_eq!(
            paste_action(&shared_tracker, &stale, Some(&published)),
            PasteAction::Explain("clip-changed")
        );
        assert!(shared_tracker.borrow().cut_in_flight.is_none());
        assert_eq!(shared_tracker.borrow().entries.as_ref().unwrap().uris, vec![A]);
    }

    #[test]
    fn successful_move_consumes_cut() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A, B], true));
        clip.on_own_changed();
        let op = clip.begin_cut().expect("first move dispatches").generation;
        assert_eq!(finish_cut_with_failed_uris(&mut clip, op, &[]), CutFinish::Consumed);
        // The old URI text remains published, but resolves to an empty
        // internal tombstone instead of falling back to a foreign copy.
        assert_eq!(
            clip.resolve(Some(&published), clip.generation),
            PasteDecision::Internal {
                uris: Vec::new(),
                cut: true,
            }
        );
        assert_eq!(clip.published_text.as_deref(), Some(published.as_str()));
        assert!(!clip.has_usable_entries());
        assert_eq!(clip.begin_cut(), None);
    }

    /// Regression: after a completed cut the system provider may still
    /// publish the old URI text (Kito never empties a clipboard it cannot
    /// claim). Resolving that tombstone must explain an empty clipboard and
    /// must not start a MOVE job or dispatch anything.
    #[test]
    fn completed_cut_tombstone_plans_empty_clipboard_instead_of_a_move() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A, B], true));
        clip.on_own_changed();
        let op = clip.begin_cut().expect("first move dispatches").generation;
        assert_eq!(
            finish_cut_with_failed_uris(&mut clip, op, &[]),
            CutFinish::Consumed
        );
        let clipboard = RefCell::new(clip);

        let decision = clipboard
            .borrow()
            .resolve(Some(&published), clipboard.borrow().generation);
        assert!(matches!(
            &decision,
            PasteDecision::Internal { uris, cut: true } if uris.is_empty()
        ));
        assert_eq!(
            paste_action(&clipboard, &decision, Some(&published)),
            PasteAction::Explain("clip-empty")
        );
        // Nothing was dispatched and nothing can be: the tombstone is never
        // reinterpreted as foreign copy data nor as a new cut.
        assert!(clipboard.borrow().cut_in_flight.is_none());
        assert_eq!(clipboard.borrow_mut().begin_cut(), None);
        assert!(!clipboard.borrow().has_usable_entries());
    }

    #[test]
    fn live_cut_dispatches_one_move_while_copy_plans_copies() {
        let clipboard = RefCell::new(ClipTracker::default());
        let cut_text = clipboard.borrow_mut().publish(entries(&[A], true));
        let generation = clipboard.borrow().generation;
        let decision = clipboard.borrow().resolve(Some(&cut_text), generation);
        let action = paste_action(&clipboard, &decision, Some(&cut_text));
        assert!(matches!(
            &action,
            PasteAction::Move(dispatch)
                if dispatch.generation == generation && dispatch.uris == vec![A.to_string()]
        ));
        // While that move runs, the same text resolves to a refusal instead
        // of a second dispatch, and the tracker stays mutably reachable.
        let decision = clipboard.borrow().resolve(Some(&cut_text), generation);
        assert_eq!(
            paste_action(&clipboard, &decision, Some(&cut_text)),
            PasteAction::MoveRefused
        );
        assert!(clipboard.borrow().cut_in_flight.is_some());

        let copy_text = clipboard.borrow_mut().publish(entries(&[B], false));
        let decision = clipboard
            .borrow()
            .resolve(Some(&copy_text), clipboard.borrow().generation);
        assert!(matches!(
            paste_action(&clipboard, &decision, Some(&copy_text)),
            PasteAction::Copy(uris) if uris == vec![B.to_string()]
        ));
    }

    #[test]
    fn cut_clipboard_is_shared_between_windows_and_new_publish_stales_old_guard() {
        // These handles model the `Ctx` stored by two windows. A failed first
        // MOVE retains its original identity for a retry from the other one.
        let shared = Rc::new(RefCell::new(ClipTracker::default()));
        let window_a_clipboard = shared.clone();
        let window_b_clipboard = shared.clone();
        let published_text = window_a_clipboard
            .borrow_mut()
            .publish(entries(&[A], true));
        let generation = window_a_clipboard.borrow().generation;
        let first_dispatch = window_a_clipboard.borrow_mut().begin_cut().unwrap();
        let original_identity = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "original-object".into(),
        };
        let first_failure = OperationItemResult::from_move_result(
            "operation-move",
            A.to_string(),
            "file:///tmp/destination".into(),
            Some(original_identity.clone()),
            Err(glib::Error::new(
                gio::IOErrorEnum::PermissionDenied,
                "retryable failure",
            )),
        );
        assert_eq!(
            window_a_clipboard
                .borrow_mut()
                .finish_cut(first_dispatch.generation, &[first_failure]),
            CutFinish::Partial
        );

        // The second window reads the same system text and resolves it using
        // the process-wide tracker, so it receives a guarded MOVE, not COPY.
        let decision = window_b_clipboard
            .borrow()
            .resolve(Some(&published_text), generation);
        let PasteAction::Move(retry_dispatch) =
            paste_action(&window_b_clipboard, &decision, Some(&published_text))
        else {
            panic!("a second window must resolve the shared cut as a move");
        };
        assert_eq!(retry_dispatch.generation, generation);
        assert_eq!(retry_dispatch.uris, vec![A]);
        assert_eq!(
            retry_dispatch.identity_guards.get(A),
            Some(&MoveIdentityGuard::Stable(original_identity))
        );

        // A publication in either window retires the shared generation and
        // identity policy. A late completion from the prior window cannot
        // alter the new cut or consume its in-flight dispatch.
        let replacement_text = window_a_clipboard
            .borrow_mut()
            .publish(entries(&[B], true));
        let replacement_dispatch = window_a_clipboard.borrow_mut().begin_cut().unwrap();
        assert_ne!(window_b_clipboard.borrow().generation, generation);
        assert_eq!(
            window_b_clipboard
                .borrow()
                .resolve(Some(&published_text), generation),
            PasteDecision::Stale
        );
        assert!(window_b_clipboard
            .borrow()
            .cut_identity_guards
            .is_empty());
        assert_eq!(
            window_b_clipboard.borrow_mut().finish_cut(
                retry_dispatch.generation,
                &[OperationItemResult::from_result(
                    "operation-move",
                    A.to_string(),
                    Some("file:///tmp/destination".into()),
                    Ok(()),
                )],
            ),
            CutFinish::Untouched
        );
        let tracker = window_b_clipboard.borrow();
        assert_eq!(tracker.published_text.as_deref(), Some(replacement_text.as_str()));
        assert_eq!(tracker.entries.as_ref().unwrap().uris, vec![B]);
        assert_eq!(tracker.cut_in_flight, Some(replacement_dispatch));
    }

    /// Regression: a publish rejected by GDK resets the internal state and
    /// must never declare Copied/Cut. Both catalogs carry the message.
    #[test]
    fn failed_clipboard_publish_never_reports_success() {
        assert_eq!(publish_toast_key(false, false), "clip-publish-failed");
        assert_eq!(publish_toast_key(false, true), "clip-publish-failed");
        assert_eq!(publish_toast_key(true, false), "toast-copied");
        assert_eq!(publish_toast_key(true, true), "toast-cut");

        let mut tracker = ClipTracker::default();
        tracker.publish(entries(&[A], true));
        assert!(tracker.has_usable_entries());
        tracker.on_external_changed();
        assert!(!tracker.has_usable_entries());

        for lang in [kito_i18n::AppLang::English, kito_i18n::AppLang::Italian] {
            let i18n = kito_i18n::I18n::new(lang, None);
            let message = i18n.tr("clip-publish-failed");
            assert_ne!(message, "clip-publish-failed", "missing catalog entry");
            assert!(!message.is_empty());
        }
    }

    #[test]
    fn failed_move_retries_as_move() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A, B], true));
        clip.on_own_changed();
        let op = clip.begin_cut().expect("first move dispatches").generation;
        assert_eq!(
            finish_cut_with_failed_uris(&mut clip, op, &[A.to_string(), B.to_string()]),
            CutFinish::Partial
        );
        // Nothing moved: the next paste retries the full move, never a copy.
        assert_eq!(
            clip.resolve(Some(&published), clip.generation),
            PasteDecision::Internal {
                uris: vec![A.to_string(), B.to_string()],
                cut: true,
            }
        );
    }

    #[test]
    fn rejected_destination_cancels_cut_without_consuming_clipboard() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A, B], true));
        let op = clip.begin_cut().expect("first move dispatches").generation;
        assert_eq!(clip.begin_cut(), None);

        clip.cancel_cut(op);

        assert_eq!(clip.entries.as_ref().unwrap().uris, vec![A, B]);
        assert!(clip.entries.as_ref().unwrap().cut);
        assert_eq!(clip.published_text.as_deref(), Some(published.as_str()));
        assert_eq!(clip.begin_cut().map(|dispatch| dispatch.generation), Some(op));
    }

    #[test]
    fn abandoned_cut_completion_marks_only_its_dispatch_unavailable() {
        let mut tracker = ClipTracker::default();
        tracker.publish(entries(&[A, B], true));
        let operation = tracker.generation;
        let identity_a = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "object-a".into(),
        };
        let identity_b = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "object-b".into(),
        };
        tracker
            .cut_identity_guards
            .insert(A.into(), MoveIdentityGuard::Stable(identity_a));
        tracker
            .cut_identity_guards
            .insert(B.into(), MoveIdentityGuard::Stable(identity_b.clone()));
        let dispatch = tracker.begin_cut_for(operation, &[A.to_string()]).unwrap();
        let tracker = Rc::new(RefCell::new(tracker));
        let worker_started = Arc::new(AtomicBool::new(true));

        drop(CutCompletionGuard::new(
            tracker.clone(),
            Some(operation),
            dispatch.uris,
            worker_started,
        ));

        let tracker = tracker.borrow();
        assert_eq!(tracker.cut_in_flight, None);
        assert_eq!(tracker.entries.as_ref().unwrap().uris, vec![A, B]);
        assert!(tracker.entries.as_ref().unwrap().cut);
        assert_eq!(
            tracker.cut_identity_guards.get(A),
            Some(&MoveIdentityGuard::Unavailable)
        );
        assert_eq!(
            tracker.cut_identity_guards.get(B),
            Some(&MoveIdentityGuard::Stable(identity_b))
        );
    }

    #[test]
    fn cut_completion_guard_does_not_mutate_a_new_clipboard_generation() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        let old_generation = clip.generation;
        let dispatch = clip.begin_cut().unwrap();
        let tracker = Rc::new(RefCell::new(clip));
        let guard = CutCompletionGuard::new(
            tracker.clone(),
            Some(old_generation),
            dispatch.uris,
            Arc::new(AtomicBool::new(true)),
        );

        let new_dispatch = {
            let mut clip = tracker.borrow_mut();
            clip.publish(entries(&[B], true));
            clip.cut_identity_guards.insert(
                B.into(),
                MoveIdentityGuard::Stable(kito_core::FileIdentity {
                    filesystem: "new-generation".into(),
                    file: "object-b".into(),
                }),
            );
            clip.begin_cut().unwrap()
        };
        drop(guard);

        let clip = tracker.borrow();
        assert_ne!(clip.generation, old_generation);
        assert_eq!(clip.entries.as_ref().unwrap().uris, vec![B]);
        assert!(matches!(
            clip.cut_identity_guards.get(B),
            Some(MoveIdentityGuard::Stable(identity))
                if identity.filesystem == "new-generation"
        ));
        assert_eq!(clip.cut_in_flight, Some(new_dispatch));
    }

    #[test]
    fn stale_rejected_cut_does_not_cancel_new_generation() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        let old_op = clip.generation;
        assert_eq!(clip.begin_cut().map(|dispatch| dispatch.generation), Some(old_op));

        clip.publish(entries(&[B], true));
        let new_op = clip.begin_cut().expect("new cut can dispatch").generation;
        clip.cancel_cut(old_op);

        assert_eq!(clip.entries.as_ref().unwrap().uris, vec![B]);
        assert_eq!(clip.begin_cut(), None);
        clip.cancel_cut(new_op);
    }

    #[test]
    fn partial_move_retries_only_failed() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A, B], true));
        clip.on_own_changed();
        let op = clip.begin_cut().expect("first move dispatches").generation;
        assert_eq!(
            finish_cut_with_failed_uris(&mut clip, op, &[B.to_string()]),
            CutFinish::Partial
        );
        assert_eq!(
            clip.resolve(Some(&published), clip.generation),
            PasteDecision::Internal {
                uris: vec![B.to_string()],
                cut: true,
            }
        );
        // The explicit retry stays a move and is bound to the same clipboard
        // generation. A clipboard replacement retires it.
        let retry = clip.begin_cut_for(op, &[B.to_string()]).unwrap();
        assert_eq!(retry.generation, op);
        assert_eq!(
            finish_cut_with_failed_uris(&mut clip, op, &[B.to_string()]),
            CutFinish::Partial
        );
        clip.on_external_changed();
        assert_eq!(clip.begin_cut_for(op, &[B.to_string()]), None);
    }

    #[test]
    fn operation_retry_candidates_exclude_successes() {
        let items = [
            OperationItemResult::from_result(
                "operation-copy",
                A.to_string(),
                Some("file:///tmp/dest".into()),
                Ok(()),
            ),
            OperationItemResult::from_result(
                "operation-copy",
                B.to_string(),
                Some("file:///tmp/dest".into()),
                Err(glib::Error::new(
                    gio::IOErrorEnum::PermissionDenied,
                    "permission denied",
                )),
            ),
            OperationItemResult::from_result(
                "operation-copy",
                "file:///partial".into(),
                Some("file:///tmp/dest".into()),
                Err(glib::Error::new(
                    gio::IOErrorEnum::Cancelled,
                    "partial copy output may remain at file:///tmp/dest/partial",
                )),
            ),
        ];
        let retry: Vec<_> = items
            .iter()
            .filter(|item| item.needs_retry())
            .map(|item| item.source.as_str())
            .collect();
        assert_eq!(retry, vec![B]);
    }

    #[test]
    fn retry_transfer_captures_identity_only_for_admitted_move_items() {
        let identity = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "original-object".into(),
        };
        let items = vec![
            OperationItemResult::from_move_result(
                "operation-move",
                A.to_string(),
                "file:///tmp/dest".into(),
                Some(identity.clone()),
                Err(glib::Error::new(gio::IOErrorEnum::PermissionDenied, "retry")),
            ),
            OperationItemResult::from_result(
                "operation-move",
                B.to_string(),
                Some("file:///tmp/dest".into()),
                Err(glib::Error::new(gio::IOErrorEnum::PermissionDenied, "no identity")),
            ),
            OperationItemResult::from_result(
                "operation-copy",
                "file:///tmp/copy.txt".into(),
                Some("file:///tmp/dest".into()),
                Err(glib::Error::new(gio::IOErrorEnum::PermissionDenied, "copy retry")),
            ),
            OperationItemResult::from_move_result(
                "operation-move",
                "file:///tmp/uncertain.txt".into(),
                "file:///tmp/dest".into(),
                Some(identity.clone()),
                Err(glib::Error::new(
                    gio::IOErrorEnum::Failed,
                    MOVE_RETRY_IDENTITY_CHANGED_MARKER,
                )),
            ),
        ];

        let (uris, identities) = retry_transfer_capture(items);

        assert_eq!(uris, vec![A, "file:///tmp/copy.txt"]);
        assert_eq!(identities.len(), 1);
        assert_eq!(
            identities.get(A),
            Some(&MoveIdentityGuard::Stable(identity))
        );
    }

    #[test]
    fn cut_retry_subset_preserves_unresolved_item_and_identity_guards() {
        let identity_a = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "object-a".into(),
        };
        let mut clip = ClipTracker::default();
        let clipboard_text = clip.publish(entries(&[A, B], true));
        assert_eq!(clipboard_text, format!("{A}\n{B}"));
        let op = clip.begin_cut().unwrap().generation;
        let first_results = vec![
            OperationItemResult::from_move_result(
                "operation-move",
                A.to_string(),
                "file:///tmp/destination".into(),
                Some(identity_a.clone()),
                Err(glib::Error::new(gio::IOErrorEnum::PermissionDenied, "retryable")),
            ),
            OperationItemResult::from_move_result(
                "operation-move",
                B.to_string(),
                "file:///tmp/destination".into(),
                None,
                Err(glib::Error::new(
                    gio::IOErrorEnum::Cancelled,
                    "cancelled; partial move output may remain at file:///tmp/destination/B",
                )),
            ),
        ];
        assert!(first_results[0].needs_retry());
        assert!(!first_results[1].needs_retry());
        assert_eq!(clip.finish_cut(op, &first_results), CutFinish::Partial);
        assert_eq!(clip.entries.as_ref().unwrap().uris, vec![A, B]);
        assert_eq!(clip.published_text.as_deref(), Some(clipboard_text.as_str()));

        // A normal Ctrl+V after the first failure carries the stored guard
        // for A and withholds the partial/uncertain B result.
        let paste_dispatch = clip.begin_cut().unwrap();
        assert_eq!(
            paste_dispatch.identity_guards.get(A),
            Some(&MoveIdentityGuard::Stable(identity_a.clone()))
        );
        assert!(matches!(
            paste_dispatch.identity_guards.get(B),
            Some(MoveIdentityGuard::Unavailable)
        ));
        assert!(matches!(
            move_identity_attempt(A, &paste_dispatch.identity_guards, || panic!(
                "Ctrl+V must not replace the stored identity with a fresh query"
            )),
            MoveIdentityAttempt::Authorized(Some(identity)) if identity == identity_a
        ));
        assert!(matches!(
            move_identity_attempt(B, &paste_dispatch.identity_guards, || panic!(
                "partial output must not trigger a manual Ctrl+V retry"
            )),
            MoveIdentityAttempt::Withheld(marker)
                if marker == MOVE_RETRY_IDENTITY_UNAVAILABLE_MARKER
        ));
        clip.cancel_cut(op);

        let (retry_uris, _retry_guards) = retry_transfer_capture(first_results.clone());
        assert_eq!(retry_uris, vec![A]);
        let retry_dispatch = clip.begin_cut_for(op, &retry_uris).unwrap();
        assert_eq!(retry_dispatch.uris, vec![A]);
        assert_eq!(
            retry_dispatch.identity_guards.get(A),
            Some(&MoveIdentityGuard::Stable(identity_a))
        );

        let retry_success = OperationItemResult::from_result(
            "operation-move",
            A.to_string(),
            Some("file:///tmp/destination".into()),
            Ok(()),
        );
        assert_eq!(clip.finish_cut(op, &[retry_success]), CutFinish::Partial);
        assert_eq!(clip.entries.as_ref().unwrap().uris, vec![B]);
        assert!(clip.entries.as_ref().unwrap().cut);
        assert!(matches!(
            clip.cut_identity_guards.get(B),
            Some(MoveIdentityGuard::Unavailable)
        ));
        assert!(!clip.cut_identity_guards.contains_key(A));
        assert!(clip.has_usable_entries(), "unretried cut item stays available");

        let remaining_dispatch = clip.begin_cut().unwrap();
        assert_eq!(remaining_dispatch.uris, vec![B]);
        assert!(matches!(
            move_identity_attempt(B, &remaining_dispatch.identity_guards, || panic!(
                "Ctrl+V must not retry B with partial output"
            )),
            MoveIdentityAttempt::Withheld(marker)
                if marker == MOVE_RETRY_IDENTITY_UNAVAILABLE_MARKER
        ));
        clip.cancel_cut(op);
    }

    #[test]
    fn unknown_cut_worker_outcome_marks_only_dispatched_uris_unavailable() {
        let identity_a = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "object-a".into(),
        };
        let identity_b = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "object-b".into(),
        };
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A, B], true));
        let op = clip.begin_cut().unwrap().generation;
        let results = [
            OperationItemResult::from_move_result(
                "operation-move",
                A.to_string(),
                "file:///tmp/destination".into(),
                Some(identity_a),
                Err(glib::Error::new(gio::IOErrorEnum::PermissionDenied, "retryable")),
            ),
            OperationItemResult::from_move_result(
                "operation-move",
                B.to_string(),
                "file:///tmp/destination".into(),
                Some(identity_b.clone()),
                Err(glib::Error::new(gio::IOErrorEnum::PermissionDenied, "retryable")),
            ),
        ];
        assert_eq!(clip.finish_cut(op, &results), CutFinish::Partial);

        let tracker = Rc::new(RefCell::new(clip));
        for failure in [
            crate::operation_queue::WorkerFailure::Panicked,
            crate::operation_queue::WorkerFailure::ChannelClosed,
        ] {
            let dispatch = tracker
                .borrow_mut()
                .begin_cut_for(op, &[A.to_string()])
                .unwrap();
            let guard = CutCompletionGuard::new(
                tracker.clone(),
                Some(op),
                dispatch.uris,
                Arc::new(AtomicBool::new(false)),
            );
            guard.worker_failed(&failure);
            drop(guard);
        }

        {
            let clip = tracker.borrow();
            assert_eq!(clip.entries.as_ref().unwrap().uris, vec![A, B]);
            assert_eq!(
                clip.cut_identity_guards.get(A),
                Some(&MoveIdentityGuard::Unavailable)
            );
            assert_eq!(
                clip.cut_identity_guards.get(B),
                Some(&MoveIdentityGuard::Stable(identity_b))
            );
        }
        let denied = tracker
            .borrow_mut()
            .begin_cut()
            .expect("unavailable source stays unresolved");
        assert!(matches!(
            move_identity_attempt(A, &denied.identity_guards, || panic!(
                "an unknown worker result must not capture a replacement identity"
            )),
            MoveIdentityAttempt::Withheld(marker)
                if marker == MOVE_RETRY_IDENTITY_UNAVAILABLE_MARKER
        ));
        tracker.borrow_mut().cancel_cut(op);
    }

    #[test]
    fn cut_worker_failure_before_start_preserves_prior_identity_guard() {
        let identity = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "original-object".into(),
        };
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        let op = clip.begin_cut().unwrap().generation;
        let failure = OperationItemResult::from_move_result(
            "operation-move",
            A.to_string(),
            "file:///tmp/destination".into(),
            Some(identity.clone()),
            Err(glib::Error::new(gio::IOErrorEnum::PermissionDenied, "retryable")),
        );
        assert_eq!(clip.finish_cut(op, &[failure]), CutFinish::Partial);

        let tracker = Rc::new(RefCell::new(clip));
        for failure in [
            crate::operation_queue::WorkerFailure::SpawnFailed,
            crate::operation_queue::WorkerFailure::QueueFull,
            crate::operation_queue::WorkerFailure::CancelledBeforeStart,
        ] {
            let dispatch = tracker
                .borrow_mut()
                .begin_cut()
                .expect("failed-before-start cut can be retried");
            assert_eq!(
                dispatch.identity_guards.get(A),
                Some(&MoveIdentityGuard::Stable(identity.clone()))
            );
            let guard = CutCompletionGuard::new(
                tracker.clone(),
                Some(op),
                dispatch.uris,
                Arc::new(AtomicBool::new(false)),
            );
            guard.worker_failed(&failure);
            drop(guard);
            assert_eq!(
                tracker.borrow().cut_identity_guards.get(A),
                Some(&MoveIdentityGuard::Stable(identity.clone()))
            );
        }
        assert!(!worker_failure_has_unknown_outcome(
            &crate::operation_queue::WorkerFailure::SpawnFailed
        ));
    }

    #[test]
    fn partial_cut_ctrl_v_stays_withheld_and_does_not_capture_identity() {
        let identity = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "original-object".into(),
        };
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        let op = clip.begin_cut().unwrap().generation;
        let partial = OperationItemResult::from_move_result(
            "operation-move",
            A.to_string(),
            "file:///tmp/destination".into(),
            Some(identity),
            Err(glib::Error::new(
                gio::IOErrorEnum::Cancelled,
                "cancelled; partial move output may remain at file:///tmp/destination/A",
            )),
        );
        assert!(!partial.needs_retry());
        assert_eq!(clip.finish_cut(op, &[partial]), CutFinish::Partial);
        assert!(matches!(
            clip.cut_identity_guards.get(A),
            Some(MoveIdentityGuard::Withheld(marker))
                if marker.contains("partial move output may remain at")
        ));

        let dispatch = clip.begin_cut().expect("withheld cut remains on clipboard");
        assert_eq!(dispatch.uris, vec![A]);
        assert!(matches!(
            dispatch.identity_guards.get(A),
            Some(MoveIdentityGuard::Withheld(marker))
                if marker.contains("partial move output may remain at")
        ));
        let blocked_marker = match move_identity_attempt(
            A,
            &dispatch.identity_guards,
            || panic!("withheld Ctrl+V must not query or upgrade source identity"),
        ) {
            MoveIdentityAttempt::Withheld(marker) => marker,
            MoveIdentityAttempt::Authorized(_) => panic!("partial move must stay withheld"),
        };
        assert!(blocked_marker.contains("partial move output may remain at"));
        let blocked_result = OperationItemResult::from_move_result(
            "operation-move",
            A.to_string(),
            "file:///tmp/destination".into(),
            None,
            Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                &blocked_marker,
            )),
        );
        assert_eq!(clip.finish_cut(op, &[blocked_result]), CutFinish::Partial);
        assert_eq!(clip.entries.as_ref().unwrap().uris, vec![A]);
        assert!(matches!(
            clip.cut_identity_guards.get(A),
            Some(MoveIdentityGuard::Withheld(marker)) if marker == &blocked_marker
        ));

        let (retry_uris, _) = retry_transfer_capture(vec![OperationItemResult::from_move_result(
            "operation-move",
            A.to_string(),
            "file:///tmp/destination".into(),
            Some(kito_core::FileIdentity {
                filesystem: "test-filesystem".into(),
                file: "original-object".into(),
            }),
            Err(glib::Error::new(
                gio::IOErrorEnum::Cancelled,
                "cancelled; partial move output may remain at file:///tmp/destination/A",
            )),
        )]);
        assert!(retry_uris.is_empty(), "manual retry omits partial moves");
    }

    #[test]
    fn identity_changed_cut_is_withheld_from_regular_paste() {
        let identity = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "original-object".into(),
        };
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        let op = clip.begin_cut().unwrap().generation;
        let changed = OperationItemResult::from_move_result(
            "operation-move",
            A.to_string(),
            "file:///tmp/destination".into(),
            Some(identity),
            Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                MOVE_RETRY_IDENTITY_CHANGED_MARKER,
            )),
        );

        assert!(!changed.needs_retry());
        assert_eq!(clip.finish_cut(op, &[changed]), CutFinish::Partial);
        assert!(matches!(
            clip.cut_identity_guards.get(A),
            Some(MoveIdentityGuard::Withheld(marker))
                if marker == MOVE_RETRY_IDENTITY_CHANGED_MARKER
        ));
        let dispatch = clip.begin_cut().expect("unresolved cut remains on clipboard");
        assert!(matches!(
            move_identity_attempt(A, &dispatch.identity_guards, || panic!(
                "identity-changed source must not be recaptured for Ctrl+V"
            )),
            MoveIdentityAttempt::Withheld(marker)
                if marker == MOVE_RETRY_IDENTITY_CHANGED_MARKER
        ));
        assert_eq!(clip.entries.as_ref().unwrap().uris, vec![A]);
    }

    #[test]
    fn failed_cut_ctrl_v_refuses_a_replacement_at_the_same_uri() {
        let temporary = tempfile::tempdir().unwrap();
        let source_path = temporary.path().join("source.txt");
        let old_source_path = temporary.path().join("old-source.txt");
        let destination_path = temporary.path().join("destination");
        std::fs::write(&source_path, b"original object").unwrap();
        std::fs::create_dir(&destination_path).unwrap();
        let source_uri = gio::File::for_path(&source_path).uri().to_string();
        let destination_uri = gio::File::for_path(&destination_path).uri().to_string();
        let original_identity = kito_core::file_identity(&source_uri).unwrap();

        let mut clip = ClipTracker::default();
        clip.publish(Clipboard {
            uris: vec![source_uri.clone()],
            cut: true,
        });
        let op = clip.begin_cut().unwrap().generation;
        let first_failure = OperationItemResult::from_move_result(
            "operation-move",
            source_uri.clone(),
            destination_uri.clone(),
            Some(original_identity.clone()),
            Err(glib::Error::new(gio::IOErrorEnum::PermissionDenied, "initial failure")),
        );
        assert_eq!(clip.finish_cut(op, &[first_failure]), CutFinish::Partial);

        // Keep the original inode alive elsewhere so the replacement cannot
        // accidentally reuse its identity.
        std::fs::rename(&source_path, &old_source_path).unwrap();
        std::fs::write(&source_path, b"replacement object").unwrap();
        let replacement_identity = kito_core::file_identity(&source_uri).unwrap();
        assert_ne!(replacement_identity, original_identity);

        let paste_dispatch = clip.begin_cut().unwrap();
        let expected = match move_identity_attempt(
            &source_uri,
            &paste_dispatch.identity_guards,
            || panic!("Ctrl+V must use the identity captured before the first failure"),
        ) {
            MoveIdentityAttempt::Authorized(Some(identity)) => identity,
            _ => panic!("stable failed-cut identity should authorize a guarded retry"),
        };
        let error = kito_core::move_to_exact_result_cancellable_if_identity(
            &source_uri,
            &destination_uri,
            &gio::Cancellable::new(),
            Some(&expected),
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains(MOVE_RETRY_IDENTITY_CHANGED_MARKER));
        assert_eq!(std::fs::read(&source_path).unwrap(), b"replacement object");
        assert_eq!(std::fs::read_dir(&destination_path).unwrap().count(), 0);
        clip.cancel_cut(op);
    }

    #[test]
    fn copy_retry_policy_distinguishes_missing_source_from_partial_output() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");
        std::fs::create_dir(&dest).unwrap();
        let missing = tmp.path().join("missing.txt");
        let missing_error = kito_core::copy_to_cancellable(
            gio::File::for_path(&missing).uri().as_str(),
            gio::File::for_path(&dest).uri().as_str(),
            &gio::Cancellable::new(),
            |_, _| {},
        )
        .unwrap_err();
        let missing_result = OperationItemResult::from_result(
            "operation-copy",
            gio::File::for_path(&missing).uri().to_string(),
            Some(gio::File::for_path(&dest).uri().to_string()),
            Err(missing_error),
        );
        assert!(missing_result.needs_retry());

        let partial = dest.join("partial.txt");
        std::fs::write(&partial, b"partial").unwrap();
        let partial_uri = gio::File::for_path(&partial).uri();
        let partial_error = glib::Error::new(
            gio::IOErrorEnum::Cancelled,
            &format!("cancelled; partial copy output may remain at {partial_uri}"),
        );
        let partial_result = OperationItemResult::from_result(
            "operation-copy",
            "file:///source/partial.txt".into(),
            Some(gio::File::for_path(&dest).uri().to_string()),
            Err(partial_error),
        );
        assert!(!partial_result.needs_retry());
    }

    #[test]
    fn move_retry_policy_requires_stable_identity_and_excludes_partial_output() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");
        std::fs::create_dir(&dest).unwrap();
        let missing = tmp.path().join("missing.txt");
        let missing_error = kito_core::move_to_exact_result_cancellable(
            gio::File::for_path(&missing).uri().as_str(),
            gio::File::for_path(&dest).uri().as_str(),
            &gio::Cancellable::new(),
        )
        .unwrap_err();
        let missing_result = OperationItemResult::from_result(
            "operation-move",
            gio::File::for_path(&missing).uri().to_string(),
            Some(gio::File::for_path(&dest).uri().to_string()),
            Err(missing_error),
        );
        assert!(missing_result.retry_identity.is_none());
        assert!(!missing_result.needs_retry());

        let identity = kito_core::FileIdentity {
            filesystem: "test-filesystem".into(),
            file: "stable-source".into(),
        };
        let retryable = OperationItemResult::from_move_result(
            "operation-move",
            "file:///source/item.txt".into(),
            "file:///tmp/dest".into(),
            Some(identity.clone()),
            Err(glib::Error::new(
                gio::IOErrorEnum::PermissionDenied,
                "target unavailable before mutation",
            )),
        );
        assert!(retryable.needs_retry());
        assert_eq!(retryable.retry_identity, Some(identity.clone()));

        let partial = dest.join("partial.txt");
        std::fs::write(&partial, b"partial").unwrap();
        let partial_uri = gio::File::for_path(&partial).uri();
        let partial_error = glib::Error::new(
            gio::IOErrorEnum::Cancelled,
            &format!("cancelled; partial move output may remain at {partial_uri}"),
        );
        let partial_result = OperationItemResult::from_move_result(
            "operation-move",
            "file:///source/partial.txt".into(),
            gio::File::for_path(&dest).uri().to_string(),
            Some(identity.clone()),
            Err(partial_error),
        );
        assert!(!partial_result.needs_retry());
        assert!(partial.exists(), "partial output is reported, never deleted");

        let changed = OperationItemResult::from_move_result(
            "operation-move",
            "file:///source/item.txt".into(),
            "file:///tmp/dest".into(),
            Some(identity),
            Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                MOVE_RETRY_IDENTITY_CHANGED_MARKER,
            )),
        );
        assert!(!changed.needs_retry());
    }

    #[test]
    fn partial_move_keeps_cut_clipboard_without_becoming_retryable() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        let op = clip.begin_cut().unwrap().generation;
        let partial = OperationItemResult::from_result(
            "operation-move",
            A.to_string(),
            Some("file:///tmp/dest".into()),
            Err(glib::Error::new(
                gio::IOErrorEnum::Cancelled,
                "partial move output may remain at file:///tmp/dest/a",
            )),
        );

        assert!(!partial.needs_retry());
        assert_eq!(clip.finish_cut(op, &[partial]), CutFinish::Partial);
        assert!(clip.has_usable_entries());
    }

    #[test]
    fn failed_cut_retains_move_without_identity_even_when_retry_is_withheld() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        let op = clip.begin_cut().unwrap().generation;
        let failed = OperationItemResult::from_result(
            "operation-move",
            A.to_string(),
            Some("file:///tmp/dest".into()),
            Err(glib::Error::new(
                gio::IOErrorEnum::PermissionDenied,
                "stable identity unavailable",
            )),
        );

        assert!(!failed.needs_retry());
        assert_eq!(failed_cut_uris(&[failed.clone()]), vec![A]);
        assert_eq!(clip.finish_cut(op, &[failed.clone()]), CutFinish::Partial);
        assert!(clip.has_usable_entries());
        assert_eq!(retry_transfer_capture(vec![failed]).0, Vec::<String>::new());
    }

    #[test]
    fn operation_cancellation_uses_the_gio_error_code() {
        let cancelled = OperationItemResult::from_result(
            "operation-copy",
            A.to_string(),
            Some("file:///tmp/dest".into()),
            Err(glib::Error::new(
                gio::IOErrorEnum::Cancelled,
                "localized message does not determine the result",
            )),
        );
        assert!(matches!(cancelled.state, ItemState::Cancelled(_)));
        assert!(cancelled.needs_retry());

        let failed = OperationItemResult::from_result(
            "operation-copy",
            B.to_string(),
            Some("file:///tmp/dest".into()),
            Err(glib::Error::new(
                gio::IOErrorEnum::PermissionDenied,
                "cancel was mentioned in the technical detail only",
            )),
        );
        assert!(matches!(failed.state, ItemState::Failed(_)));
    }

    #[test]
    fn new_publish_during_move_stays_intact() {
        let mut clip = ClipTracker::default();
        let published_a = clip.publish(entries(&[A], true));
        let op = clip.begin_cut().expect("first move dispatches").generation;
        // New copy while the old move runs: independent state.
        let published_b = clip.publish(entries(&[B], false));
        clip.on_own_changed();
        // Old completion touches nothing (not even with full success).
        assert_eq!(clip.finish_cut(op, &[]), CutFinish::Untouched);
        assert_eq!(
            clip.resolve(Some(&published_b), clip.generation),
            PasteDecision::Internal {
                uris: vec![B.to_string()],
                cut: false,
            }
        );
        assert_ne!(published_a, published_b);
    }

    #[test]
    fn external_replace_during_move_is_not_restored() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        let op = clip.begin_cut().expect("first move dispatches").generation;
        clip.on_external_changed();
        assert_eq!(clip.finish_cut(op, &[]), CutFinish::Untouched);
        // Old cut is gone for good: foreign content pastes as copy.
        assert_eq!(
            clip.resolve(Some(B), clip.generation),
            PasteDecision::External {
                uris: vec![B.to_string()]
            }
        );
    }

    #[test]
    fn repeated_paste_during_move_dispatches_once() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        clip.on_own_changed();
        let op = clip.begin_cut().expect("first move dispatches").generation;
        // Second paste of the same cut: refused, no duplicate move.
        assert_eq!(clip.begin_cut(), None);
        // After completion the guard is released.
        assert_eq!(finish_cut_with_failed_uris(&mut clip, op, &[]), CutFinish::Consumed);
        clip.publish(entries(&[A], true));
        assert!(clip.begin_cut().is_some());
    }

    #[test]
    fn external_replace_invalidates_pending_cut() {
        let mut clip = ClipTracker::default();
        clip.publish(entries(&[A], true));
        clip.on_own_changed();
        clip.on_external_changed();
        // Old cut is gone: B pastes as copy, never as move.
        assert_eq!(
            clip.resolve(Some(B), clip.generation),
            PasteDecision::External {
                uris: vec![B.to_string()]
            }
        );
    }

    #[test]
    fn own_publish_notification_keeps_state_valid() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A], false));
        // Notification caused by our own publish: still valid...
        clip.on_own_changed();
        assert_eq!(
            clip.resolve(Some(&published), clip.generation),
            PasteDecision::Internal {
                uris: vec![A.to_string()],
                cut: false,
            }
        );
        // ...even if delivered twice.
        clip.on_own_changed();
        assert_eq!(
            clip.resolve(Some(&published), clip.generation),
            PasteDecision::Internal {
                uris: vec![A.to_string()],
                cut: false,
            }
        );
    }

    #[test]
    fn async_read_after_change_is_discarded() {
        let mut clip = ClipTracker::default();
        let published = clip.publish(entries(&[A], false));
        let read_generation = clip.generation;
        // Clipboard replaced while the async read was in flight.
        clip.on_external_changed();
        assert_eq!(
            clip.resolve(Some(&published), read_generation),
            PasteDecision::Stale
        );
        // A fresh read at the current generation resolves normally.
        assert_eq!(
            clip.resolve(Some(B), clip.generation),
            PasteDecision::External {
                uris: vec![B.to_string()]
            }
        );
    }

    #[test]
    fn external_uris_filter_file_lines() {
        assert_eq!(
            parse_external_uris("  file:///tmp/a.txt\nnot a file\nfile:///tmp/b.txt  "),
            vec![
                "file:///tmp/a.txt".to_string(),
                "file:///tmp/b.txt".to_string()
            ]
        );
        assert!(parse_external_uris("plain text\n").is_empty());
    }
}
