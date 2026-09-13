use super::{Alternative, Budget, Metrics, Settings, describe, terminal};
use crate::{
    dice::{Dice, tables},
    policy,
    rules::Action,
    state::{Phase, Position},
};
use std::collections::HashMap;

pub struct Exact<'a> {
    settings: &'a Settings,
    budget: &'a Budget,
    cache: HashMap<Position, Metrics>,
    pub nodes: usize,
    pub max_depth: usize,
    policy: policy::Evaluator,
}
impl<'a> Exact<'a> {
    pub(super) fn policy_note(&self) -> &str {
        &self.policy.note
    }
    pub(super) fn new(settings: &'a Settings, budget: &'a Budget) -> Self {
        Self {
            settings,
            budget,
            cache: HashMap::new(),
            nodes: 0,
            max_depth: 0,
            policy: policy::Evaluator::configured(
                settings.cache.as_deref(),
                settings.baseline_model.as_deref(),
            ),
        }
    }
    fn value(&mut self, p: &Position, depth: usize) -> Option<Metrics> {
        if self.budget.stopped() {
            return None;
        }
        if matches!(p.phase, Phase::Finished) {
            return Some(terminal(p, self.settings));
        }
        if let Some(v) = self.cache.get(p) {
            return Some(*v);
        }
        if self.nodes >= self.settings.max_nodes {
            return None;
        }
        self.nodes += 1;
        let depth = depth
            + usize::from(
                p.active_player == self.settings.perspective
                    && matches!(p.phase, Phase::Decision { .. }),
            );
        self.max_depth = self.max_depth.max(depth);
        let value = match p.phase {
            Phase::BeforeFirstRoll => self.chance(p, Dice::default(), 2, depth)?,
            Phase::AwaitingRoll {
                kept,
                rerolls_after,
            } => self.chance(p, kept, rerolls_after, depth)?,
            Phase::Decision { dice, rerolls_left } => {
                if p.active_player == self.settings.perspective {
                    let mut best = Metrics {
                        utility: f64::NEG_INFINITY,
                        ..Metrics::default()
                    };
                    for a in p.actions() {
                        let v = self.action(p, a, depth)?;
                        if v.utility > best.utility + 1e-12 {
                            best = v;
                        }
                    }
                    best
                } else {
                    let action = self.policy.choose(
                        &p.players[p.active_player].sheet,
                        dice,
                        rerolls_left,
                        self.settings.model(p.active_player),
                    );
                    self.action(p, action, depth)?
                }
            }
            Phase::Finished => unreachable!(),
        };
        self.cache.insert(p.clone(), value);
        Some(value)
    }
    fn chance(&mut self, p: &Position, kept: Dice, r: u8, depth: usize) -> Option<Metrics> {
        let mut sum = Metrics::default();
        for &(d, probability) in &tables().transitions[tables().partial_id(kept)] {
            let mut next = p.clone();
            next.phase = Phase::Decision {
                dice: tables().full[d],
                rerolls_left: r,
            };
            sum.add(self.value(&next, depth)?.scaled(probability));
        }
        Some(sum)
    }
    fn action(&mut self, p: &Position, a: Action, depth: usize) -> Option<Metrics> {
        if let Action::RollAll = a {
            return self.chance(p, Dice::default(), 2, depth);
        }
        let mut next = p.clone();
        next.apply(a).expect("generated legal action");
        self.value(&next, depth)
    }
    pub(super) fn root(&mut self, p: &Position) -> Option<(Vec<Alternative>, Option<Metrics>)> {
        if p.active_player != self.settings.perspective
            || matches!(p.phase, Phase::AwaitingRoll { .. })
        {
            return Some((vec![], Some(self.value(p, 0)?)));
        }
        let depth = usize::from(matches!(p.phase, Phase::Decision { .. }));
        self.max_depth = self.max_depth.max(depth);
        let mut alternatives = Vec::new();
        for action in p.actions() {
            alternatives.push(Alternative {
                action,
                description: describe(p, action),
                metrics: Some(self.action(p, action, depth)?),
                samples: 0,
                interval: None,
                baseline_comparison: None,
            });
        }
        // Quantization gives a transitive comparator while preserving deterministic ties.
        alternatives.sort_by(|a, b| {
            (b.metrics.unwrap().utility * 1e12)
                .round()
                .total_cmp(&(a.metrics.unwrap().utility * 1e12).round())
        });
        let forecast = alternatives.first().and_then(|a| a.metrics);
        Some((alternatives, forecast))
    }
}
