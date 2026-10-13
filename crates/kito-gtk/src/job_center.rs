//! F08 — operation job registry and Operations view.
//!
//! The registry wraps the existing application-wide [`OperationQueue`]:
//! every submitted filesystem mutation becomes a job with a stable ID,
//! sources/destination captured at enqueue time, cooperative cancellation
//! and measured progress. Per-element outcomes stay in `ops.rs` (B03);
//! the registry aggregates them into the job state shown in the
//! Operations window.
//!
//! No GTK widgets on worker threads: the worker reports progress through
//! atomics read by the GTK thread, and cancellation works by cancelling
//! the GIO `Cancellable` the running worker passes to every cancellable
//! GIO call. Queued jobs are removed before they start. When a total is
//! unknown, the progress stays indeterminate instead of inventing a
//! percentage. Retrying failed items starts a new job; the old one keeps
//! its per-element records.

use adw::prelude::*;
use gtk::{gio, glib};
use std::{
    cell::Cell,
    collections::HashSet,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};

/// Ordered job lifecycle. `partial` marks a job with at least one success
/// and one failed or cancelled element; a job with failed or cancelled
/// items is never reported as plain success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JobStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Partial,
    Failed,
    Cancelled,
}

impl JobStatus {
    fn code(self) -> u8 {
        match self {
            JobStatus::Queued => 0,
            JobStatus::Running => 1,
            JobStatus::Cancelling => 2,
            JobStatus::Completed => 3,
            JobStatus::Partial => 4,
            JobStatus::Failed => 5,
            JobStatus::Cancelled => 6,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            0 => JobStatus::Queued,
            1 => JobStatus::Running,
            2 => JobStatus::Cancelling,
            3 => JobStatus::Completed,
            4 => JobStatus::Partial,
            5 => JobStatus::Failed,
            _ => JobStatus::Cancelled,
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            JobStatus::Queued => "jobs-status-queued",
            JobStatus::Running => "jobs-status-running",
            JobStatus::Cancelling => "jobs-status-cancelling",
            JobStatus::Completed => "jobs-status-completed",
            JobStatus::Partial => "jobs-status-partial",
            JobStatus::Failed => "jobs-status-failed",
            JobStatus::Cancelled => "jobs-status-cancelled",
        }
    }

    /// User-facing classification of a finished job.
    fn finish(succeeded: usize, failed: usize, cancelled: usize) -> Self {
        if attempted(succeeded, failed, cancelled) == 0 {
            return JobStatus::Completed;
        }
        if succeeded == 0 {
            if failed > 0 {
                JobStatus::Failed
            } else {
                JobStatus::Cancelled
            }
        } else if failed > 0 || cancelled > 0 {
            JobStatus::Partial
        } else {
            JobStatus::Completed
        }
    }

    fn is_active(self) -> bool {
        matches!(
            self,
            JobStatus::Queued | JobStatus::Running | JobStatus::Cancelling
        )
    }
}

fn attempted(succeeded: usize, failed: usize, cancelled: usize) -> usize {
    succeeded + failed + cancelled
}

/// Per-element outcome retained for the job summary.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JobItemState {
    Succeeded,
    Failed(String),
    Cancelled(String),
}

/// Thread-safe progress snapshot shared between worker and GTK thread.
/// `has_total == false` keeps the bar indeterminate; when a total is known
/// (captured sources count or reported totals) the fraction uses really
/// attempted items. Current-file byte volume reports through GIO's copy
/// progress callback only.
#[derive(Debug, Default)]
struct ProgressShared {
    attempted: AtomicU64,
    files_total: AtomicU64,
    has_total: AtomicBool,
    bytes_current: AtomicU64,
    bytes_current_total: AtomicU64,
}

/// One operation job shown in the Operations window. The same `Arc<Job>`
/// reaches the worker thread (Send via atomics/Mutex only) and the GTK
/// thread, so progress stays consistent without channels.
pub(crate) struct Job {
    pub(crate) id: u64,
    /// Fluent operation key, e.g. `operation-copy`.
    pub(crate) operation: &'static str,
    /// Sources captured at enqueue time; later tab/selection changes never
    /// mutate this.
    pub(crate) sources: Vec<String>,
    /// Destination captured at enqueue time.
    pub(crate) destination: Option<String>,
    status: AtomicU8,
    progress: ProgressShared,
    items: Mutex<Vec<(String, JobItemState)>>,
    /// GIO cancellable shared with the running worker.
    cancellable: Option<gio::Cancellable>,
}

impl Job {
    pub(crate) fn status(&self) -> JobStatus {
        JobStatus::from_code(self.status.load(Ordering::Acquire))
    }

    fn set_status(&self, status: JobStatus) {
        self.status.store(status.code(), Ordering::Release);
    }

    pub(crate) fn cancellable(&self) -> Option<&gio::Cancellable> {
        self.cancellable.as_ref()
    }

