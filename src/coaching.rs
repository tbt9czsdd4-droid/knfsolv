//! Analysis away from the interface, behind [`Computation`]. A [`Scheduler`] keeps two
//! replaceable slots, computer then coaching, and a [`Runner`] advances the chosen job one slice
//! at a time. Natively [`ComputationWorker`] drives them on one persistent thread; in the browser
//! a Web Worker drives them between messages (`web/src/bin/worker.rs`).
use crate::{
    Error, Result,
    game::{JobPurpose, JobToken},
    solver::{AnalysisSession, Cancel, Report, Settings, WorkSlice},
    state::Position,
};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::{self, JoinHandle};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use web_time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisKey {
    pub token: JobToken,
    pub serial: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalysisRequest {
    pub key: AnalysisKey,
    pub position: Position,
    pub settings: Settings,
    pub continuous: bool,
    pub paused: bool,
    /// Computer baseline decisions require no sampled search; still run off the UI thread.
    pub baseline_only: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalysisUpdate {
    pub key: AnalysisKey,
    pub report: std::result::Result<Report, String>,
    pub finished: bool,
    pub paused: bool,
}
/// Asks the interface to repaint because new reports are waiting.
pub type Wake = Box<dyn Fn() + Send + Sync>;
/// Runs coaching and computer-decision analyses for the interface. Each purpose has one slot:
/// a new request replaces the old one, and only its latest report is kept.
pub trait Computation {
    fn submit(&self, request: AnalysisRequest) -> Result<()>;
    fn controls(&self, key: AnalysisKey, continuous: bool, paused: bool);
    fn clear(&self);
    /// At most one report per purpose is retained; slow readers never build a queue.
    fn take_updates(&self) -> Vec<AnalysisUpdate>;
    #[cfg(all(test, feature = "gui"))]
    fn inject(&self, update: AnalysisUpdate);
}
struct Shared {
    requests: [Option<AnalysisRequest>; 2],
    reports: [Option<AnalysisUpdate>; 2],
    cancel: Cancel,
    active: Option<usize>,
    generation: u64,
    shutdown: bool,
}
struct Running {
    key: AnalysisKey,
    session: std::result::Result<AnalysisSession, String>,
    paused: bool,
    continuous: bool,
    published: Instant,
    baseline_only: bool,
}
impl Running {
    fn finished(&self) -> bool {
        self.baseline_only
            || self
                .session
                .as_ref()
                .map_or(true, AnalysisSession::finished)
    }
    fn update(&self) -> AnalysisUpdate {
        AnalysisUpdate {
            key: self.key,
            report: self
                .session
                .as_ref()
                .map(|session| {
                    let mut report = session.snapshot();
                    if self.baseline_only {
                        report.status = "policy_decision".into();
                        report.method = "Declared baseline policy".into();
                        report.recommendation_basis = "declared_policy".into();
                        report.confidence = "not_applicable".into();
                        report.selection_reason = format!(
                            "Declared policy: {}. No sampled search.",
                            report.settings.model(self.key.token.actor).name()
                        );
                    }
                    report
                })
                .map_err(Clone::clone),
            finished: self.finished(),
            paused: self.paused,
        }
    }
}
fn slot(purpose: JobPurpose) -> Option<usize> {
    match purpose {
        JobPurpose::ComputerDecision => Some(0),
        JobPurpose::Coaching => Some(1),
        _ => None,
    }
}

/// Requests and their latest reports, one slot per purpose. Clones share the same slots, so
/// the interface and a [`Runner`] on another thread can use one scheduler.
#[derive(Clone)]
pub struct Scheduler(Arc<(Mutex<Shared>, Condvar)>);
impl Default for Scheduler {
    fn default() -> Self {
        Self(Arc::new((
            Mutex::new(Shared {
                requests: [None, None],
                reports: [None, None],
                cancel: Cancel::default(),
                active: None,
                generation: 0,
                shutdown: false,
            }),
            Condvar::new(),
        )))
    }
}
impl Scheduler {
    pub fn submit(&self, request: AnalysisRequest) -> Result<()> {
        let i = slot(request.key.token.purpose).ok_or_else(|| {
            Error::Invalid(
                "Only coaching and computer-decision jobs use the computation worker".into(),
            )
        })?;
        if request.key.token.actor != request.position.active_player {
            return Err(Error::Invalid(
                "Analysis actor does not match the position".into(),
            ));
        }
        if request.baseline_only && i != 0 {
            return Err(Error::Invalid(
                "Only computer jobs may request a baseline decision".into(),
            ));
        }
        let (mutex, condition) = &*self.0;
        let mut shared = mutex.lock().unwrap();
        if shared.requests[i].as_ref().is_some_and(|old| {
            old.key == request.key
                && (old.position != request.position
                    || old.settings != request.settings
                    || old.baseline_only != request.baseline_only)
        }) {
            return Err(Error::Invalid(
                "Changed positions or settings require a new analysis key".into(),
            ));
        }
        // Lower-priority coaching changes cannot interrupt a computer decision.
        if shared.active.is_some_and(|active| i <= active) {
            shared.cancel.cancel();
        }
        shared.requests[i] = Some(request);
        shared.reports[i] = None;
        shared.generation += 1;
        condition.notify_one();
        Ok(())
    }
    pub fn controls(&self, key: AnalysisKey, continuous: bool, paused: bool) {
        let Some(i) = slot(key.token.purpose) else {
            return;
        };
        let (mutex, condition) = &*self.0;
        let mut shared = mutex.lock().unwrap();
        if let Some(request) = &mut shared.requests[i]
            && request.key == key
            && (request.continuous != continuous || request.paused != paused)
        {
            request.continuous = continuous;
            request.paused = paused;
            if shared.active == Some(i) {
                shared.cancel.cancel();
            }
            shared.generation += 1;
            condition.notify_one();
        }
    }
    pub fn clear(&self) {
        let (mutex, condition) = &*self.0;
        let mut shared = mutex.lock().unwrap();
        shared.cancel.cancel();
        shared.requests = [None, None];
        shared.reports = [None, None];
        shared.generation += 1;
        condition.notify_one();
    }
    /// At most one report per purpose is retained; slow readers never build a queue.
    pub fn take_updates(&self) -> Vec<AnalysisUpdate> {
        self.0
            .0
            .lock()
            .unwrap()
            .reports
            .iter_mut()
            .filter_map(Option::take)
            .collect()
    }
    pub fn buffered_reports(&self) -> usize {
        self.0.0.lock().unwrap().reports.iter().flatten().count()
    }
    /// The current requests with their latest controls, to replay into a fresh computation.
    pub fn requests(&self) -> Vec<AnalysisRequest> {
        self.0
            .0
            .lock()
            .unwrap()
            .requests
            .iter()
            .flatten()
            .cloned()
            .collect()
    }
    /// Keeps a report, possibly computed elsewhere, only while it answers the current request
    /// of its slot; stale reports from replaced or cleared requests are dropped.
    pub fn accept(&self, update: AnalysisUpdate) -> bool {
        let Some(i) = slot(update.key.token.purpose) else {
            return false;
        };
        let mut shared = self.0.0.lock().unwrap();
        let current = shared.requests[i]
            .as_ref()
            .is_some_and(|r| r.key == update.key);
        if current && !shared.shutdown {
            shared.reports[i] = Some(update);
        }
        current && !shared.shutdown
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn shutdown(&self) {
        let (mutex, condition) = &*self.0;
        let mut shared = mutex.lock().unwrap();
        shared.shutdown = true;
        shared.cancel.cancel();
        condition.notify_one();
    }
    /// Blocks until a request changes after `generation`, or the scheduler shuts down.
    #[cfg(not(target_arch = "wasm32"))]
    fn wait_for_change(&self, generation: u64) {
        let (mutex, condition) = &*self.0;
        let mut shared = mutex.lock().unwrap();
        while !shared.shutdown && shared.generation == generation {
            shared = condition.wait(shared).unwrap();
        }
    }
    #[cfg(all(test, feature = "gui"))]
    fn inject(&self, update: AnalysisUpdate) {
        let i = slot(update.key.token.purpose).unwrap();
        self.0.0.lock().unwrap().reports[i] = Some(update);
    }
}

/// What one [`Runner::step`] did.
pub enum Step {
    Worked,
    /// Nothing can run until a request changes after this generation.
    Idle(u64),
    Shutdown,
}
/// Owns the searches of a [`Scheduler`]'s requests and advances them where they run.
#[derive(Default)]
pub struct Runner {
    states: [Option<Running>; 2],
}
impl Runner {
    /// One scheduling round: sync job states with requests, then advance the chosen job one
    /// slice. `wake` runs whenever a report was published.
    pub fn step(&mut self, scheduler: &Scheduler, slice: WorkSlice, wake: &impl Fn()) -> Step {
        let states = &mut self.states;
        let shared = &scheduler.0;
        let (requests, generation) = {
            let shared = shared.0.lock().unwrap();
            if shared.shutdown {
                return Step::Shutdown;
            }
            (shared.requests.clone(), shared.generation)
        };
        for i in 0..2 {
            if states[i].as_ref().map(|s| s.key) != requests[i].as_ref().map(|r| r.key) {
                states[i] = None;
            }
            if let (Some(state), Some(request)) = (&mut states[i], &requests[i])
                && (state.paused != request.paused || state.continuous != request.continuous)
            {
                state.paused = request.paused;
                state.continuous = request.continuous;
                if let Ok(session) = &mut state.session {
                    session.set_paused(request.paused);
                    session.set_continuous(request.continuous);
                }
                publish(scheduler, state, wake);
            }
        }
        let chosen = (0..2).find(|&i| {
            requests[i].as_ref().is_some_and(|r| !r.paused)
                && states[i].as_ref().is_none_or(|s| !s.finished())
        });
        let Some(i) = chosen else {
            shared.0.lock().unwrap().active = None;
            return Step::Idle(generation);
        };
        let request = requests[i].as_ref().unwrap();
        let cancel = {
            let mut shared = shared.0.lock().unwrap();
            if shared.generation != generation || shared.shutdown {
                return Step::Worked;
            }
            shared.cancel = Cancel::default();
            shared.active = Some(i);
            shared.cancel.clone()
        };
        if states[i].is_none() {
            let mut session = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                AnalysisSession::new(request.position.clone(), request.settings.clone())
                    .map_err(|e| e.to_string())
            }))
            .unwrap_or_else(|_| Err("Computation worker failed while preparing the search".into()));
            if let Ok(session) = &mut session {
                session.set_continuous(request.continuous);
            }
            states[i] = Some(Running {
                key: request.key,
                session,
                paused: false,
                continuous: request.continuous,
                published: Instant::now(),
                baseline_only: request.baseline_only,
            });
            publish(scheduler, states[i].as_ref().unwrap(), wake); // Baseline before any sampled work.
        }
        let state = states[i].as_mut().unwrap();
        if !state.baseline_only
            && let Ok(session) = &mut state.session
            && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                session.advance_cancellable(slice, &cancel)
            }))
            .is_err()
        {
            state.session = Err("Computation worker failed during search".into());
        }
        if state.finished() || state.published.elapsed() >= Duration::from_millis(250) {
            publish(scheduler, state, wake);
            state.published = Instant::now();
        }
        Step::Worked
    }
}
fn publish(scheduler: &Scheduler, running: &Running, wake: &impl Fn()) {
    if scheduler.accept(running.update()) {
        wake();
    }
}

