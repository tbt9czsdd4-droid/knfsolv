use super::{
    Alternative, BaselineComparison, Budget, Method, Metrics, Objective, Report, Settings,
    describe, terminal,
};
use crate::{
    dice::Dice,
    policy,
    rules::Action,
    state::{Phase, Position},
};
use rand::{Rng, RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

#[derive(Clone, Default)]
struct Stat {
    sum: Metrics,
    n: u64,
}
impl Stat {
    fn add(&mut self, m: Metrics) {
        self.sum.add(m);
        self.n += 1;
    }
    fn mean(&self) -> Option<Metrics> {
        (self.n > 0).then(|| self.sum.scaled(1. / self.n as f64))
    }
}

fn scale(s: &Settings) -> f64 {
    if s.objective == Objective::ExpectedScore {
        2000.
    } else {
        1.
    }
}
// Union bound over actions and every sample size: alpha/(K*n*(n+1)).
// Selection uses only previous rounds; fresh streams give conditionally IID samples.
fn radius(n: u64, count: usize, range: f64) -> f64 {
    if n == 0 {
        return f64::INFINITY;
    }
    range * ((2. * count as f64 * n as f64 * (n as f64 + 1.) / 0.05).ln() / (2. * n as f64)).sqrt()
}
fn utility_order(a: &Stat, b: &Stat) -> std::cmp::Ordering {
    let a = a.mean().unwrap_or_default();
    let b = b.mean().unwrap_or_default();
    a.utility
        .total_cmp(&b.utility)
        .then_with(|| a.expected_score.total_cmp(&b.expected_score))
}
/// Initial coverage, then race the leading estimates/upper bounds plus baseline
/// and a rotating explorer. No heuristic elimination; every action is revisited.
fn allocation(
    stats: &[Stat],
    paired: &[(f64, u64)],
    base: Option<usize>,
    round: u64,
    range: f64,
) -> Vec<usize> {
    if stats.iter().any(|s| s.n < 8) {
        return (0..stats.len()).collect();
    }
    let mut chosen = vec![];
    if let Some(base) = base {
        chosen.push(base);
    }
    let mut ranked: Vec<_> = (0..stats.len()).collect();
    let estimate = |i: usize| {
        if paired[i].1 > 0 {
            paired[i].0 / paired[i].1 as f64
        } else {
            stats[i].mean().unwrap_or_default().utility
        }
    };
    ranked.sort_by(|&a, &b| {
        estimate(b)
            .total_cmp(&estimate(a))
            .then_with(|| utility_order(&stats[b], &stats[a]))
            .then_with(|| a.cmp(&b))
    });
    chosen.extend(ranked.iter().take(2).copied());
    ranked.sort_by(|&a, &b| {
        let bound = |i: usize| estimate(i) + radius(paired[i].1, stats.len(), 2. * range);
        bound(b).total_cmp(&bound(a)).then_with(|| a.cmp(&b))
    });
    chosen.extend(ranked.iter().take(2).copied());
    chosen.push(round as usize % stats.len());
    chosen.sort_unstable();
    chosen.dedup();
    chosen
}
struct Node {
    actions: Vec<Action>,
    stats: Vec<Stat>,
    visits: u64,
}
type TreeKey = (Position, usize);
type Tree = HashMap<TreeKey, Node>;
fn key(p: &Position, s: &Settings, depth: usize) -> TreeKey {
    // For expected-score optimization opponents cannot change the perspective policy.
    let position = if s.objective == Objective::ExpectedScore {
        let mut k = Position::new(1);
        k.players[0].sheet = p.players[s.perspective].sheet;
        k.phase = p.phase;
        k
    } else {
        p.clone()
    };
    (position, if s.max_depth == 0 { 0 } else { depth })
}
fn streams(seed: u64, sample: u64) -> [ChaCha8Rng; 8] {
    let mut master = ChaCha8Rng::seed_from_u64(seed);
    master.set_stream(sample);
    let mut sample_seed = [0_u8; 32];
    master.fill_bytes(&mut sample_seed);
    std::array::from_fn(|i| {
        let mut r = ChaCha8Rng::from_seed(sample_seed);
        r.set_stream(i as u64 * 64);
        r
    })
}
fn roll(p: &mut Position, rng: &mut ChaCha8Rng) {
    let (mut d, r) = match p.phase {
        Phase::BeforeFirstRoll => (Dice::default(), 2),
        Phase::AwaitingRoll {
            kept,
            rerolls_after,
        } => (kept, rerolls_after),
        _ => return,
    };
    // Keep future turns/rolls paired even when root alternatives use different numbers of dice.
    let turn = 13 - p.players[p.active_player].sheet.open();
    rng.set_stream(p.active_player as u64 * 64 + turn as u64 * 4 + (2 - r) as u64);
    rng.set_word_pos(0);
    for _ in d.len()..5 {
        d.0[rng.gen_range(0..6)] += 1;
    }
    p.phase = Phase::Decision {
        dice: d,
        rerolls_left: r,
    };
}
fn frozen_action(
    p: &Position,
    s: &Settings,
    tree: &Tree,
    baseline: Action,
    depth: usize,
) -> Action {
    let Some(node) = tree.get(&key(p, s, depth)) else {
        return baseline;
    };
    // An exploratory win after one visit is not a reliable policy improvement.
    // Freeze under-sampled nodes to the declared baseline, not their lucky maximum.
    if node.stats.iter().any(|x| x.n < 32) {
        return baseline;
    }
    let chosen = node
        .stats
        .iter()
        .enumerate()
        .filter(|(_, x)| x.n > 0)
        .max_by(|(ia, a), (ib, b)| {
            a.mean()
                .unwrap()
                .utility
                .total_cmp(&b.mean().unwrap().utility)
                .then_with(|| {
                    a.mean()
                        .unwrap()
                        .expected_score
                        .total_cmp(&b.mean().unwrap().expected_score)
                })
                .then_with(|| ib.cmp(ia))
        })
        .map(|(i, _)| node.actions[i])
        .unwrap_or(baseline);
    let base = node.actions.iter().position(|&a| a == baseline).unwrap();
    let candidate = node.actions.iter().position(|&a| a == chosen).unwrap();
    // Nonstationary exploratory returns: a stability heuristic, NOT a confidence
    // interval. All reported intervals come from independent root validation.
    let margin = scale(s)
        * (2. * (node.actions.len() as f64).ln()
            / node.stats[candidate].n.min(node.stats[base].n) as f64)
            .sqrt();
    if node.stats[candidate].mean().unwrap().utility - node.stats[base].mean().unwrap().utility
        > margin
    {
        chosen
    } else {
        baseline
    }
}
fn train_action(
    p: &Position,
    s: &Settings,
    tree: &mut Tree,
    path: &mut Vec<(TreeKey, usize)>,
    leaf: &mut bool,
    depth: usize,
) -> Option<Action> {
    if *leaf {
        return None;
    }
    let k = key(p, s, depth);
    if !tree.contains_key(&k) {
        if tree.len() < s.max_nodes {
            let actions = p.actions();
            tree.insert(
                k,
                Node {
                    stats: vec![Stat::default(); actions.len()],
                    actions,
                    visits: 0,
                },
            );
        }
        *leaf = true;
        return None;
    }
    let node = tree.get(&k).unwrap();
    let index = node.stats.iter().position(|x| x.n == 0).unwrap_or_else(|| {
        let scale = if s.objective == Objective::ExpectedScore {
            2000.
        } else {
            1.
        };
        (0..node.actions.len())
            .max_by(|&a, &b| {
                let uct = |i: usize| {
                    node.stats[i].mean().unwrap().utility / scale
                        + (2. * (node.visits.max(1) as f64).ln() / node.stats[i].n as f64).sqrt()
                };
                uct(a).total_cmp(&uct(b)).then_with(|| b.cmp(&a))
            })
            .unwrap()
    });
    let action = node.actions[index];
    path.push((k, index));
    Some(action)
}
/// A suspended full-game sample. Pausing retains RNG streams, path and partial position.
struct GameSample {
    position: Position,
    rngs: [ChaCha8Rng; 8],
    depth: usize,
    path: Vec<(TreeKey, usize)>,
    leaf: bool,
    training: bool,
}
impl GameSample {
    fn new(
        initial: &Position,
        s: &Settings,
        forced: Option<Action>,
        sample: u64,
        training: bool,
    ) -> Self {
        let mut position = initial.clone();
        let depth = usize::from(
            forced.is_some()
                && initial.active_player == s.perspective
                && matches!(initial.phase, Phase::Decision { .. }),
        );
        if let Some(action) = forced {
            position.apply(action).expect("legal root action");
        }
        Self {
            position,
            rngs: streams(s.seed, sample),
            depth,
            path: vec![],
            leaf: false,
            training,
        }
    }
    fn step(
        &mut self,
        s: &Settings,
        policy: &mut policy::Evaluator,
        tree: &mut Tree,
        max_depth: &mut usize,
    ) -> Option<Metrics> {
        let p = &mut self.position;
        match p.phase {
            Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } => {
                let active = p.active_player;
                roll(p, &mut self.rngs[active]);
            }
            Phase::Decision { dice, rerolls_left } => {
                let baseline = policy.choose(
                    &p.players[p.active_player].sheet,
                    dice,
                    rerolls_left,
                    s.model(p.active_player),
                );
                let perspective = p.active_player == s.perspective;
                let limited = s.max_depth > 0 && self.depth >= s.max_depth;
                let action = if !perspective || limited {
                    baseline
                } else if self.training {
                    {
                        if !self.leaf
                            && (tree.contains_key(&key(p, s, self.depth))
                                || tree.len() < s.max_nodes)
                        {
                            *max_depth = (*max_depth).max(self.depth + 1);
                        }
                        train_action(p, s, tree, &mut self.path, &mut self.leaf, self.depth)
                    }
                    .unwrap_or(baseline)
                } else {
                    frozen_action(p, s, tree, baseline, self.depth)
                };
                self.depth += usize::from(perspective);
                p.apply(action).expect("policy action is legal");
            }
            Phase::Finished => (),
        }
        if !matches!(p.phase, Phase::Finished) {
            return None;
        }
        let result = terminal(p, s);
        for (key, i) in self.path.drain(..) {
            let node = tree.get_mut(&key).unwrap();
            node.visits += 1;
            node.stats[i].add(result);
        }
        Some(result)
    }
}