    /// Worker-side start: marks running and binds the batch total when the
    /// caller knows it (number of captured top-level sources), never a
    /// recursive-scan estimate.
    pub(crate) fn start(&self, total_known: Option<u64>) -> bool {
        if self.cancellable().is_some_and(gio::Cancellable::is_cancelled) {
            return false;
        }
        match self.status() {
            JobStatus::Queued => {
                if self
                    .status
                    .compare_exchange(
                        JobStatus::Queued.code(),
                        JobStatus::Running.code(),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_err()
                {
                    return false;
                }
            }
            JobStatus::Running => {}
            _ => return false,
        }
        if self.cancellable().is_some_and(gio::Cancellable::is_cancelled) {
            let _ = self.status.compare_exchange(
                JobStatus::Running.code(),
                JobStatus::Cancelling.code(),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            return false;
        }
        if let Some(total) = total_known {
            self.progress.files_total.store(total, Ordering::Release);
            self.progress.has_total.store(true, Ordering::Release);
        }
        self.reset_current_file();
        true
    }

    /// Worker-side completion of one element: counts as really attempted.
    pub(crate) fn record_attempt(&self) {
        self.progress.attempted.fetch_add(1, Ordering::AcqRel);
        self.reset_current_file();
    }

    fn reset_current_file(&self) {
        self.progress.bytes_current.store(0, Ordering::Release);
        self.progress.bytes_current_total.store(0, Ordering::Release);
    }

    /// Worker-side current-file progress from the GIO copy callback.
    /// Measured bytes only; negative or unknown totals stay indeterminate.
    pub(crate) fn report_current_file(&self, bytes_current: u64, bytes_current_total: i64) {
        self.progress
            .bytes_current
            .store(bytes_current.min(u64::MAX / 256), Ordering::Release);
        self.progress.bytes_current_total.store(
            bytes_current_total.clamp(0, i64::MAX / 256) as u64,
            Ordering::Release,
        );
    }

    fn count_items(&self) -> (usize, usize, usize) {
        let items = self.items.lock().unwrap();
        let mut succeeded = 0;
        let mut failed = 0;
        let mut cancelled = 0;
        for (_, state) in items.iter() {
            match state {
                JobItemState::Succeeded => succeeded += 1,
                JobItemState::Failed(_) => failed += 1,
                JobItemState::Cancelled(_) => cancelled += 1,
            }
        }
        (succeeded, failed, cancelled)
    }

    /// Attempted items for the progress bar: real element count, not a
    /// recursive scan estimate.
    fn attempted(&self) -> u64 {
        let (succeeded, failed, _) = self.count_items();
        ((succeeded + failed) as u64)
            .max(self.progress.attempted.load(Ordering::Acquire))
    }

    fn load_fraction(&self) -> Option<(f64, u64, u64)> {
        let has_total = self.progress.has_total.load(Ordering::Acquire);
        if !has_total {
            return None;
        }
        let total = self.progress.files_total.load(Ordering::Acquire);
        if total == 0 {
            return None;
        }
        let done = self.attempted().min(total);
        let bytes = self.progress.bytes_current.load(Ordering::Acquire);
        let bytes_total = self.progress.bytes_current_total.load(Ordering::Acquire);
        let fraction = if bytes_total > 0 && done < total {
            (done as f64 + bytes.min(bytes_total) as f64 / bytes_total as f64) / total as f64
        } else {
            done as f64 / total as f64
        };
        Some((fraction, done, total))
    }

    pub(crate) fn items_summary(&self) -> String {
        let items = self.items.lock().unwrap();
        items
            .iter()
            .map(|(source, state)| {
                let shown = kito_core::uri_to_display(source);
                match state {
                    JobItemState::Succeeded => {
                        crate::l10n::tr_with_one("jobs-item-succeeded", "source", &shown)
                    }
                    JobItemState::Failed(error) => {
                        let mut args = kito_i18n::FluentArgs::new();
                        args.set("source", shown.as_str());
                        args.set("error", error.as_str());
                        crate::l10n::tr_with("jobs-item-failed", &args)
                    }
                    JobItemState::Cancelled(error) => {
                        let mut args = kito_i18n::FluentArgs::new();
                        args.set("source", shown.as_str());
                        args.set("error", error.as_str());
                        crate::l10n::tr_with("jobs-item-cancelled", &args)
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Shared registry: one instance per application, reachable from every
/// window. GTK thread only (the widget side is guarded by the main
/// context); job internals carry the thread-safe parts.
#[derive(Default)]
pub(crate) struct JobCenter {
    next_id: Cell<u64>,
    jobs: std::cell::RefCell<Vec<Arc<Job>>>,
    /// Weak handles of live Operations dialogs, refreshed on changes.
    presenters: std::cell::RefCell<Vec<glib::WeakRef<adw::Dialog>>>,
}

impl JobCenter {
    const TERMINAL_HISTORY_LIMIT: usize = 100;
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Registers a new job and returns its handle plus the cancellable the
    /// worker must propagate to every cancellable GIO call.
    pub(crate) fn begin_job(
        &self,
        operation: &'static str,
        sources: Vec<String>,
        destination: Option<String>,
    ) -> (Arc<Job>, gio::Cancellable) {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let job = Arc::new(Job {
            id,
            operation,
            sources,
            destination,
            status: AtomicU8::new(JobStatus::Queued.code()),
            progress: ProgressShared::default(),
            items: Mutex::new(Vec::new()),
            cancellable: Some(gio::Cancellable::new()),
        });
        self.jobs.borrow_mut().push(job.clone());
        self.prune_history();
        self.notify_changed();
        let cancellable = job.cancellable.clone().unwrap();
        (job, cancellable)
    }

    fn job(&self, job_id: u64) -> Option<Arc<Job>> {
        self.jobs.borrow().iter().find(|job| job.id == job_id).cloned()
    }

    /// Cancels queued work without losing its visible registry entry. The
    /// queue observes the cancelled GIO token before running the worker.
    pub(crate) fn cancel_queued(&self, job_id: u64) -> bool {
        let Some(job) = self.job(job_id) else {
            return false;
        };
        if job.status() != JobStatus::Queued {
            return false;
        }
        job.set_status(JobStatus::Cancelling);
        if let Some(cancellable) = job.cancellable() {
            cancellable.cancel();
        }
        self.notify_changed();
        true
    }

    /// Cooperative cancellation of a running job: marks `cancelling` and
    /// cancels the GIO cancellable the worker observes at the next
    /// cancellable GIO call. Completed elements are already recorded and
    /// stay registered.
    pub(crate) fn cancel_running(&self, job_id: u64) -> bool {
        let Some(job) = self.job(job_id) else {
            return false;
        };
        if job.status() != JobStatus::Running {
            return false;
        }
        job.set_status(JobStatus::Cancelling);
        if let Some(cancellable) = job.cancellable() {
            cancellable.cancel();
        }
        self.notify_changed();
        true
    }

    /// One command for the UI button: cancel queued work before it starts,
    /// otherwise request cooperative cancellation of the running job.
    pub(crate) fn cancel(&self, job_id: u64) -> bool {
        self.cancel_queued(job_id) || self.cancel_running(job_id)
    }

    /// Finalizes the job from the per-element outcomes collected by the
    /// operation engine (see `ops.rs`). Classifies Completed/Partial/
    /// Failed/Cancelled and never promotes a mixed result to success.
    pub(crate) fn finish_items(
        self: &Rc<Self>,
        job: &Arc<Job>,
        results: Vec<(String, JobItemState)>,
    ) {
        {
            let mut items = job.items.lock().unwrap();
            items.extend(results);
        }
        let (succeeded, failed, cancelled) = job.count_items();
        let total = attempted(succeeded, failed, cancelled);
        job.progress
            .attempted
            .fetch_max((succeeded + failed) as u64, Ordering::AcqRel);
        if !job.progress.has_total.load(Ordering::Acquire) && total > 0 {
            job.progress.files_total.store(total as u64, Ordering::Release);
            job.progress.has_total.store(true, Ordering::Release);
        }
        job.reset_current_file();
        job.set_status(JobStatus::finish(succeeded, failed, cancelled));
        self.prune_history();
        self.notify_changed();
    }

    /// Marks a job failed as a whole (worker panic, spawn failure): the
    /// filesystem outcome is unknown, so no per-element success is implied.
    /// Returns false when another completion already gave it a terminal state.
    pub(crate) fn finish_worker_failure(&self, job: &Arc<Job>) -> bool {
        let mut observed = job.status.load(Ordering::Acquire);
        loop {
            if !JobStatus::from_code(observed).is_active() {
                return false;
            }
            match job.status.compare_exchange(
                observed,
                JobStatus::Failed.code(),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => observed = actual,
            }
        }
        self.prune_history();
        self.notify_changed();
        true
    }

    fn prune_history(&self) {
        let mut jobs = self.jobs.borrow_mut();
        let mut terminal = jobs.iter().filter(|job| !job.status().is_active()).count();
        if terminal > Self::TERMINAL_HISTORY_LIMIT {
            jobs.retain(|job| {
                if job.status().is_active() { return true; }
                if terminal > Self::TERMINAL_HISTORY_LIMIT {
                    terminal -= 1;
                    return false;
                }
                true
            });
        }
    }

    /// Live jobs snapshot for tests and the dialog.
    pub(crate) fn jobs(&self) -> Vec<Arc<Job>> {
        self.jobs.borrow().clone()
    }

    fn notify_changed(&self) {
        let presenters = self.presenters.borrow_mut();
        for weak in presenters.iter() {
            if let Some(dialog) = weak.upgrade() {
                let _ = gtk::prelude::WidgetExt::activate_action(&dialog, "jobs.refresh", None);
            }
        }
    }

    /// Keeps the weak presenter list after the dialog opened.
    fn track_dialog(&self, dialog: adw::Dialog) {
        let weak = dialog.downgrade();
        self.presenters.borrow_mut().push(weak);
        self.presenters
            .borrow_mut()
            .retain(|weak| weak.upgrade().is_some());
    }

    /// Opens (or reparents) the Operations dialog on this window. Several
    /// windows share the same registry; each can present its own dialog.
    pub(crate) fn present_dialog(self: &Rc<Self>, parent: &adw::ApplicationWindow) {
        let dialog = build_operations_dialog(self);
        self.track_dialog(dialog.clone());
        dialog.present(Some(parent));
    }
}

/// Fails a job if its completion callback exits while it is still active.
/// The callback may panic before its operation-specific `finish_items`; a
/// terminal status already recorded by that callback is left untouched.
pub(crate) struct JobCompletionGuard {
    center: Rc<JobCenter>,
    job: Arc<Job>,
}

impl JobCompletionGuard {
    pub(crate) fn new(center: Rc<JobCenter>, job: Arc<Job>) -> Self {
        Self { center, job }
    }
}

impl Drop for JobCompletionGuard {
    fn drop(&mut self) {
        // Drop can run during callback unwinding. Catch any panic from history
        // bookkeeping or presenter notification to avoid a double panic; the
        // status CAS happens first, so the job is still failed if that occurs.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if self.job.status().is_active() {
                self.center.finish_worker_failure(&self.job);
            }
        }));
    }
}

/// Builds the Operations dialog listing every job with live status,
/// sources/destination, measurable progress, cancel command and per-item
/// details. Rebuilt on each change notification while open.
fn build_operations_dialog(center: &Rc<JobCenter>) -> adw::Dialog {
    let dialog = adw::Dialog::builder()
        .title(crate::l10n::tr("jobs-title"))
        .content_width(560)
        .content_height(420)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(16)
        .margin_end(16)
        .margin_top(16)
        .margin_bottom(16)
        .build();
    let jobs_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&jobs_box)
        .vexpand(true)
        .build();
    content.append(&scroll);

    let close = gtk::Button::builder()
        .label(crate::l10n::tr("operation-close"))
        .halign(gtk::Align::End)
        .css_classes(["suggested-action"])
        .build();
    close.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(dialog) = dialog.upgrade() { dialog.close(); }
        }
    });
    content.append(&close);