/// The native [`Computation`]: one persistent thread runs the scheduler's jobs.
#[cfg(not(target_arch = "wasm32"))]
pub struct ComputationWorker {
    scheduler: Scheduler,
    thread: Option<JoinHandle<()>>,
}
#[cfg(not(target_arch = "wasm32"))]
impl ComputationWorker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let scheduler = Scheduler::default();
        let jobs = scheduler.clone();
        let thread = thread::Builder::new()
            .name("kniffel-analysis".into())
            .spawn(move || {
                let mut runner = Runner::default();
                loop {
                    match runner.step(&jobs, WorkSlice::default(), &wake) {
                        Step::Worked => {}
                        Step::Shutdown => break,
                        Step::Idle(generation) => jobs.wait_for_change(generation),
                    }
                }
            })
            .expect("start analysis worker");
        Self {
            scheduler,
            thread: Some(thread),
        }
    }
    pub fn buffered_reports(&self) -> usize {
        self.scheduler.buffered_reports()
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl Computation for ComputationWorker {
    fn submit(&self, request: AnalysisRequest) -> Result<()> {
        self.scheduler.submit(request)
    }
    fn controls(&self, key: AnalysisKey, continuous: bool, paused: bool) {
        self.scheduler.controls(key, continuous, paused);
    }
    fn clear(&self) {
        self.scheduler.clear();
    }
    fn take_updates(&self) -> Vec<AnalysisUpdate> {
        self.scheduler.take_updates()
    }
    #[cfg(all(test, feature = "gui"))]
    fn inject(&self, update: AnalysisUpdate) {
        self.scheduler.inject(update);
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl Drop for ComputationWorker {
    fn drop(&mut self) {
        self.scheduler.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dice::Dice, policy::Policy, solver::Method, state::Phase};
    use std::sync::mpsc;

    fn request(
        serial: u64,
        purpose: JobPurpose,
        continuous: bool,
        paused: bool,
    ) -> AnalysisRequest {
        let mut position = Position::new(1);
        position.players[0].sheet.cells = [Some(0); 13];
        position.players[0].sheet.cells[12] = None;
        position.phase = Phase::Decision {
            dice: Dice::from_faces(&[6, 6, 6, 6, 2]).unwrap(),
            rerolls_left: 1,
        };
        AnalysisRequest {
            key: AnalysisKey {
                token: JobToken {
                    actor: 0,
                    revision: serial,
                    purpose,
                },
                serial,
            },
            position,
            settings: Settings {
                method: Method::Rollout,
                policy: Policy::Greedy,
                time_ms: 0,
                simulations: 60,
                ..Settings::default()
            },
            continuous,
            paused,
            baseline_only: false,
        }
    }
    fn wait_for(
        worker: &ComputationWorker,
        condition: impl Fn(&AnalysisUpdate) -> bool,
    ) -> AnalysisUpdate {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            for update in worker.take_updates() {
                if condition(&update) {
                    return update;
                }
            }
            assert!(
                Instant::now() < deadline,
                "worker did not publish the expected update"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn worker_pauses_resumes_and_extends_limited_evidence_on_one_thread() {
        let (tx, rx) = mpsc::channel();
        let worker = ComputationWorker::new(move || {
            let _ = tx.send(thread::current().id());
        });
        let job = request(1, JobPurpose::Coaching, false, false);
        worker.submit(job.clone()).unwrap();
        let limited = wait_for(&worker, |u| u.finished);
        assert_eq!(limited.report.as_ref().unwrap().samples, 60);
        worker.controls(job.key, true, false);
        let running = wait_for(&worker, |u| {
            !u.finished && u.report.as_ref().is_ok_and(|r| r.samples > 60)
        });
        worker.controls(job.key, true, true);
        let paused = wait_for(&worker, |u| u.paused);
        let paused_n = paused.report.as_ref().unwrap().samples;
        assert!(paused_n >= running.report.as_ref().unwrap().samples);
        thread::sleep(Duration::from_millis(80));
        assert!(
            worker.take_updates().is_empty(),
            "paused worker should sleep, not publish repeated reports"
        );
        worker.controls(job.key, true, false);
        wait_for(&worker, |u| {
            !u.paused && u.report.as_ref().is_ok_and(|r| r.samples > paused_n)
        });
        drop(worker);
        let threads: Vec<_> = rx.try_iter().collect();
        assert!(threads.len() >= 4);
        assert!(
            threads.iter().all(|id| *id == threads[0]),
            "one persistent computation thread"
        );
    }

    #[test]
    fn latest_request_replaces_queued_work_and_stale_results() {
        let worker = ComputationWorker::new(|| {});
        for serial in 0..100 {
            worker
                .submit(request(serial, JobPurpose::Coaching, true, true))
                .unwrap();
        }
        let last = request(100, JobPurpose::Coaching, false, false);
        worker.submit(last.clone()).unwrap();
        let update = wait_for(&worker, |u| u.finished);
        assert_eq!(update.key, last.key);
        assert_eq!(update.report.unwrap().samples, 60);
        assert!(worker.buffered_reports() <= 2);
        worker.clear();
        assert!(worker.take_updates().is_empty());
        let mut changed = request(101, JobPurpose::Coaching, false, false);
        worker.submit(changed.clone()).unwrap();
        changed.settings.max_depth = 4;
        assert!(
            worker.submit(changed).is_err(),
            "changed settings must use a fresh revision/key"
        );
    }

    #[test]
    fn scheduler_replays_requests_with_their_latest_controls() {
        let scheduler = Scheduler::default();
        let coach = request(1, JobPurpose::Coaching, true, false);
        let computer = request(2, JobPurpose::ComputerDecision, false, false);
        scheduler.submit(coach.clone()).unwrap();
        scheduler.submit(computer.clone()).unwrap();
        scheduler.controls(coach.key, false, true);
        let replay = scheduler.requests();
        assert_eq!(
            replay.iter().map(|r| r.key).collect::<Vec<_>>(),
            [computer.key, coach.key]
        );
        assert!(replay[1].paused && !replay[1].continuous);
        scheduler.clear();
        assert!(scheduler.requests().is_empty());
    }

    #[test]
    fn scheduler_accepts_only_reports_for_current_requests() {
        let scheduler = Scheduler::default();
        let update = |key| AnalysisUpdate {
            key,
            report: Err("placeholder".into()),
            finished: false,
            paused: false,
        };
        let old = request(1, JobPurpose::Coaching, true, false);
        scheduler.submit(old.clone()).unwrap();
        assert!(scheduler.accept(update(old.key)));
        // An edit or Undo replaces the request; reports still in flight for it are stale.
        let new = request(2, JobPurpose::Coaching, true, false);
        scheduler.submit(new.clone()).unwrap();
        assert!(scheduler.take_updates().is_empty());
        assert!(!scheduler.accept(update(old.key)));
        assert!(scheduler.accept(update(new.key)));
        scheduler.clear();
        assert!(!scheduler.accept(update(new.key)));
        assert!(scheduler.take_updates().is_empty());
    }

    #[test]
    fn computer_priority_preempts_coaching_and_keeps_coaching_state() {
        let worker = ComputationWorker::new(|| {});
        let coach = request(1, JobPurpose::Coaching, true, false);
        worker.submit(coach.clone()).unwrap();
        let before = wait_for(&worker, |u| u.report.as_ref().is_ok_and(|r| r.samples > 60));
        // Pause the low-priority slot while queueing both jobs, then resume both together.
        worker.controls(coach.key, true, true);
        let paused = wait_for(&worker, |u| u.paused);
        let mut computer = request(2, JobPurpose::ComputerDecision, false, true);
        computer.settings.simulations = 400;
        worker.submit(computer.clone()).unwrap();
        {
            let mut shared = worker.scheduler.0.0.lock().unwrap();
            shared.requests[0].as_mut().unwrap().paused = false;
            shared.requests[1].as_mut().unwrap().paused = false;
            shared.generation += 1;
            worker.scheduler.0.1.notify_one();
        }
        let done = wait_for(&worker, |u| u.key == computer.key && u.finished);
        assert_eq!(done.report.unwrap().samples, 400);
        let resumed = wait_for(&worker, |u| {
            u.key == coach.key
                && u.report
                    .as_ref()
                    .is_ok_and(|r| r.samples > paused.report.as_ref().unwrap().samples)
        });
        assert!(resumed.report.unwrap().samples > before.report.unwrap().samples);
        // A slow reader retains two snapshots at most, never a report backlog.
        thread::sleep(Duration::from_millis(300));
        assert!(worker.buffered_reports() <= 2);
    }

    #[test]
    fn cancellation_and_drop_do_not_wait_for_a_large_budget() {
        let worker = ComputationWorker::new(|| {});
        let mut job = request(1, JobPurpose::Coaching, true, false);
        job.position = Position::new(8);
        job.position.phase = Phase::Decision {
            dice: Dice::from_faces(&[1, 2, 3, 4, 5]).unwrap(),
            rerolls_left: 2,
        };
        job.settings.policy = Policy::Opportunity;
        worker.submit(job).unwrap();
        wait_for(&worker, |u| u.report.is_ok());
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn failures_are_reported_once_and_other_job_purposes_cannot_run() {
        let worker = ComputationWorker::new(|| {});
        assert!(
            worker
                .submit(request(0, JobPurpose::ComputerAction, false, false))
                .is_err()
        );
        let mut invalid = request(1, JobPurpose::ComputerDecision, false, false);
        invalid.settings.simulations = 0;
        worker.submit(invalid).unwrap();
        let error = wait_for(&worker, |u| u.finished);
        assert!(error.report.is_err());
        thread::sleep(Duration::from_millis(30));
        assert!(worker.take_updates().is_empty());
    }
    #[test]
    fn computer_baseline_jobs_return_legal_declared_choices_without_sampling() {
        let worker = ComputationWorker::new(|| {});
        for (i, policy) in [
            Policy::Greedy,
            Policy::LegacyOpportunity,
            Policy::Opportunity,
        ]
        .into_iter()
        .enumerate()
        {
            let mut job = request(i as u64, JobPurpose::ComputerDecision, false, false);
            job.baseline_only = true;
            job.settings.policy = policy;
            let expected = crate::policy::choose(
                &job.position.players[0].sheet,
                Dice::from_faces(&[6, 6, 6, 6, 2]).unwrap(),
                1,
                policy,
            );
            worker.submit(job.clone()).unwrap();
            let update = wait_for(&worker, |u| u.key == job.key && u.finished);
            let report = update.report.unwrap();
            assert_eq!(report.samples, 0);
            assert_eq!(report.best_action, Some(expected));
            assert_eq!(report.status, "policy_decision");
            job.key.token.purpose = JobPurpose::Coaching;
            assert!(worker.submit(job).is_err());
        }
    }
}
