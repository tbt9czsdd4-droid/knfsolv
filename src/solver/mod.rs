pub mod endgame;
mod incremental;
pub use incremental::{AnalysisSession, WorkSlice};
mod simulation;
pub mod solitaire;
pub mod turn;

use crate::{
    Error, Result,
    policy::Policy,
    rules::Action,
    state::{Phase, Position},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Objective {
    #[default]
    SharedFirst,
    StrictFirst,
    ExpectedScore,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    #[default]
    Auto,
    Exact,
    Rollout,
    Mcts,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub perspective: usize,
    pub objective: Objective,
    pub policy: Policy,
    pub opponent_policies: Vec<Policy>,
    pub method: Method,
    pub time_ms: u64,
    /// Total full-game samples, split between actions (and training/validation for MCTS).
    pub simulations: u64,
    pub seed: u64,
    pub max_nodes: usize,
    /// MCTS perspective-player decisions from the root; 0 means unlimited.
    /// Beyond the limit, finish games with the baseline. Exact search is unaffected.
    pub max_depth: usize,
    pub cache: Option<std::path::PathBuf>,
    /// Optional offline-trained continuation model; absent uses the bundled model.
    pub baseline_model: Option<std::path::PathBuf>,
    /// Require demonstrated improvement before departing from the baseline.
    pub conservative_advice: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            perspective: 0,
            objective: Objective::SharedFirst,
            policy: Policy::Opportunity,
            opponent_policies: vec![],
            method: Method::Auto,
            time_ms: 2000,
            simulations: 20000,
            seed: 42,
            max_nodes: 50000,
            max_depth: 0,
            cache: None,
            baseline_model: None,
            conservative_advice: true,
        }
    }
}
impl Settings {
    pub fn model(&self, player: usize) -> Policy {
        self.opponent_policies
            .get(player)
            .copied()
            .unwrap_or(self.policy)
    }
    pub fn validate(&self, p: &Position) -> Result<()> {
        if self.perspective >= p.players.len() {
            return Err(Error::Invalid("Perspective player is out of range".into()));
        }
        if !self.opponent_policies.is_empty() && self.opponent_policies.len() != p.players.len() {
            return Err(Error::Invalid(
                "Per-player policies must have one entry per player".into(),
            ));
        }
        if self.simulations == 0 || self.max_nodes == 0 {
            return Err(Error::Invalid(
                "Simulation and node budgets must be positive".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub utility: f64,
    pub expected_score: f64,
    pub outright_win: f64,
    pub tied_first: f64,
}
impl Metrics {
    pub fn scaled(self, scale: f64) -> Self {
        Self {
            utility: self.utility * scale,
            expected_score: self.expected_score * scale,
            outright_win: self.outright_win * scale,
            tied_first: self.tied_first * scale,
        }
    }
    pub fn add(&mut self, rhs: Self) {
        self.utility += rhs.utility;
        self.expected_score += rhs.expected_score;
        self.outright_win += rhs.outright_win;
        self.tied_first += rhs.tied_first;
    }
}
pub fn terminal(p: &Position, s: &Settings) -> Metrics {
    let score = p.players[s.perspective].sheet.total();
    let top = p.players.iter().map(|p| p.sheet.total()).max().unwrap();
    let winners = p.players.iter().filter(|p| p.sheet.total() == top).count();
    let win = f64::from(score == top && winners == 1);
    let tie = f64::from(score == top && winners > 1);
    let utility = match s.objective {
        Objective::SharedFirst => {
            if score == top {
                1. / winners as f64
            } else {
                0.
            }
        }
        Objective::StrictFirst => win,
        Objective::ExpectedScore => score as f64,
    };
    Metrics {
        utility,
        expected_score: score as f64,
        outright_win: win,
        tied_first: tie,
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alternative {
    pub action: Action,
    pub description: String,
    pub metrics: Option<Metrics>,
    pub samples: u64,
    /// Simultaneous 95% sampling interval for utility under a fixed policy.
    pub interval: Option<[f64; 2]>,
    pub baseline_comparison: Option<BaselineComparison>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchPhase {
    MctsTraining,
    MctsValidation,
    Exact,
    Rollout,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchProgress {
    pub phase: SearchPhase,
    pub max_decision_depth: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<SearchProgress>,
    pub status: String,
    pub method: String,
    pub settings: Settings,
    pub best_action: Option<Action>,
    pub recommendation_basis: String,
    pub baseline_action: Option<Action>,
    pub sampled_best_action: Option<Action>,
    pub confidence: String,
    pub selection_reason: String,
    pub baseline_comparison: Option<BaselineComparison>,
    pub alternatives: Vec<Alternative>,
    pub forecast: Option<Metrics>,
    pub elapsed_ms: u128,
    pub nodes: usize,
    pub samples: u64,
    pub stop_reason: String,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaselineComparison {
    /// Sampled leader minus baseline utility, using paired full-game outcomes.
    pub mean_gain: f64,
    pub interval: [f64; 2],
    pub samples: u64,
}
impl Report {
    fn new(settings: &Settings) -> Self {
        Self {
            progress: None,
            status: "incomplete".into(),
            method: String::new(),
            settings: settings.clone(),
            best_action: None,
            recommendation_basis: "none".into(),
            baseline_action: None,
            sampled_best_action: None,
            confidence: "insufficient".into(),
            selection_reason: String::new(),
            baseline_comparison: None,
            alternatives: vec![],
            forecast: None,
            elapsed_ms: 0,
            nodes: 0,
            samples: 0,
            stop_reason: String::new(),
            note: String::new(),
        }
    }
    pub fn text(&self) -> String {
        let mut text = if let Some(a) = self.best_action {
            let name = self
                .alternatives
                .iter()
                .find(|x| x.action == a)
                .map(|x| x.description.clone())
                .unwrap_or_else(|| a.to_string());
            format!("Suggested: {name}\n")
        } else {
            "No recommended action (forecast or incomplete analysis)\n".into()
        };
        text.push_str(&format!(
            "{} | {} | {:?} | {}\n",
            self.status, self.method, self.settings.objective, self.recommendation_basis
        ));
        text.push_str(&format!(
            "Confidence: {}. {}\n",
            self.confidence, self.selection_reason
        ));
        if let Some(action) = self.baseline_action {
            text.push_str(&format!("Baseline: {action}\n"));
        }
        if let Some(action) = self.sampled_best_action {
            text.push_str(&format!(
                "Highest sampled estimate (paired ranking): {action}\n"
            ));
        }
        if let Some(c) = &self.baseline_comparison {
            text.push_str(&format!("Sampled leader minus baseline: {:+.4} utility; paired 95% interval [{:+.4}, {:+.4}], {} pairs\n", c.mean_gain, c.interval[0], c.interval[1], c.samples));
        }
        if let Some(m) = self.forecast {
            text.push_str(&format!(
                "Forecast: utility {:.4}, score {:.2}, outright {:.1}%, tied first {:.1}%\n",
                m.utility,
                m.expected_score,
                100. * m.outright_win,
                100. * m.tied_first
            ));
        }
        for a in &self.alternatives {
            if let Some(m) = a.metrics {
                text.push_str(&format!(
                    "{}: utility {:.4}, score {:.2}, win {:.1}%, tie {:.1}%, n={}{}\n",
                    a.description,
                    m.utility,
                    m.expected_score,
                    100. * m.outright_win,
                    100. * m.tied_first,
                    a.samples,
                    a.interval
                        .map(|[l, h]| format!(", interval [{l:.4}, {h:.4}]"))
                        .unwrap_or_default()
                ));
            } else {
                text.push_str(&format!("{}: unassessed\n", a.description));
            }
            if let Some(c) = &a.baseline_comparison {
                text.push_str(&format!(
                    "  Paired gain vs baseline {:+.4} [{:+.4}, {:+.4}], {} pairs\n",
                    c.mean_gain, c.interval[0], c.interval[1], c.samples
                ));
            }
        }
        text.push_str(&format!(
            "{} ms, {} nodes, {} samples; {}",
            self.elapsed_ms, self.nodes, self.samples, self.stop_reason
        ));
        text.push_str(&format!("\n{}", self.note));
        text
    }
}
#[derive(Clone, Default)]
pub struct Cancel(pub Arc<AtomicBool>);
impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
pub(crate) struct Budget {
    pub start: Instant,
    pub time_ms: u64,
    pub cancel: Cancel,
}
impl Budget {
    pub fn stopped(&self) -> bool {
        self.cancel.cancelled()
            || (self.time_ms > 0 && self.start.elapsed().as_millis() >= self.time_ms as u128)
    }
}
pub fn describe(p: &Position, a: Action) -> String {
    match (p.phase, a) {
        (Phase::Decision { dice, .. }, Action::Write { category }) => format!(
            "Write {} in {}{}",
            crate::rules::score(&p.players[p.active_player].sheet, dice, category),
            crate::rules::NAMES[category],
            if crate::rules::extra_eligible(&p.players[p.active_player].sheet, dice) {
                " (+50 extra Kniffel)"
            } else {
                ""
            }
        ),
        _ => a.to_string(),
    }
}
pub fn analyze(p: &Position, settings: &Settings, cancel: Cancel) -> Result<Report> {
    p.validate()?;
    settings.validate(p)?;
    let start = Instant::now();
    let mut report = Report::new(settings);
    if matches!(p.phase, Phase::Finished) {
        report.status = "exact_under_model".into();
        report.confidence = "exact_under_model".into();
        report.selection_reason = "Finished match; no action needed.".into();
        report.method = "terminal".into();
        report.forecast = Some(terminal(p, settings));
        report.stop_reason = "Game finished".into();
        return Ok(report);
    }
    let budget = Budget {
        start,
        time_ms: settings.time_ms,
        cancel,
    };
    crate::policy::warm_up();
    if settings.method == Method::Exact || (settings.method == Method::Auto && p.open() <= 3) {
        // Auto spends at most a quarter of its wall budget on exact search.
        let exact_ms = if settings.method == Method::Auto && settings.time_ms > 0 {
            (settings.time_ms / 4).max(1)
        } else {
            settings.time_ms
        };
        let exact_budget = Budget {
            start,
            time_ms: exact_ms,
            cancel: budget.cancel.clone(),
        };
        let mut exact = endgame::Exact::new(settings, &exact_budget);
        let result = exact.root(p);
        report.progress = Some(SearchProgress {
            phase: SearchPhase::Exact,
            max_decision_depth: exact.max_depth,
        });
        match result {
            Some((alternatives, forecast)) => {
                report.status = "exact_under_model".into();
                report.confidence = "exact_under_model".into();
                report.selection_reason =
                    "All relevant branches evaluated under the declared opponent policies.".into();
                report.method = "backward induction".into();
                report.alternatives = alternatives;
                report.forecast = forecast;
                report.best_action = report.alternatives.first().map(|a| a.action);
                report.recommendation_basis = if report.best_action.is_some() {
                    "exact"
                } else {
                    "none"
                }
                .into();
                report.stop_reason = "All relevant branches solved".into();
                report.note = format!(
                    "Exact within f64 tolerance, against {}. Opponent turns show forecast only.",
                    settings.policy.name()
                );
                report
                    .note
                    .push_str(" Per-player overrides are recorded in settings.");
                report.note.push_str(exact.policy_note());
                report.nodes = exact.nodes;
                report.elapsed_ms = start.elapsed().as_millis();
                return Ok(report);
            }
            None => {
                report.nodes = exact.nodes;
                if settings.method == Method::Exact {
                    report.method = "backward induction".into();
                    report.stop_reason =
                        "Exact search budget exhausted or cancelled; no exact recommendation"
                            .into();
                    report.elapsed_ms = start.elapsed().as_millis();
                    return Ok(report);
                }
            }
        }
    }
    simulation::run(p, settings, &budget, &mut report);
    report.elapsed_ms = start.elapsed().as_millis();
    Ok(report)
}
