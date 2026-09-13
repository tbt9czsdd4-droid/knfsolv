//! One persistent computation thread with two replaceable slots: computer, then coaching.
use crate::{
    Error, Result,
    game::{JobPurpose, JobToken},
    solver::{AnalysisSession, Cancel, Report, Settings, WorkSlice},
    state::Position,
};
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnalysisKey {
    pub token: JobToken,
    pub serial: u64,
}
#[derive(Clone)]
pub struct AnalysisRequest {
    pub key: AnalysisKey,
    pub position: Position,
    pub settings: Settings,
    pub continuous: bool,
    pub paused: bool,
    /// Computer baseline decisions require no sampled search; still run off the UI thread.
    pub baseline_only: bool,
}
#[derive(Clone)]
pub struct AnalysisUpdate {
    pub key: AnalysisKey,
    pub report: std::result::Result<Report, String>,
    pub finished: bool,
    pub paused: bool,
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
pub struct ComputationWorker {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}
impl ComputationWorker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let shared = Arc::new((
            Mutex::new(Shared {
                requests: [None, None],
                reports: [None, None],
                cancel: Cancel::default(),
                active: None,
                generation: 0,
                shutdown: false,
            }),
            Condvar::new(),
        ));
        let worker_shared = shared.clone();
        let thread = thread::Builder::new()
            .name("kniffel-analysis".into())
            .spawn(move || run(worker_shared, &wake))
            .expect("start analysis worker");
        Self {
            shared,
            thread: Some(thread),
        }
    }
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
        let (mutex, condition) = &*self.shared;
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
        let (mutex, condition) = &*self.shared;
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
        let (mutex, condition) = &*self.shared;
        let mut shared = mutex.lock().unwrap();
        shared.cancel.cancel();
        shared.requests = [None, None];
        shared.reports = [None, None];
        shared.generation += 1;
        condition.notify_one();
    }
    /// At most one report per purpose is retained; slow readers never build a queue.
    pub fn take_updates(&self) -> Vec<AnalysisUpdate> {
        self.shared
            .0
            .lock()
            .unwrap()
            .reports
            .iter_mut()
            .filter_map(Option::take)
            .collect()
    }
    pub fn buffered_reports(&self) -> usize {
        self.shared
            .0
            .lock()
            .unwrap()
            .reports
            .iter()
            .flatten()
            .count()
    }
    #[cfg(all(test, feature = "gui"))]
    pub(crate) fn inject(&self, update: AnalysisUpdate) {
        let i = slot(update.key.token.purpose).unwrap();
        self.shared.0.lock().unwrap().reports[i] = Some(update);
    }
}
impl Drop for ComputationWorker {
    fn drop(&mut self) {
        {
            let (mutex, condition) = &*self.shared;
            let mut shared = mutex.lock().unwrap();
            shared.shutdown = true;
            shared.cancel.cancel();
            condition.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn publish(shared: &Arc<(Mutex<Shared>, Condvar)>, i: usize, running: &Running, wake: &impl Fn()) {
    let update = running.update();
    let accepted = {
        let mut shared = shared.0.lock().unwrap();
        if shared.requests[i]
            .as_ref()
            .is_some_and(|r| r.key == update.key)
            && !shared.shutdown
        {
            shared.reports[i] = Some(update);
            true
        } else {
            false
        }
    };
    if accepted {
        wake();
    }
}
fn run(shared: Arc<(Mutex<Shared>, Condvar)>, wake: &impl Fn()) {
    let mut states: [Option<Running>; 2] = [None, None];
    loop {
        let (requests, generation) = {
            let shared = shared.0.lock().unwrap();
            if shared.shutdown {
                break;
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
                publish(&shared, i, state, wake);
            }
        }
        let chosen = (0..2).find(|&i| {
            requests[i].as_ref().is_some_and(|r| !r.paused)
                && states[i].as_ref().is_none_or(|s| !s.finished())
        });
        let Some(i) = chosen else {
            let (mutex, condition) = &*shared;
            let mut guard = mutex.lock().unwrap();
            guard.active = None;
            while !guard.shutdown && guard.generation == generation {
                guard = condition.wait(guard).unwrap();
            }
            continue;
        };
        let request = requests[i].as_ref().unwrap();
        let cancel = {
            let mut shared = shared.0.lock().unwrap();
            if shared.generation != generation || shared.shutdown {
                continue;
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
            publish(&shared, i, states[i].as_ref().unwrap(), wake); // Baseline before any sampled work.
        }
        let state = states[i].as_mut().unwrap();
        if !state.baseline_only
            && let Ok(session) = &mut state.session
            && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                session.advance_cancellable(WorkSlice::default(), &cancel)
            }))
            .is_err()
        {
            state.session = Err("Computation worker failed during search".into());
        }
        if state.finished() || state.published.elapsed() >= Duration::from_millis(250) {
            publish(&shared, i, state, wake);
            state.published = Instant::now();
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
            let mut shared = worker.shared.0.lock().unwrap();
            shared.requests[0].as_mut().unwrap().paused = false;
            shared.requests[1].as_mut().unwrap().paused = false;
            shared.generation += 1;
            worker.shared.1.notify_one();
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