    let action_group = gio::SimpleActionGroup::new();
    let refresh = gio::SimpleAction::new("refresh", None);
    let expanded = Rc::new(std::cell::RefCell::new(HashSet::new()));
    refresh.connect_activate({
        let center = Rc::downgrade(center);
        let jobs_box = jobs_box.downgrade();
        let dialog = dialog.downgrade();
        let expanded = expanded.clone();
        move |_, _| {
            if let (Some(center), Some(jobs_box), Some(dialog)) =
                (center.upgrade(), jobs_box.upgrade(), dialog.upgrade())
            {
                rebuild_jobs(&center, &jobs_box, &dialog, &expanded)
            }
        }
    });
    action_group.add_action(&refresh);
    dialog.insert_action_group("jobs", Some(&action_group));
    dialog.set_child(Some(&content));

    // Live view: periodic rebuild while the dialog is open. The registry
    // terminal history is bounded, so a rebuild per tick is
    // cheap and avoids cross-thread widget notifications.
    glib::timeout_add_local(std::time::Duration::from_millis(400), {
        let center = Rc::downgrade(center);
        let jobs_box = jobs_box.downgrade();
        let dialog = dialog.downgrade();
        let expanded = expanded.clone();
        move || {
            let (Some(dialog), Some(jobs_box), Some(center)) =
                (dialog.upgrade(), jobs_box.upgrade(), center.upgrade())
            else {
                return glib::ControlFlow::Break;
            };
            if !dialog.is_visible() {
                return glib::ControlFlow::Break;
            }
            rebuild_jobs(&center, &jobs_box, &dialog, &expanded);
            glib::ControlFlow::Continue
        }
    });
    rebuild_jobs(center, &jobs_box, &dialog, &expanded);
    dialog
}