pub(super) struct SampleSearch {
    position: Position,
    settings: Settings,
    policy: policy::Evaluator,
    tree: Tree,
    tree_mode: bool,
    max_depth: usize,
    training_n: u64,
    training_limit: u64,
    training_time: std::time::Duration,
    training_cap: Option<std::time::Duration>,
    frozen: bool,
    actions: Vec<Action>,
    baseline_root: Option<Action>,
    baseline_index: Option<usize>,
    stats: Vec<Stat>,
    paired: Vec<(f64, u64)>,
    n: u64,
    round_id: u64,
    selected: Vec<usize>,
    round: Vec<(usize, Metrics)>,
    current: Option<GameSample>,
}
impl SampleSearch {
    pub(super) fn new(p: &Position, s: &Settings, coaching: bool) -> Self {
        let tree_mode = matches!(s.method, Method::Auto | Method::Mcts);
        let mut policy =
            policy::Evaluator::configured(s.cache.as_deref(), s.baseline_model.as_deref());
        policy.set_cache_limit(s.max_nodes.min(8192));
        let actions =
            if p.active_player == s.perspective && !matches!(p.phase, Phase::AwaitingRoll { .. }) {
                p.actions()
            } else {
                vec![]
            };
        let baseline_root = match p.phase {
            Phase::Decision { dice, rerolls_left } if p.active_player == s.perspective => {
                Some(policy.choose(
                    &p.players[p.active_player].sheet,
                    dice,
                    rerolls_left,
                    s.model(p.active_player),
                ))
            }
            Phase::BeforeFirstRoll if p.active_player == s.perspective => Some(Action::RollAll),
            _ => None,
        };
        let baseline_index = actions.iter().position(|a| Some(*a) == baseline_root);
        let count = actions.len().max(1);
        let training_limit = if coaching {
            (s.simulations / 2).min(2000)
        } else {
            s.simulations / 2
        };
        let training_cap = if coaching {
            Some(std::time::Duration::from_millis(if s.time_ms > 0 {
                (s.time_ms / 2).min(500)
            } else {
                500
            }))
        } else {
            (s.time_ms > 0).then(|| std::time::Duration::from_millis(s.time_ms / 2))
        };
        Self {
            position: p.clone(),
            settings: s.clone(),
            policy,
            tree: Tree::new(),
            tree_mode,
            max_depth: 0,
            training_n: 0,
            training_limit,
            training_time: std::time::Duration::ZERO,
            training_cap,
            frozen: !tree_mode,
            actions,
            baseline_root,
            baseline_index,
            stats: vec![Stat::default(); count],
            paired: vec![(0., 0); count],
            n: 0,
            round_id: 0,
            selected: vec![],
            round: vec![],
            current: None,
        }
    }
    pub(super) fn samples(&self) -> u64 {
        self.n + self.training_n
    }
    pub(super) fn cache_entries(&self) -> (usize, usize) {
        self.policy.cache_entries()
    }
    /// One game step. A completed game is counted exactly once; pairs commit only after a full round.
    pub(super) fn step(&mut self) {
        let start = web_time::Instant::now();
        if !self.frozen
            && (self.training_n >= self.training_limit
                || self
                    .training_cap
                    .is_some_and(|cap| self.training_time >= cap))
        {
            self.frozen = true;
            self.current = None; // An unfinished training game contributed no counted evidence.
        }
        if self.current.is_none() {
            let (forced, stream) = if !self.frozen {
                (None, self.training_n)
            } else {
                if self.selected.is_empty() {
                    self.selected = allocation(
                        &self.stats,
                        &self.paired,
                        self.baseline_index,
                        self.round_id,
                        scale(&self.settings),
                    );
                }
                (
                    self.actions.get(self.selected[self.round.len()]).copied(),
                    (1_u64 << 60) + self.round_id,
                )
            };
            self.current = Some(GameSample::new(
                &self.position,
                &self.settings,
                forced,
                stream,
                !self.frozen,
            ));
        }
        let result = self.current.as_mut().unwrap().step(
            &self.settings,
            &mut self.policy,
            &mut self.tree,
            &mut self.max_depth,
        );
        if !self.frozen {
            self.training_time += start.elapsed();
        }
        if let Some(metrics) = result {
            self.current = None;
            if !self.frozen {
                self.training_n += 1;
            } else {
                let i = self.selected[self.round.len()];
                self.stats[i].add(metrics);
                self.round.push((i, metrics));
                self.n += 1;
                if self.round.len() == self.selected.len() {
                    if let Some((_, base)) = self
                        .round
                        .iter()
                        .find(|(i, _)| Some(*i) == self.baseline_index)
                    {
                        for &(i, value) in &self.round {
                            self.paired[i].0 += value.utility - base.utility;
                            self.paired[i].1 += 1;
                        }
                    }
                    self.round.clear();
                    self.selected.clear();
                    self.round_id += 1;
                }
            }
        }
    }
    pub(super) fn snapshot(&self, provisional: bool) -> Report {
        let p = &self.position;
        let s = &self.settings;
        let mut output = Report::new(s);
        output.progress = if self.tree_mode && self.max_depth == 0 {
            None
        } else {
            Some(super::SearchProgress {
                phase: if !self.tree_mode {
                    super::SearchPhase::Rollout
                } else if self.frozen {
                    super::SearchPhase::MctsValidation
                } else {
                    super::SearchPhase::MctsTraining
                },
                max_decision_depth: self.max_depth,
            })
        };
        let report = &mut output;
        let actions = &self.actions;
        let stats = &self.stats;
        let paired = &self.paired;
        let count = stats.len();
        let baseline_root = self.baseline_root;
        let baseline_index = self.baseline_index;
        let tree_mode = self.tree_mode;
        let training_n = self.training_n;
        let n = self.n;
        report.samples = n + training_n;
        report.nodes = self.tree.len();
        report.method = if tree_mode {
            "MCTS + independent frozen-policy rollouts"
        } else {
            "fixed-policy full-game rollouts"
        }
        .into();
        report.status = if stats.iter().all(|s| s.n > 0) {
            "estimated"
        } else {
            "incomplete"
        }
        .into();
        report.note = format!(
            "Opponent/default continuation: {}. {} training and {} validation games. Intervals cover sampling error under the frozen policy, not human behavior or optimality. Per-player overrides are recorded in settings.",
            s.policy.name(),
            training_n,
            n
        );
        report.note.push_str(&self.policy.note);
        for (i, &action) in actions.iter().enumerate() {
            let st = &stats[i];
            let interval = st.mean().map(|m| {
                // Union over actions and all n >= 1 using alpha / (K*n*(n+1)).
                // This remains valid at an interrupted/adaptive stopping time.
                let range = scale(s);
                let width = radius(st.n, count, range);
                [(m.utility - width).max(0.), (m.utility + width).min(range)]
            });
            report.alternatives.push(Alternative {
                action,
                description: describe(p, action),
                metrics: st.mean(),
                samples: st.n,
                interval,
                baseline_comparison: baseline_index.map(|base| {
                    let (sum, n) = paired[i];
                    let mean = sum / n.max(1) as f64;
                    let width = if i == base {
                        0.
                    } else {
                        radius(n, count, 2. * scale(s))
                    };
                    BaselineComparison {
                        mean_gain: mean,
                        interval: [(mean - width).max(-scale(s)), (mean + width).min(scale(s))],
                        samples: n,
                    }
                }),
            });
        }
        report.alternatives.sort_by(|a, b| {
            let estimate = |a: &Alternative| {
                a.baseline_comparison
                    .as_ref()
                    .filter(|c| c.samples > 0)
                    .map(|c| c.mean_gain)
                    .or_else(|| a.metrics.map(|m| m.utility))
                    .unwrap_or(f64::NEG_INFINITY)
            };
            estimate(b).total_cmp(&estimate(a)).then_with(|| {
                b.metrics
                    .map(|m| m.utility)
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&a.metrics.map(|m| m.utility).unwrap_or(f64::NEG_INFINITY))
                    .then_with(|| {
                        b.metrics
                            .map(|m| m.expected_score)
                            .unwrap_or(f64::NEG_INFINITY)
                            .total_cmp(
                                &a.metrics
                                    .map(|m| m.expected_score)
                                    .unwrap_or(f64::NEG_INFINITY),
                            )
                    })
            })
        });
        if actions.is_empty() {
            report.forecast = stats[0].mean();
        } else {
            report.baseline_action = baseline_root;
            report.sampled_best_action = report
                .alternatives
                .first()
                .filter(|a| a.metrics.is_some())
                .map(|a| a.action);
            report.forecast = report.alternatives.first().and_then(|a| a.metrics);
            // An incomplete action comparison is not a recommendation.
            if report.status == "estimated" {
                report.best_action = report.alternatives.first().map(|a| a.action);
                report.recommendation_basis = "sampled_best".into();
                report.confidence = "low".into();
                report.selection_reason = "Highest paired improvement estimate (shared baseline outcomes); alternatives may still be better. Compute longer to reduce sampling uncertainty.".into();
                if report.best_action == baseline_root {
                    report.baseline_comparison = report.alternatives[0].baseline_comparison.clone();
                    report.selection_reason = "The baseline is also the highest paired estimate. Other actions remain plausible unless intervals separate them.".into();
                }
                if report.best_action != baseline_root
                    && let Some(base) = baseline_root
                {
                    let candidate = actions
                        .iter()
                        .position(|a| Some(*a) == report.best_action)
                        .unwrap();
                    let (sum, n) = paired[candidate];
                    let mean = sum / n.max(1) as f64;
                    // Paired differences lie in [-range, range]. Bounds are simultaneous
                    // over candidates/sample sizes, so choosing the leader is covered.
                    let range = scale(s);
                    let width = radius(n, count, 2. * range);
                    let interval = [(mean - width).max(-range), (mean + width).min(range)];
                    report.baseline_comparison = Some(BaselineComparison {
                        mean_gain: mean,
                        interval,
                        samples: n,
                    });
                    if s.conservative_advice && interval[0] <= 0. {
                        report.best_action = Some(base);
                        report.recommendation_basis = "baseline_uncertain_advantage".into();
                        report.selection_reason = "Conservative mode: provisional baseline, not the sampled leader. Improvement over baseline is unresolved; this does not establish that baseline is better.".into();
                    }
                }
                if stats.iter().any(|x| x.n < 8) {
                    report.best_action = baseline_root;
                    report.recommendation_basis = "baseline_small_sample".into();
                    report.confidence = "insufficient".into();
                    report.selection_reason = "Provisional baseline only: fewer than eight samples for at least one action. Insufficient analysis to rank the alternatives reliably.".into();
                    report.note.push_str(" Too few validation samples per action; advice falls back to the declared baseline policy.");
                }
                report.forecast = report
                    .alternatives
                    .iter()
                    .find(|a| Some(a.action) == report.best_action)
                    .and_then(|a| a.metrics);
            }
            if report.alternatives.len() > 1 {
                let top = report.alternatives[0].interval;
                let separated = top.is_some_and(|[low, _]| {
                    report.alternatives[1..]
                        .iter()
                        .all(|a| a.interval.is_some_and(|[_, high]| low > high))
                });
                if !separated {
                    report
                        .note
                        .push_str(" Alternatives are statistically unresolved.");
                } else if report.recommendation_basis == "sampled_best" {
                    report.confidence = "separated_under_model".into();
                    report.selection_reason = "Simultaneous sampling intervals separate the leader from all alternatives under the frozen continuation policies, not against optimal or human play.".into();
                }
            }
        }
        if provisional && report.best_action.is_none() && baseline_root.is_some() {
            report.best_action = baseline_root;
            report.baseline_action = baseline_root;
            report.recommendation_basis = "baseline_pending_validation".into();
            report.confidence = "insufficient".into();
            report.selection_reason = "Provisional baseline while analysis develops; no sampled ranking is established yet.".into();
        }
        output
    }
}

