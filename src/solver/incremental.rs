//! Owned coaching search. Time limits count computation, never time spent paused.
use super::{Cancel, Method, Report, Settings, analyze, simulation::SampleSearch};
use crate::{
    Result,
    state::{Phase, Position},
};
use std::time::Duration;
use web_time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct WorkSlice {
    pub max_time: Duration,
    pub max_samples: u64,
    pub max_steps: u64,
}
impl Default for WorkSlice {
    fn default() -> Self {
        Self {
            max_time: Duration::from_millis(40),
            max_samples: 128,
            max_steps: u64::MAX,
        }
    }
}
impl WorkSlice {
    /// Deterministic completed-work boundary, useful for reproducible offline callers.
    pub fn samples(samples: u64) -> Self {
        Self {
            max_time: Duration::MAX,
            max_samples: samples,
            max_steps: u64::MAX,
        }
    }
}

/// The longest bounded exact attempt before Auto falls back to rollouts.
const EXACT_ATTEMPT: Duration = Duration::from_millis(500);

pub struct AnalysisSession {
    position: Position,
    settings: Settings,
    search: Option<SampleSearch>,
    exact_pending: bool,
    exact_result: Option<Report>,
    exact_nodes: usize,
    exact_depth: usize,
    elapsed: Duration,
    continuous: bool,
    paused: bool,
}
impl AnalysisSession {
    /// Auto coaching uses fixed-policy rollouts after a bounded exact attempt in small endgames.
    pub fn new(position: Position, settings: Settings) -> Result<Self> {
        let start = Instant::now();
        position.validate()?;
        settings.validate(&position)?;
        let exact_pending = settings.method == Method::Exact
            || (settings.method == Method::Auto && position.open() <= 3);
        let (search, exact_result) = if matches!(position.phase, Phase::Finished) {
            (
                None,
                Some(analyze(&position, &settings, Cancel::default())?),
            )
        } else {
            let mut effective = settings.clone();
            if effective.method == Method::Auto || effective.method == Method::Exact {
                effective.method = Method::Rollout;
            }
            (Some(SampleSearch::new(&position, &effective, true)), None)
        };
        Ok(Self {
            position,
            settings,
            search,
            exact_pending,
            exact_result,
            exact_nodes: 0,
            exact_depth: 0,
            elapsed: start.elapsed(),
            continuous: true,
            paused: false,
        })
    }
    pub fn set_continuous(&mut self, continuous: bool) {
        self.continuous = continuous;
    }
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
    pub fn samples(&self) -> u64 {
        self.search.as_ref().map_or(0, SampleSearch::samples)
    }
    pub fn cache_entries(&self) -> (usize, usize) {
        self.search
            .as_ref()
            .map_or((0, 0), SampleSearch::cache_entries)
    }
    pub fn computation_time(&self) -> Duration {
        self.elapsed
    }
    pub fn finished(&self) -> bool {
        self.exact_result.is_some()
            || (!self.continuous
                && (self.samples() >= self.settings.simulations
                    || (self.settings.time_ms > 0
                        && self.elapsed >= Duration::from_millis(self.settings.time_ms))))
    }
    pub fn advance(&mut self, slice: WorkSlice) {
        self.advance_cancellable(slice, &Cancel::default());
    }
    pub fn advance_cancellable(&mut self, slice: WorkSlice, cancel: &Cancel) {
        if self.paused
            || self.finished()
            || cancel.cancelled()
            || slice.max_samples == 0
            || slice.max_steps == 0
            || slice.max_time.is_zero()
        {
            return;
        }
        let start = Instant::now();
        let before = self.samples();
        let mut steps = 0;
        if self.exact_pending {
            // Exact is a single bounded attempt, not a changing rollout policy. Publish the
            // provisional snapshot before this call; sampled evidence has not started yet.
            // It may outlast the slice: small endgames need a few hundred milliseconds, and
            // cancellation still interrupts it.
            let mut settings = self.settings.clone();
            settings.method = Method::Exact;
            let remaining = if !self.continuous && self.settings.time_ms > 0 {
                Duration::from_millis(self.settings.time_ms).saturating_sub(self.elapsed)
            } else {
                EXACT_ATTEMPT
            };
            settings.time_ms = remaining.min(EXACT_ATTEMPT).as_millis().max(1) as u64;
            let mut report = analyze(&self.position, &settings, cancel.clone())
                .expect("validated analysis session");
            report.settings = self.settings.clone();
            self.exact_nodes = report.nodes;
            self.exact_depth = report.progress.map_or(0, |p| p.max_decision_depth);
            self.exact_pending = false;
            if report.status == "exact_under_model" || self.settings.method == Method::Exact {
                self.exact_result = Some(report);
            }
            // A pause/preemption before an exact answer preserves the chance to retry;
            // no sampled games or paired evidence have been counted.
            if cancel.cancelled()
                && self
                    .exact_result
                    .as_ref()
                    .is_none_or(|r| r.status != "exact_under_model")
            {
                self.exact_pending = true;
                self.exact_result = None;
            }
        }
        while self.exact_result.is_none()
            && !cancel.cancelled()
            && start.elapsed() < slice.max_time
            && self.samples() - before < slice.max_samples
            && steps < slice.max_steps
        {
            if !self.continuous
                && (self.samples() >= self.settings.simulations
                    || (self.settings.time_ms > 0
                        && self.elapsed + start.elapsed()
                            >= Duration::from_millis(self.settings.time_ms)))
            {
                break;
            }
            self.search.as_mut().unwrap().step();
            steps += 1;
        }
        self.elapsed += start.elapsed();
    }
    pub fn snapshot(&self) -> Report {
        let mut report = if let Some(report) = &self.exact_result {
            report.clone()
        } else {
            self.search.as_ref().unwrap().snapshot(true)
        };
        if self.exact_pending {
            report.progress = None;
        }
        report.settings = self.settings.clone();
        report.elapsed_ms = self.elapsed.as_millis();
        if self.exact_result.is_none() {
            report.nodes += self.exact_nodes;
            report.stop_reason = if self.paused {
                "Analysis paused; evidence retained"
            } else if self.finished() && self.samples() >= self.settings.simulations {
                "Simulation budget reached"
            } else if self.finished() {
                "Computation time budget reached"
            } else {
                "Refining analysis"
            }
            .into();
            if self.exact_nodes > 0 {
                report.note.push_str(&format!(
                    " Previous exact attempt: depth {}.",
                    self.exact_depth
                ));
                report.note.push_str(" Bounded exact attempt was incomplete; estimates use the unchanged frozen continuation policy.");
            }
        }
        report
    }
}