fn rebuild_jobs(
    center: &Rc<JobCenter>,
    jobs_box: &gtk::Box,
    dialog: &adw::Dialog,
    expanded: &Rc<std::cell::RefCell<HashSet<u64>>>,
) {
    while let Some(child) = jobs_box.first_child() {
        jobs_box.remove(&child);
    }
    for job in center.jobs() {
        let card = job_card(center, &job, dialog, expanded);
        jobs_box.append(&card);
    }
}

/// One job card: operation name + localized status, sources → destination,
/// a progress bar (determinate only when a real total exists; unknown
/// totals stay indeterminate), a Cancel command for active jobs and an
/// expandable per-item detail list.
fn job_card(
    center: &Rc<JobCenter>,
    job: &Arc<Job>,
    dialog: &adw::Dialog,
    expanded: &Rc<std::cell::RefCell<HashSet<u64>>>,
) -> gtk::Box {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(4)
        .margin_bottom(4)
        .build();

    let status = job.status();
    let heading = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    heading.append(
        &gtk::Label::builder()
            .label(&crate::l10n::tr(job.operation))
            .halign(gtk::Align::Start)
            .hexpand(true)
            .css_classes(["heading"])
            .build(),
    );
    heading.append(
        &gtk::Label::builder()
            .label(&crate::l10n::tr(status.label_key()))
            .halign(gtk::Align::End)
            .build(),
    );
    if status.is_active() {
        let spinner = gtk::Spinner::new();
        spinner.start();
        heading.append(&spinner);
    }
    card.append(&heading);

    let sources = job
        .sources
        .iter()
        .map(|uri| kito_core::uri_to_display(uri))
        .collect::<Vec<_>>()
        .join(", ");
    card.append(
        &gtk::Label::builder()
            .label(&sources)
            .halign(gtk::Align::Start)
            .wrap(true)
            .build(),
    );
    if let Some(destination) = &job.destination {
        card.append(
            &gtk::Label::builder()
                .label(&crate::l10n::tr_with_one(
                    "jobs-destination",
                    "destination",
                    &kito_core::uri_to_display(destination),
                ))
                .halign(gtk::Align::Start)
                .wrap(true)
                .build(),
        );
    }

    // Progress: deterministic only with a real measurable fraction. A known
    // batch total yields "attempted of total"; a known current-file size
    // refines it only with GIO-measured data, never invented scans.
    let progress = gtk::ProgressBar::new();
    match job.load_fraction() {
        Some((fraction, done, total)) => {
            progress.set_show_text(true);
            let mut args = kito_i18n::FluentArgs::new();
            args.set("done", done);
            args.set("total", total);
            progress.set_text(Some(&crate::l10n::tr_with("jobs-progress-files", &args)));
            progress.set_fraction(fraction.clamp(0.0, 1.0));
        }
        None => {
            progress.set_show_text(false);
            if status.is_active() {
                glib::timeout_add_local(std::time::Duration::from_millis(150), {
                    let job_weak = WeakRefHolder::new(job.clone());
                    let progress = progress.downgrade();
                    move || {
                        let (Some(bar), Some(job)) = (progress.upgrade(), job_weak.get())
                        else {
                            return glib::ControlFlow::Break;
                        };
                        if !job.status().is_active() {
                            bar.set_fraction(1.0);
                            return glib::ControlFlow::Break;
                        }
                        bar.pulse();
                        glib::ControlFlow::Continue
                    }
                });
            }
        }
    }
    card.append(&progress);

    if status.is_active() {
        let cancel = gtk::Button::builder()
            .label(crate::l10n::tr(if status == JobStatus::Cancelling {
                "jobs-cancelling"
            } else {
                "jobs-cancel"
            }))
            .halign(gtk::Align::Start)
            .build();
        cancel.set_sensitive(status != JobStatus::Cancelling);
        cancel.connect_clicked({
            let center = center.clone();
            let job_id = job.id;
            let dialog = dialog.downgrade();
            move |_| {
                center.cancel(job_id);
                if let Some(dialog) = dialog.upgrade() {
                    let _ = gtk::prelude::WidgetExt::activate_action(&dialog, "jobs.refresh", None);
                }
            }
        });
        card.append(&cancel);
    }

    let summary = job.items_summary();
    if !summary.is_empty() {
        let details_label = gtk::Label::builder()
            .label(&summary)
            .halign(gtk::Align::Start)
            .selectable(true)
            .wrap(true)
            .build();
        let details = gtk::Expander::builder()
            .label(crate::l10n::tr("operation-details"))
            .expanded(expanded.borrow().contains(&job.id))
            .child(&details_label)
            .build();
        details.connect_expanded_notify({
            let expanded = expanded.clone();
            let job_id = job.id;
            move |expander| {
                if expander.is_expanded() {
                    expanded.borrow_mut().insert(job_id);
                } else {
                    expanded.borrow_mut().remove(&job_id);
                }
            }
        });
        card.append(&details);
    }
    card
}

