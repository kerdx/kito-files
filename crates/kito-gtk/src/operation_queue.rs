//! Process-wide FIFO for filesystem mutations and their UI completions.

use gio::prelude::ApplicationExtManual;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    panic::{catch_unwind, AssertUnwindSafe},
    rc::Rc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UndoRecord {
    Move {
        source: String,
        target: String,
        identity: kito_core::FileIdentity,
    },
    Trash {
        source: String,
        target: String,
        identity: kito_core::FileIdentity,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UndoEntry {
    pub(crate) id: u64,
    pub(crate) record: UndoRecord,
}

#[derive(Default)]
struct UndoJournal {
    records: Vec<UndoEntry>,
    in_flight: Option<u64>,
    next_id: u64,
}

impl UndoJournal {
    const LIMIT: usize = 50;

    fn push(&mut self, record: UndoRecord) {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).expect("undo ID exhausted");
        self.records.push(UndoEntry { id, record });
        if self.records.len() > Self::LIMIT {
            let oldest_evictable = self
                .records
                .iter()
                .position(|entry| Some(entry.id) != self.in_flight)
                .expect("an in-flight undo entry cannot fill the journal");
            self.records.remove(oldest_evictable);
        }
    }

    fn finish(&mut self, id: u64, succeeded: bool) {
        if self.in_flight == Some(id) {
            self.in_flight = None;
        }
        if succeeded {
            self.records.retain(|entry| entry.id != id);
        }
    }

    fn invalidate(&mut self) {
        self.records.clear();
        self.in_flight = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UndoStartError {
    Busy,
    InProgress,
    Empty,
}

#[derive(Debug)]
pub(crate) enum WorkerFailure {
    Panicked,
    ChannelClosed,
    SpawnFailed,
    CancelledBeforeStart,
    QueueFull,
}

type Task = Box<dyn FnOnce(Rc<OperationQueue>)>;

pub(crate) const MAX_PENDING_TASKS: usize = 128;

/// One application-wide serializer. Jobs run on worker threads, but each
/// completion runs on the GTK thread before the next worker is started.
/// The pending FIFO is capped independently of its single running worker.
#[derive(Default)]
pub(crate) struct OperationQueue {
    pending: RefCell<VecDeque<Task>>,
    running: Cell<bool>,
    undo: RefCell<UndoJournal>,
    application: RefCell<Option<QueueApplication>>,
}

struct QueueApplication {
    application: gio::Application,
    active_holds: Rc<Cell<usize>>,
}

struct ApplicationHold {
    _gio_hold: gio::ApplicationHoldGuard,
    active_holds: Rc<Cell<usize>>,
}

impl Drop for ApplicationHold {
    fn drop(&mut self) {
        self.active_holds.set(self.active_holds.get() - 1);
    }
}

impl OperationQueue {
    /// Configures the application once, before windows can submit work.
    pub(crate) fn set_application(&self, application: gio::Application) {
        let mut configured_application = self.application.borrow_mut();
        assert!(
            configured_application.is_none(),
            "operation queue application set more than once"
        );
        *configured_application = Some(QueueApplication {
            application,
            active_holds: Rc::new(Cell::new(0)),
        });
    }

    fn acquire_application_hold(&self) -> Option<ApplicationHold> {
        self.application
            .borrow()
            .as_ref()
            .map(|application| {
                application.active_holds.set(application.active_holds.get() + 1);
                ApplicationHold {
                    _gio_hold: application.application.hold(),
                    active_holds: application.active_holds.clone(),
                }
            })
    }

    #[cfg(test)]
    fn active_application_holds(&self) -> usize {
        self.application
            .borrow()
            .as_ref()
            .map_or(0, |application| application.active_holds.get())
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.running.get() || !self.pending.borrow().is_empty()
    }

    pub(crate) fn push_undo(&self, record: UndoRecord) {
        self.undo.borrow_mut().push(record);
    }

    pub(crate) fn begin_undo(&self) -> Result<UndoEntry, UndoStartError> {
        if self.is_busy() {
            return Err(UndoStartError::Busy);
        }
        let mut undo = self.undo.borrow_mut();
        if undo.in_flight.is_some() {
            return Err(UndoStartError::InProgress);
        }
        let Some(entry) = undo.records.last().cloned() else {
            return Err(UndoStartError::Empty);
        };
        undo.in_flight = Some(entry.id);
        Ok(entry)
    }

    pub(crate) fn finish_undo(&self, id: u64, succeeded: bool) {
        self.undo.borrow_mut().finish(id, succeeded);
    }

    /// Appends an entire mutation batch as one FIFO job. `done` is invoked on
    /// the GTK main context; the queue does not start another worker until it
    /// returns, including when it panics.
    pub(crate) fn submit<R>(
        self: &Rc<Self>,
        job: impl FnOnce() -> R + Send + 'static,
        done: impl FnOnce(Result<R, WorkerFailure>) + 'static,
    ) where
        R: Send + 'static,
    {
        self.submit_cancellable(job, || false, done);
    }

    pub(crate) fn submit_cancellable<R>(
        self: &Rc<Self>,
        job: impl FnOnce() -> R + Send + 'static,
        cancelled: impl Fn() -> bool + Send + 'static,
        done: impl FnOnce(Result<R, WorkerFailure>) + 'static,
    ) where
        R: Send + 'static,
    {
        self.submit_cancellable_with_start(job, cancelled, || true, done);
    }

    pub(crate) fn submit_cancellable_with_start<R>(
        self: &Rc<Self>,
        job: impl FnOnce() -> R + Send + 'static,
        cancelled: impl Fn() -> bool + Send + 'static,
        start: impl Fn() -> bool + 'static,
        done: impl FnOnce(Result<R, WorkerFailure>) + 'static,
    ) where
        R: Send + 'static,
    {
        // Reject before acquiring an application hold or touching the worker
        // pump. Completion still runs on the caller (GTK main) thread so
        // domain-specific cleanup is never silently skipped.
        if self.pending.borrow().len() >= MAX_PENDING_TASKS {
            if catch_unwind(AssertUnwindSafe(|| done(Err(WorkerFailure::QueueFull)))).is_err() {
                eprintln!("Kito full-queue completion callback panicked");
            }
            return;
        }
        // Pending work must also prevent application shutdown, not only the
        // worker currently at the head of the FIFO.
        let application_hold = self.acquire_application_hold();
        self.pending.borrow_mut().push_back(Box::new(move |queue| {
            if cancelled() || !start() {
                if catch_unwind(AssertUnwindSafe(|| {
                    done(Err(WorkerFailure::CancelledBeforeStart))
                }))
                .is_err()
                {
                    eprintln!("Kito cancelled operation completion callback panicked");
                }
                queue.finish_current();
                return;
            }
            let (tx, rx) = async_channel::bounded::<Result<R, WorkerFailure>>(1);
            let completion_queue = queue.clone();
            glib::spawn_future_local(async move {
                // Keep the hold through callback, finish_current and the next
                // pump, avoiding an unheld interval between FIFO tasks.
                let _application_hold = application_hold;
                let result = rx.recv().await.unwrap_or(Err(WorkerFailure::ChannelClosed));
                if let Err(failure) = &result {
                    completion_queue.invalidate_for_worker_failure(failure);
                }
                if catch_unwind(AssertUnwindSafe(|| done(result))).is_err() {
                    eprintln!("Kito operation completion callback panicked");
                    completion_queue.invalidate_undo();
                }
                completion_queue.finish_current();
            });

            let spawn_failure_tx = tx.clone();
            let worker_tx = tx;
            let worker_cancelled = cancelled;
            let spawned = std::thread::Builder::new()
                .name("kito-files-operation".to_string())
                .spawn(move || {
                    let result = if worker_cancelled() {
                        Err(WorkerFailure::CancelledBeforeStart)
                    } else {
                        match catch_unwind(AssertUnwindSafe(job)) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                eprintln!("Kito filesystem mutation worker panicked");
                                Err(WorkerFailure::Panicked)
                            }
                        }
                    };
                    let _ = worker_tx.send_blocking(result);
                });
            if let Err(error) = spawned {
                eprintln!("could not start Kito filesystem worker: {error}");
                // No worker started, so the filesystem and undo history are
                // untouched. Send a distinct failure instead of interpreting
                // the dropped worker sender as an uncertain mutation.
                let _ = spawn_failure_tx.send_blocking(Err(WorkerFailure::SpawnFailed));
            }
        }));
        self.pump();
    }

    fn invalidate_for_worker_failure(&self, failure: &WorkerFailure) {
        if !matches!(
            failure,
            WorkerFailure::SpawnFailed | WorkerFailure::CancelledBeforeStart | WorkerFailure::QueueFull
        ) {
            self.invalidate_undo();
        }
    }

    fn invalidate_undo(&self) {
        self.undo.borrow_mut().invalidate();
    }

    fn finish_current(self: &Rc<Self>) {
        debug_assert!(self.running.get());
        self.running.set(false);
        self.pump();
    }

    fn pump(self: &Rc<Self>) {
        let task = {
            let mut pending = self.pending.borrow_mut();
            if self.running.get() {
                return;
            }
            let Some(task) = pending.pop_front() else {
                return;
            };
            self.running.set(true);
            task
        };
        task(self.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gio::prelude::*;
    use glib::MainContext;
    use std::{
        fs,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc, Mutex,
        },
        time::{Duration, Instant},
    };

    fn drive_until_idle(context: &MainContext, queue: &OperationQueue) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while queue.is_busy() {
            assert!(Instant::now() < deadline, "operation queue did not drain");
            context.iteration(true);
        }
    }

    fn test_record(name: &str) -> UndoRecord {
        UndoRecord::Move {
            source: name.into(),
            target: format!("{name}-target"),
            identity: kito_core::FileIdentity {
                filesystem: "fs".into(),
                file: name.into(),
            },
        }
    }

    #[test]
    fn undo_journal_is_bounded_and_keeps_newest_entries_in_lifo_order() {
        let mut journal = UndoJournal::default();
        for index in 0..=UndoJournal::LIMIT {
            journal.push(test_record(&format!("source-{index}")));
        }
        assert_eq!(journal.records.len(), UndoJournal::LIMIT);
        assert!(matches!(
            journal.records.last(),
            Some(UndoEntry {
                record: UndoRecord::Move { source, .. },
                ..
            }) if source == "source-50"
        ));
        assert!(matches!(
            journal.records.first(),
            Some(UndoEntry {
                record: UndoRecord::Move { source, .. },
                ..
            }) if source == "source-1"
        ));
        assert_eq!(journal.in_flight, None);
    }

    #[test]
    fn undo_completion_preserves_the_entry_on_failure_and_pops_only_on_success() {
        let mut journal = UndoJournal::default();
        journal.push(test_record("A"));
        let a_id = journal.records[0].id;
        journal.in_flight = Some(a_id);
        journal.push(test_record("B"));
        journal.finish(a_id, true);
        assert_eq!(journal.records.len(), 1);
        assert_eq!(journal.records[0].record, test_record("B"));

        journal.records.clear();
        journal.push(test_record("A"));
        let a_id = journal.records[0].id;
        journal.in_flight = Some(a_id);
        journal.push(test_record("B"));
        journal.finish(a_id, false);
        assert_eq!(journal.records[0].record, test_record("A"));
        assert_eq!(journal.records[1].record, test_record("B"));

        journal.in_flight = Some(a_id);
        journal.finish(a_id, true);
        assert_eq!(journal.in_flight, None);
        assert_eq!(journal.records.len(), 1);
        assert_eq!(journal.records[0].record, test_record("B"));
    }

    #[test]
    fn in_flight_undo_survives_capacity_eviction_and_id_checked_completion() {
        let mut journal = UndoJournal::default();
        journal.push(test_record("A"));
        let a_id = journal.records[0].id;
        journal.in_flight = Some(a_id);
        for index in 0..UndoJournal::LIMIT {
            journal.push(test_record(&format!("later-{index}")));
        }
        assert_eq!(journal.records.len(), UndoJournal::LIMIT);
        assert!(journal.records.iter().any(|entry| entry.id == a_id));
        let b_id = journal.records.last().unwrap().id;
        assert_ne!(b_id, a_id);

        journal.in_flight = Some(b_id);
        journal.finish(a_id, false);
        assert_eq!(journal.in_flight, Some(b_id));
        assert!(journal.records.iter().any(|entry| entry.id == a_id));
        assert_eq!(journal.records.last().unwrap().id, b_id);

        journal.in_flight = Some(a_id);
        journal.finish(a_id, false);
        assert_eq!(journal.in_flight, None);
        assert!(journal.records.iter().any(|entry| entry.id == a_id));
        assert_eq!(journal.records.last().unwrap().id, b_id);

        journal.in_flight = Some(a_id);
        journal.finish(a_id, true);
        assert_eq!(journal.in_flight, None);
        assert_eq!(journal.records.len(), UndoJournal::LIMIT - 1);
        assert!(!journal.records.iter().any(|entry| entry.id == a_id));
        assert_eq!(journal.records.last().unwrap().id, b_id);
    }

    #[test]
    fn queue_is_fifo_single_worker_and_waits_for_ui_completion() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let queue = Rc::new(OperationQueue::default());
                let first_window_queue = queue.clone();
                let second_window_queue = queue.clone();
                assert!(Rc::ptr_eq(&first_window_queue, &second_window_queue));

                let active = Arc::new(AtomicUsize::new(0));
                let peak = Arc::new(AtomicUsize::new(0));
                let events = Arc::new(Mutex::new(Vec::<String>::new()));

                let active_worker = active.clone();
                let peak_worker = peak.clone();
                let worker_events = events.clone();
                first_window_queue.submit(
                    move || {
                        let active_now = active_worker.fetch_add(1, Ordering::SeqCst) + 1;
                        peak_worker.fetch_max(active_now, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(20));
                        worker_events.lock().unwrap().push("worker-1".into());
                        active_worker.fetch_sub(1, Ordering::SeqCst);
                        1
                    },
                    {
                        let events = events.clone();
                        move |result| {
                            assert_eq!(result.unwrap(), 1);
                            events.lock().unwrap().push("done-1".into());
                        }
                    },
                );

                let active_worker = active.clone();
                let peak_worker = peak.clone();
                let worker_events = events.clone();
                second_window_queue.submit(
                    move || {
                        let active_now = active_worker.fetch_add(1, Ordering::SeqCst) + 1;
                        peak_worker.fetch_max(active_now, Ordering::SeqCst);
                        worker_events.lock().unwrap().push("worker-2".into());
                        active_worker.fetch_sub(1, Ordering::SeqCst);
                        2
                    },
                    {
                        let events = events.clone();
                        move |result| {
                            assert_eq!(result.unwrap(), 2);
                            events.lock().unwrap().push("done-2".into());
                        }
                    },
                );

                assert!(queue.is_busy());
                assert_eq!(queue.begin_undo(), Err(UndoStartError::Busy));
                drive_until_idle(&context, &queue);
                assert_eq!(peak.load(Ordering::SeqCst), 1);
                assert_eq!(
                    *events.lock().unwrap(),
                    ["worker-1", "done-1", "worker-2", "done-2"]
                );
            })
            .unwrap();
    }

    #[test]
    fn application_stays_held_for_running_and_pending_tasks_until_completions() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let application = gio::Application::new(
                    Some("it.kito.KitoFiles.OperationQueueTest"),
                    gio::ApplicationFlags::NON_UNIQUE,
                );
                let queue = Rc::new(OperationQueue::default());
                queue.set_application(application.clone());

                let gate = Arc::new(AtomicBool::new(false));
                queue.submit(
                    {
                        let gate = gate.clone();
                        move || {
                            while !gate.load(Ordering::Acquire) {
                                std::thread::sleep(Duration::from_millis(1));
                            }
                        }
                    },
                    {
                        let queue = queue.clone();
                        move |result| {
                            result.unwrap();
                            assert_eq!(queue.active_application_holds(), 2);
                        }
                    },
                );
                queue.submit(
                    || 2,
                    {
                        let queue = queue.clone();
                        move |result| {
                            assert_eq!(result.unwrap(), 2);
                            assert_eq!(queue.active_application_holds(), 1);
                        }
                    },
                );

                assert_eq!(queue.active_application_holds(), 2);
                gate.store(true, Ordering::Release);
                drive_until_idle(&context, &queue);
                assert_eq!(queue.active_application_holds(), 0);
            })
            .unwrap();
    }

    #[test]
    fn pending_fifo_is_bounded_and_rejected_completion_runs_on_main_thread() {
        let context = MainContext::new();
        context.with_thread_default(|| {
            let queue = Rc::new(OperationQueue::default());
            queue.set_application(gio::Application::new(
                Some("it.kito.KitoFiles.BoundedQueueTest"),
                gio::ApplicationFlags::NON_UNIQUE,
            ));
            queue.push_undo(test_record("preserved"));
            let center = crate::job_center::JobCenter::new();
            let (overflow_job, _) = center.begin_job(
                "operation-copy",
                vec!["file:///overflow-source".into()],
                Some("file:///destination".into()),
            );
            let gate = Arc::new(AtomicBool::new(false));
            let worker_started = Arc::new(AtomicBool::new(false));
            let worker_count = Arc::new(AtomicUsize::new(0));
            let events = Arc::new(Mutex::new(Vec::new()));
            let main_thread = std::thread::current().id();

            queue.submit(
                {
                    let gate = gate.clone();
                    let started = worker_started.clone();
                    let count = worker_count.clone();
                    move || {
                        count.fetch_add(1, Ordering::SeqCst);
                        started.store(true, Ordering::Release);
                        while !gate.load(Ordering::Acquire) {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                },
                |_| {},
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            while !worker_started.load(Ordering::Acquire) {
                assert!(Instant::now() < deadline, "gate worker did not start");
                std::thread::sleep(Duration::from_millis(1));
            }

            for id in 0..MAX_PENDING_TASKS {
                let events_worker = events.clone();
                let count = worker_count.clone();
                queue.submit(
                    move || {
                        count.fetch_add(1, Ordering::SeqCst);
                        events_worker.lock().unwrap().push(id);
                    },
                    {
                        let events_done = events.clone();
                        move |result| {
                            result.unwrap();
                            events_done.lock().unwrap().push(MAX_PENDING_TASKS + id);
                        }
                    },
                );
            }
            assert_eq!(queue.pending.borrow().len(), MAX_PENDING_TASKS);
            assert_eq!(queue.active_application_holds(), MAX_PENDING_TASKS + 1);

            let rejected_called = Rc::new(Cell::new(0));
            let rejected_called_cb = rejected_called.clone();
            let failed_job = overflow_job.clone();
            let center_for_rejection = center.clone();
            let marker = Arc::new(AtomicBool::new(false));
            let marker_worker = marker.clone();
            queue.submit(
                move || marker_worker.store(true, Ordering::Release),
                move |result| {
                    assert_eq!(std::thread::current().id(), main_thread);
                    assert!(matches!(result, Err(WorkerFailure::QueueFull)));
                    center_for_rejection.finish_worker_failure(&failed_job);
                    rejected_called_cb.set(rejected_called_cb.get() + 1);
                },
            );
            assert_eq!(rejected_called.get(), 1);
            assert_eq!(overflow_job.status(), crate::job_center::JobStatus::Failed);
            assert!(overflow_job.items_summary().is_empty());
            assert_eq!(worker_count.load(Ordering::SeqCst), 1);
            assert!(!marker.load(Ordering::Acquire));
            assert_eq!(queue.active_application_holds(), MAX_PENDING_TASKS + 1);

            gate.store(true, Ordering::Release);
            drive_until_idle(&context, &queue);
            assert_eq!(worker_count.load(Ordering::SeqCst), MAX_PENDING_TASKS + 1);
            assert!(!marker.load(Ordering::Acquire));
            assert_eq!(queue.active_application_holds(), 0);
            let undo = queue.begin_undo().expect("queue full must preserve undo");
            queue.finish_undo(undo.id, false);
            let observed = events.lock().unwrap();
            let expected = (0..MAX_PENDING_TASKS)
                .flat_map(|id| [id, MAX_PENDING_TASKS + id])
                .collect::<Vec<_>>();
            assert_eq!(*observed, expected);
        })
        .unwrap();
    }

    #[test]
    fn cancelled_queued_task_skips_worker_and_runs_completion() {
        let context = MainContext::new();
        context.with_thread_default(|| {
            let queue = Rc::new(OperationQueue::default());
            queue.set_application(gio::Application::new(
                Some("it.kito.KitoFiles.CancelledQueueTest"),
                gio::ApplicationFlags::NON_UNIQUE,
            ));
            let fixture = tempfile::tempdir().unwrap();
            let marker = fixture.path().join("must-not-exist");
            let center = crate::job_center::JobCenter::new();
            let source = gio::File::for_path(&marker).uri().to_string();
            let (job, token) = center.begin_job("operation-delete", vec![source], None);
            let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
            queue.submit(
                {
                    let gate = gate.clone();
                    move || {
                        while !gate.load(Ordering::Acquire) {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                },
                |_| {},
            );
            let completed = Arc::new(AtomicBool::new(false));
            let worker_job = job.clone();
            let start_job = job.clone();
            queue.submit_cancellable_with_start(
                {
                    let marker = marker.clone();
                    move || std::fs::write(marker, b"unexpected operation")
                },
                {
                    let token = token.clone();
                    move || token.is_cancelled()
                },
                move || start_job.start(Some(1)),
                {
                    let completed = completed.clone();
                    let center = center.clone();
                    let job = job.clone();
                    let queue = queue.clone();
                    move |result| {
                        assert!(matches!(result, Err(WorkerFailure::CancelledBeforeStart)));
                        assert_eq!(queue.active_application_holds(), 2);
                        center.finish_items(
                            &job,
                            job.sources.iter().map(|source| (
                                source.clone(),
                                crate::job_center::JobItemState::Cancelled("before start".into()),
                            )).collect(),
                        );
                        completed.store(true, Ordering::Release);
                    }
                },
            );
            assert!(center.cancel_queued(job.id));
            gate.store(true, Ordering::Release);
            drive_until_idle(&context, &queue);
            assert!(!marker.exists(), "cancelled queued operation touched disk");
            assert!(completed.load(Ordering::Acquire));
            assert_eq!(queue.active_application_holds(), 0);
            assert_eq!(worker_job.status(), crate::job_center::JobStatus::Cancelled);
            })
            .unwrap();
    }

    #[test]
    fn worker_and_completion_panics_release_the_queue_and_invalidate_history() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let queue = Rc::new(OperationQueue::default());
                queue.set_application(gio::Application::new(
                    Some("it.kito.KitoFiles.PanicQueueTest"),
                    gio::ApplicationFlags::NON_UNIQUE,
                ));
                let identity = kito_core::FileIdentity {
                    filesystem: "fs".into(),
                    file: "id".into(),
                };
                queue.push_undo(UndoRecord::Move {
                    source: "source".into(),
                    target: "target".into(),
                    identity,
                });

                let worker_failed = Rc::new(Cell::new(false));
                let worker_failed_done = worker_failed.clone();
                queue.submit(
                    || -> () { panic!("test worker panic") },
                    move |result| worker_failed_done.set(result.is_err()),
                );
                let next_done = Rc::new(Cell::new(false));
                let next_done_callback = next_done.clone();
                queue.submit(
                    || 1,
                    move |result| {
                        assert_eq!(result.unwrap(), 1);
                        next_done_callback.set(true);
                    },
                );
                drive_until_idle(&context, &queue);
                assert!(worker_failed.get());
                assert!(next_done.get());
                assert_eq!(queue.active_application_holds(), 0);
                assert_eq!(queue.begin_undo(), Err(UndoStartError::Empty));

                queue.push_undo(UndoRecord::Move {
                    source: "source".into(),
                    target: "target".into(),
                    identity: kito_core::FileIdentity {
                        filesystem: "fs".into(),
                        file: "id-2".into(),
                    },
                });
                queue.submit(|| 1, |_| panic!("test completion panic"));
                let after_panic_done = Rc::new(Cell::new(false));
                let after_panic_callback = after_panic_done.clone();
                queue.submit(
                    || 2,
                    move |result| {
                        assert_eq!(result.unwrap(), 2);
                        after_panic_callback.set(true);
                    },
                );
                drive_until_idle(&context, &queue);
                assert!(after_panic_done.get());
                assert_eq!(queue.active_application_holds(), 0);
                assert_eq!(queue.begin_undo(), Err(UndoStartError::Empty));
            })
            .unwrap();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn nested_moves_are_serialized_and_undo_in_lifo_order() {
        gtk::test_synced(|| {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path();
            let p = root.join("P");
            let q = root.join("Q");
            let r = root.join("R");
            fs::create_dir_all(p.join("child")).unwrap();
            fs::create_dir(&q).unwrap();
            fs::create_dir(&r).unwrap();
            fs::write(p.join("child/file.txt"), b"undo order").unwrap();

            let uri = |path: &std::path::Path| gio::File::for_path(path).uri().to_string();
            let p_uri = uri(&p);
            let child_uri = uri(&p.join("child"));
            let q_uri = uri(&q);
            let r_uri = uri(&r);
            let queue = Rc::new(OperationQueue::default());
            let completions = Rc::new(RefCell::new(Vec::<String>::new()));

            let child_source = child_uri.clone();
            let child_destination = q_uri.clone();
            let child_source_worker = child_source.clone();
            let child_queue = queue.clone();
            queue.submit(
                move || {
                    let before = kito_core::file_identity(&child_source_worker).unwrap();
                    let target =
                        kito_core::move_to_exact_result(&child_source_worker, &child_destination)
                            .unwrap();
                    let after = kito_core::file_identity(&target).unwrap();
                    assert_eq!(before, after);
                    UndoRecord::Move {
                        source: child_source_worker,
                        target,
                        identity: before,
                    }
                },
                {
                    let completions = completions.clone();
                    move |result| {
                        let record = result.unwrap();
                        child_queue.push_undo(record);
                        completions.borrow_mut().push("child-move".into());
                    }
                },
            );

            let parent_source_worker = p_uri.clone();
            let parent_destination = r_uri.clone();
            let parent_queue = queue.clone();
            queue.submit(
                move || {
                    let before = kito_core::file_identity(&parent_source_worker).unwrap();
                    let target =
                        kito_core::move_to_exact_result(&parent_source_worker, &parent_destination)
                            .unwrap();
                    let after = kito_core::file_identity(&target).unwrap();
                    assert_eq!(before, after);
                    UndoRecord::Move {
                        source: parent_source_worker,
                        target,
                        identity: before,
                    }
                },
                {
                    let completions = completions.clone();
                    move |result| {
                        let record = result.unwrap();
                        parent_queue.push_undo(record);
                        completions.borrow_mut().push("parent-move".into());
                    }
                },
            );

            let context = glib::MainContext::default();
            drive_until_idle(&context, &queue);
            assert_eq!(&*completions.borrow(), &["child-move", "parent-move"]);
            assert!(r.join("P").is_dir());
            assert!(!r.join("P").join("child").exists());
            assert!(q.join("child").join("file.txt").exists());

            for expected_source in [&p_uri, &child_uri] {
                let entry = queue.begin_undo().unwrap();
                let (source, target, identity) = match entry.record {
                    UndoRecord::Move {
                        source,
                        target,
                        identity,
                    }
                    | UndoRecord::Trash {
                        source,
                        target,
                        identity,
                    } => (source, target, identity),
                };
                assert_eq!(&source, expected_source);
                let queue_for_done = queue.clone();
                queue.submit(
                    move || kito_core::move_exact_checked(&target, &source, &identity),
                    move |result| queue_for_done.finish_undo(entry.id, result.unwrap().is_ok()),
                );
                drive_until_idle(&context, &queue);
            }

            assert_eq!(fs::read(p.join("child/file.txt")).unwrap(), b"undo order");
            assert!(!q.join("child").exists());
            assert!(!r.join("P").exists());
        });
    }

    #[test]
    fn worker_spawn_failure_preserves_undo_but_uncertain_worker_failure_invalidates_it() {
        let queue = OperationQueue::default();
        let identity = kito_core::FileIdentity {
            filesystem: "fs".into(),
            file: "id".into(),
        };
        queue.push_undo(UndoRecord::Move {
            source: "source".into(),
            target: "target".into(),
            identity: identity.clone(),
        });

        queue.invalidate_for_worker_failure(&WorkerFailure::SpawnFailed);
        let entry = queue.begin_undo().unwrap();
        assert_eq!(
            entry.record,
            UndoRecord::Move {
                source: "source".into(),
                target: "target".into(),
                identity,
            }
        );
        queue.finish_undo(entry.id, false);

        queue.invalidate_for_worker_failure(&WorkerFailure::Panicked);
        assert_eq!(queue.begin_undo(), Err(UndoStartError::Empty));
    }
}