pub(super) fn run(p: &Position, s: &Settings, budget: &Budget, report: &mut Report) {
    let exact_nodes = report.nodes;
    let exact_progress = report.progress;
    let mut search = SampleSearch::new(p, s, false);
    while search.samples() < s.simulations && !budget.stopped() {
        search.step();
    }
    *report = search.snapshot(false);
    report.nodes += exact_nodes;
    if let Some(p) = exact_progress {
        report.note.push_str(&format!(
            " Previous exact attempt: depth {} (incomplete).",
            p.max_decision_depth
        ));
    }
    report.stop_reason = if budget.cancel.cancelled() {
        "Cancelled"
    } else if budget.stopped() {
        "Time budget reached"
    } else {
        "Simulation budget reached"
    }
    .into();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adaptive_allocation_covers_then_focuses_without_abandoning_actions() {
        let mut stats = vec![Stat::default(); 20];
        let paired = vec![(0., 0); 20];
        assert_eq!(allocation(&stats, &paired, Some(19), 0, 1.).len(), 20);
        for (i, s) in stats.iter_mut().enumerate() {
            for _ in 0..32 {
                s.add(Metrics {
                    utility: if i == 0 { 1. } else { 0. },
                    ..Metrics::default()
                });
            }
        }
        let mut seen = [false; 20];
        for round in 0..20 {
            let selected = allocation(&stats, &paired, Some(19), round, 1.);
            assert!(selected.contains(&0));
            assert!(selected.contains(&19));
            assert!(selected.len() <= 6);
            for i in selected {
                seen[i] = true;
            }
        }
        assert!(seen.into_iter().all(|v| v));
        assert!(radius(1000, 20, 2.) < radius(100, 20, 2.));
        assert!(radius(0, 20, 2.).is_infinite());
    }
}