/// Keeps a plain `Arc` alive across a GTK timeout without going through
/// glib WeakRef (which requires IsA<Object> types).
#[derive(Clone)]
struct WeakRefHolder<T>(Rc<std::cell::RefCell<Option<Arc<T>>>>);

impl<T> WeakRefHolder<T> {
    fn new(job: Arc<T>) -> Self {
        Self(Rc::new(std::cell::RefCell::new(Some(job))))
    }

    fn get(&self) -> Option<Arc<T>> {
        self.0.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("file:///src-{i}")).collect()
    }

    /// `finish` classifies without ever promoting cancelled or failed
    /// elements to plain success.
    #[test]
    fn job_finish_states_never_call_mixed_results_success() {
        assert_eq!(JobStatus::finish(3, 0, 0), JobStatus::Completed);
        assert_eq!(JobStatus::finish(0, 2, 0), JobStatus::Failed);
        assert_eq!(JobStatus::finish(0, 0, 2), JobStatus::Cancelled);
        assert_eq!(JobStatus::finish(2, 1, 0), JobStatus::Partial);
        assert_eq!(JobStatus::finish(2, 0, 1), JobStatus::Partial);
        assert_eq!(JobStatus::finish(1, 1, 1), JobStatus::Partial);
    }

    #[test]
    fn completion_panic_fails_active_job_and_guard_preserves_terminal_status() {
        let center = JobCenter::new();
        let (job, _) = center.begin_job("operation-copy", sources(1), None);
        assert!(job.start(Some(1)));

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let center = center.clone();
            let job = job.clone();
            move || {
                let _guard = JobCompletionGuard::new(center, job);
                panic!("simulated completion callback panic");
            }
        }));
        assert!(panic.is_err());
        assert_eq!(job.status(), JobStatus::Failed);

        let (completed, _) = center.begin_job("operation-copy", sources(1), None);
        let guard = JobCompletionGuard::new(center.clone(), completed.clone());
        center.finish_items(
            &completed,
            vec![(completed.sources[0].clone(), JobItemState::Succeeded)],
        );
        drop(guard);
        assert_eq!(completed.status(), JobStatus::Completed);

        let (partial, _) = center.begin_job("operation-copy", sources(2), None);
        let guard = JobCompletionGuard::new(center.clone(), partial.clone());
        center.finish_items(
            &partial,
            vec![
                (partial.sources[0].clone(), JobItemState::Succeeded),
                (
                    partial.sources[1].clone(),
                    JobItemState::Failed("incomplete".into()),
                ),
            ],
        );
        drop(guard);
        assert_eq!(partial.status(), JobStatus::Partial);

        let (cancelled, _) = center.begin_job("operation-copy", sources(1), None);
        let guard = JobCompletionGuard::new(center.clone(), cancelled.clone());
        center.finish_items(
            &cancelled,
            vec![(
                cancelled.sources[0].clone(),
                JobItemState::Cancelled("stopped".into()),
            )],
        );
        drop(guard);
        assert_eq!(cancelled.status(), JobStatus::Cancelled);

        let (worker_failed, _) = center.begin_job("operation-copy", sources(1), None);
        assert!(center.finish_worker_failure(&worker_failed));
        let guard = JobCompletionGuard::new(center.clone(), worker_failed.clone());
        assert!(!center.finish_worker_failure(&worker_failed));
        drop(guard);
        assert_eq!(worker_failed.status(), JobStatus::Failed);
    }

    /// Queue removal touches only queued jobs: a running job survives a
    /// stale cancel of its ID and is cancellable cooperatively through a
    /// real GIO cancellable.
    #[test]
    fn queued_cancel_stays_registered_and_running_cancel_propagates_gio_cancellation() {
        let center = JobCenter::new();
        let (queued, queued_cancellable) = center.begin_job("operation-copy", sources(2), None);
        assert_eq!(queued.status(), JobStatus::Queued);
        assert!(center.cancel_queued(queued.id));
        assert_eq!(center.jobs().len(), 1);
        assert_eq!(queued.status(), JobStatus::Cancelling);
        assert!(queued_cancellable.is_cancelled());
        assert!(!queued.start(Some(2)));
        assert_eq!(queued.status(), JobStatus::Cancelling);
        assert!(!center.cancel_queued(queued.id));
        center.finish_items(
            &queued,
            queued.sources.iter().map(|source| (
                source.clone(), JobItemState::Cancelled("before start".into()),
            )).collect(),
        );
        assert_eq!(queued.status(), JobStatus::Cancelled);

        let (running, cancellable) = center.begin_job("operation-copy", sources(1), None);
        running.start(Some(1));
        assert_eq!(running.status(), JobStatus::Running);
        assert!(!center.cancel_queued(running.id));
        assert!(center.cancel_running(running.id));
        assert_eq!(running.status(), JobStatus::Cancelling);
        assert!(cancellable.is_cancelled());

        // Cancellation keeps recorded items and classifies the remainder.
        center.finish_items(
            &running,
            vec![
                ("file:///src-0".into(), JobItemState::Succeeded),
                ("file:///src-1".into(), JobItemState::Cancelled("stopped".into())),
            ],
        );
        assert_eq!(running.status(), JobStatus::Partial);
        assert_eq!(running.count_items(), (1, 0, 1));
    }

    /// The captured sources/destination outlive tab and selection changes:
    /// the registry holds exactly what it was given at enqueue time, and
    /// the shared handle never mutates it through `&Arc<Job>`.
    #[test]
    fn captured_context_is_immutable_after_enqueue() {
        let center = JobCenter::new();
        let snapshot = {
            let (job, _) =
                center.begin_job("operation-move", sources(3), Some("file:///dest".into()));
            (job.sources.clone(), job.destination.clone(), job.id)
        };
        let (fresh, _) = center.begin_job("operation-move", sources(3), None);
        assert_eq!(fresh.sources.len(), 3);
        assert_ne!(fresh.id, snapshot.2);
        assert_eq!(snapshot.0.len(), 3);
        assert_eq!(snapshot.1.as_deref(), Some("file:///dest"));
    }

    /// Job IDs are stable and strictly increasing: retry and cancel target
    /// the ENQUEUED job, never a re-created one.
    #[test]
    fn job_ids_are_stable_and_ordered() {
        let center = JobCenter::new();
        let first = center.begin_job("operation-copy", sources(1), None).0;
        let second = center.begin_job("operation-copy", sources(1), None).0;
        assert!(second.id > first.id);
        assert_eq!(
            center.jobs().iter().map(|job| job.id).collect::<Vec<_>>(),
            vec![first.id, second.id]
        );
    }

    #[test]
    fn terminal_history_is_capped_without_evicting_active_jobs() {
        let center = JobCenter::new();
        let (active, _) = center.begin_job("operation-copy", sources(1), None);
        for _ in 0..(JobCenter::TERMINAL_HISTORY_LIMIT + 5) {
            let (job, _) = center.begin_job("operation-copy", sources(1), None);
            center.finish_items(
                &job,
                vec![(job.sources[0].clone(), JobItemState::Succeeded)],
            );
        }
        let jobs = center.jobs();
        assert!(jobs.iter().any(|job| job.id == active.id));
        assert_eq!(
            jobs.iter().filter(|job| !job.status().is_active()).count(),
            JobCenter::TERMINAL_HISTORY_LIMIT
        );
    }

    /// Unknown totals never fabricate a fraction: the snapshot keeps the
    /// indeterminate flag until a real total arrives (captured count or
    /// worker report).
    #[test]
    fn progress_stays_indeterminate_without_a_reported_total() {
        let center = JobCenter::new();
        let (job, _cancellable) = center.begin_job("operation-copy", sources(2), None);
        let _ = _cancellable;
        assert!(job.cancellable().is_some());
        assert!(job.load_fraction().is_none());
        job.start(None);
        assert!(job.load_fraction().is_none());
        job.report_current_file(10, -1);
        assert!(job.load_fraction().is_none());
        job.start(Some(4));
        let (fraction, done, total) = job.load_fraction().unwrap();
        assert_eq!(total, 4);
        assert_eq!(done, 0);
        assert!(fraction == 0.0);
    }

    /// Live attempt counting drives the determinate fraction: really
    /// attempted items, never extrapolated from recursive scans.
    #[test]
    fn live_attempts_drive_the_progress_fraction() {
        let center = JobCenter::new();
        let (job, _) = center.begin_job("operation-copy", sources(4), None);
        job.start(Some(4));
        job.record_attempt();
        job.record_attempt();
        let (fraction, done, total) = job.load_fraction().unwrap();
        assert_eq!((done, total), (2, 4));
        assert_eq!(fraction, 0.5);
    }

    #[test]
    fn external_move_already_completed_retains_worker_attempt_progress() {
        let center = JobCenter::new();
        let (job, _) = center.begin_job("operation-trash", sources(1), None);
        job.start(Some(1));
        job.record_attempt();

        center.finish_items(&job, Vec::new());

        assert_eq!(job.status(), JobStatus::Completed);
        assert_eq!(job.load_fraction(), Some((1.0, 1, 1)));
    }

    #[test]
    fn copy_byte_progress_refines_the_batch_and_cancelled_items_are_not_complete() {
        let center = JobCenter::new();
        let (job, _) = center.begin_job("operation-copy", sources(2), None);
        job.start(Some(2));
        job.report_current_file(50, 100);
        let (fraction, done, total) = job.load_fraction().unwrap();
        assert_eq!((done, total), (0, 2));
        assert_eq!(fraction, 0.25);
        center.finish_items(
            &job,
            vec![("file:///src-0".into(), JobItemState::Cancelled("stopped".into()))],
        );
        assert_eq!(job.load_fraction().unwrap().0, 0.0);
    }

    /// Retry classification: only failed/cancelled elements need retry and
    /// successes stay in the summary without being replayed.
    #[test]
    fn retry_candidates_exclude_recorded_successes() {
        crate::l10n::init(kito_i18n::I18n::new(
            kito_i18n::AppLang::English,
            kito_i18n::detect_system(),
        ));
        let center = JobCenter::new();
        let (job, _) = center.begin_job("operation-copy", sources(3), None);
        center.finish_items(
            &job,
            vec![
                ("file:///src-0".into(), JobItemState::Succeeded),
                ("file:///src-1".into(), JobItemState::Failed("denied".into())),
                ("file:///src-2".into(), JobItemState::Cancelled("stopped".into())),
            ],
        );
        assert_eq!(job.status(), JobStatus::Partial);
        let summary = job.items_summary();
        // The summary shows decoded display paths, not raw URIs.
        let failed_shown = kito_core::uri_to_display("file:///src-1");
        let success_shown = kito_core::uri_to_display("file:///src-0");
        assert!(summary.contains(&failed_shown), "summary: {summary}");
        assert!(summary.contains("denied"));
        assert!(summary.contains(&success_shown));
    }

    /// A failed job keeps the registered items; a worker failure marks the
    /// whole job without implying per-element outcomes.
    #[test]
    fn worker_failure_marks_job_failed_without_replaying_items() {
        let center = JobCenter::new();
        let (job, _) = center.begin_job("operation-trash", sources(1), None);
        job.start(None);
        center.finish_worker_failure(&job);
        assert_eq!(job.status(), JobStatus::Failed);
        assert_eq!(job.count_items(), (0, 0, 0));
    }

    /// The job handle is usable from the queue worker thread: all shared
    /// state is atomics/Mutex/`Cancellable` only (compiles only when
    /// `Arc<Job>` really is `Send`).
    #[test]
    fn job_handle_is_send_for_worker_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<Arc<Job>>();
        assert_send::<gio::Cancellable>();
    }

    /// Running-worker cooperative cancellation with real temporary files:
    /// the same pattern `Ctx::submit_job` uses in production, exercised
    /// with a real thread and a real Gio Cancellable into a real copy.
    #[test]
    fn running_job_cancels_cooperatively_with_real_files() {
        use std::{fs, io::Write, time::Duration};
        let temporary = tempfile::tempdir().unwrap();
        let source_dir = temporary.path().join("source");
        let dest_dir = temporary.path().join("dest");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();
        let mut big = fs::File::create(source_dir.join("big.bin")).unwrap();
        big.write_all(&vec![0u8; 4 * 1024 * 1024]).unwrap();
        drop(big);
        let source_uri = gio::File::for_path(source_dir.join("big.bin"))
            .uri()
            .to_string();
        let dest_uri = gio::File::for_path(&dest_dir).uri().to_string();
        let dest_path = dest_dir.join("big.bin");
        let dest_file_uri = gio::File::for_path(&dest_path).uri().to_string();

        let center = JobCenter::new();
        let (job, cancellable) = center.begin_job(
            "operation-copy",
            vec![source_uri.clone()],
            Some(dest_uri.clone()),
        );
        job.start(Some(1));
        // Worker: exactly what submit_job runs (Send captures only). The
        // classification is driven by the main thread per-element record
        // after join, mirroring ops.rs where the completion runs on GTK.
        let worker_job = job.clone();
        let worker_source = source_uri.clone();
        let worker_dest = dest_uri.clone();
        let handle = std::thread::spawn(move || {
            kito_core::copy_to_cancellable(
                &worker_source,
                &worker_dest,
                &cancellable,
                |bytes_current, bytes_current_total| {
                    worker_job.report_current_file(bytes_current as u64, bytes_current_total);
                },
            )
        });

        // Cancel from the main thread while the copy is in flight.
        while job.status() != JobStatus::Running {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(center.cancel(job.id));
        assert_eq!(job.status(), JobStatus::Cancelling);
        let result = handle.join().unwrap();
        let partial_output_exists = dest_path.exists();
        // Classification on the main thread (as ops.rs completions do).
        let outcome = match result {
            Ok(()) => (source_uri.clone(), JobItemState::Succeeded),
            Err(error) if error.matches(gio::IOErrorEnum::Cancelled) => {
                let marker = "partial copy output may remain at";
                let error_message = error.to_string();
                let retry_allowed = !error_message.contains(marker);
                if partial_output_exists {
                    assert!(error_message.contains(marker));
                    assert!(error_message.contains(&dest_file_uri));
                    assert!(!retry_allowed, "partial target must not allow blind retry");
                } else {
                    assert!(!error_message.contains(marker));
                    assert!(retry_allowed);
                }
                (source_uri.clone(), JobItemState::Cancelled(error_message))
            }
            Err(error) => (source_uri.clone(), JobItemState::Failed(error.to_string())),
        };
        center.finish_items(&job, vec![outcome]);

        // The job classifies honestly; the source file survives untouched.
        assert_eq!(job.status(), JobStatus::Cancelled);
        let (succeeded, failed, cancelled) = job.count_items();
        assert_eq!((succeeded, failed, cancelled), (0, 0, 1));
        let item_summary = job.items_summary();
        if partial_output_exists {
            assert!(item_summary.contains("partial copy output may remain at"));
            assert!(item_summary.contains(&dest_file_uri));
        } else {
            assert!(!item_summary.contains("partial copy output may remain at"));
        }
        assert!(source_dir.join("big.bin").exists());
        assert_eq!(
            fs::metadata(source_dir.join("big.bin")).unwrap().len(),
            4 * 1024 * 1024
        );
    }

    /// A single-copy success keeps the job Completed with both sides
    /// present: no partial classification leaks into plain success.
    #[test]
    fn successful_worker_copy_classifies_completed() {
        use std::fs;
        let temporary = tempfile::tempdir().unwrap();
        let source_path = temporary.path().join("one.bin");
        fs::write(&source_path, &[7u8; 1024]).unwrap();
        let source_uri = gio::File::for_path(&source_path).uri().to_string();
        let dest_dir = temporary.path().join("out");
        fs::create_dir_all(&dest_dir).unwrap();
        let dest_uri = gio::File::for_path(&dest_dir).uri().to_string();

        let center = JobCenter::new();
        let (job, cancellable) = center.begin_job(
            "operation-copy",
            vec![source_uri.clone()],
            Some(dest_uri.clone()),
        );
        job.start(Some(1));
        let worker_source = source_uri.clone();
        let worker_dest = dest_uri.clone();
        let handle = std::thread::spawn(move || {
            kito_core::copy_to_cancellable(&worker_source, &worker_dest, &cancellable, |_, _| {})
        });
        let result = handle.join().unwrap();
        let outcome = match result {
            Ok(()) => (source_uri.clone(), JobItemState::Succeeded),
            Err(error) => (source_uri.clone(), JobItemState::Failed(error.to_string())),
        };
        center.finish_items(&job, vec![outcome]);
        assert_eq!(job.status(), JobStatus::Completed);
        assert!(dest_dir.join("one.bin").exists());
        assert!(source_path.exists());
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn operations_dialog_has_content_and_refresh_action() {
        gtk::init().unwrap();
        let center = JobCenter::new();
        let dialog = build_operations_dialog(&center);
        let content = dialog.child().expect("dialog content child");
        let scroll = content.first_child().expect("job list scroll area");
        assert!(scroll.is::<gtk::ScrolledWindow>());
        let close = scroll.next_sibling().expect("close button");
        assert!(close.is::<gtk::Button>());
        assert!(gtk::prelude::WidgetExt::activate_action(&dialog, "jobs.refresh", None).is_ok());
        assert!(crate::APP_MENU_ITEMS
            .iter()
            .any(|(label, action, _)| *label == "menu-operations" && *action == "win.operations"));
    }
}
