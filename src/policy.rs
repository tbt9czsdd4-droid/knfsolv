//! A fast, declared heuristic: exact single-category turn values minus opportunity costs.
use crate::{
    dice::{Dice, tables},
    rules::{self, Action},
    solver::turn::TurnTable,
    state::Sheet,
};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub struct Evaluator {
    cache: Option<crate::solver::solitaire::Cache>,
    continuation: continuation::Evaluator,
    pub note: String,
}
impl Evaluator {
    pub(crate) fn set_cache_limit(&mut self, limit: usize) {
        self.continuation.set_cache_limit(limit);
    }
    pub(crate) fn cache_entries(&self) -> (usize, usize) {
        self.continuation.cache_entries()
    }
    pub fn new(path: Option<&std::path::Path>) -> Self {
        Self::configured(path, None)
    }
    pub fn configured(
        path: Option<&std::path::Path>,
        model_path: Option<&std::path::Path>,
    ) -> Self {
        let (cache, note) = match path {
            None => (None, String::new()),
            Some(p) => match crate::solver::solitaire::Cache::load(p) {
                Ok(c) => {
                    let note = format!(" Solitaire cache: {} completed states.", c.len());
                    (Some(c), note)
                }
                Err(e) => (None, format!(" Cache ignored: {e}.")),
            },
        };
        let mut note = note;
        let model = match model_path {
            Some(path) => match continuation::Model::load(path) {
                Ok(model) => std::sync::Arc::new(model),
                Err(e) => {
                    note.push_str(&format!(" Continuation model ignored: {e}."));
                    continuation::Model::bundled()
                }
            },
            None => continuation::Model::bundled(),
        };
        note.push_str(&format!(" Continuation v2: {} games/iteration, {} policy-evaluation iterations, seed {}. Marginal category model is approximate; last two open rows use exact expected-score continuation.", model.games, model.iterations, model.seed));
        Self {
            cache,
            continuation: continuation::Evaluator::new(model),
            note,
        }
    }
    pub fn choose(&mut self, sheet: &Sheet, dice: Dice, r: u8, model: Policy) -> Action {
        if model == Policy::Opportunity {
            return self
                .continuation
                .choose(sheet, dice, r, self.cache.as_ref());
        }
        legacy_choose(sheet, dice, r, model)
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    #[default]
    Opportunity,
    LegacyOpportunity,
    Greedy,
}
impl Policy {
    pub fn name(self) -> &'static str {
        match self {
            Self::Opportunity => "continuation_v2 (learned sheet value; exact last two rows)",
            Self::LegacyOpportunity => "legacy_opportunity (category opportunity costs)",
            Self::Greedy => "greedy_v1 (score immediately)",
        }
    }
}
fn category_tables() -> &'static [Vec<TurnTable>; 2] {
    static T: OnceLock<[Vec<TurnTable>; 2]> = OnceLock::new();
    T.get_or_init(|| {
        std::array::from_fn(|extra| {
            (0..13)
                .map(|c| {
                    TurnTable::build(|d| {
                        let score = if extra == 1 && d.0.contains(&5) {
                            rules::MAXIMA[c] + 50
                        } else {
                            rules::ordinary_score(d, c)
                        };
                        vec![(Action::Write { category: c }, score as f64)]
                    })
                })
                .collect()
        })
    })
}
pub fn warm_up() {
    let _ = category_tables();
}

fn adjustments(sheet: &Sheet, category: usize) -> (f64, f64) {
    const COSTS: [f64; 13] = [2., 5., 8., 11., 14., 17., 19., 13., 17., 25., 30., 14., 22.];
    let mut weight = 1.0;
    if category < 6 && sheet.upper() < 63 {
        let typical: f64 = (0..6)
            .filter(|&c| sheet.cells[c].is_none())
            .map(|c| 3.0 * (c + 1) as f64)
            .sum();
        let deficit = 63.0 - sheet.upper() as f64;
        if deficit <= typical + 12.0 {
            weight += 0.65;
        }
    }
    (weight, COSTS[category] * weight)
}
/// The declared legacy policy; also retained as the continuation model's training seed.
pub fn legacy_choose(sheet: &Sheet, dice: Dice, rerolls: u8, model: Policy) -> Action {
    let mut best = Action::RollAll;
    let mut value = f64::NEG_INFINITY;
    for action in rules::scoring_actions(sheet, dice) {
        let c = match action {
            Action::Write { category } | Action::CrossOut { category } => category,
            _ => unreachable!(),
        };
        let points = rules::score(sheet, dice, c) as f64
            + if rules::extra_eligible(sheet, dice) {
                50.
            } else {
                0.
            };
        let (weight, cost) = if model == Policy::Greedy {
            (1., 0.)
        } else {
            adjustments(sheet, c)
        };
        let candidate = weight * points - cost;
        if candidate > value {
            value = candidate;
            best = action;
        }
    }
    if rerolls > 0 && model != Policy::Greedy {
        let t = tables();
        let d = t.full_id(dice);
        let extra = usize::from(sheet.cells[11] == Some(50));
        let cats = &category_tables()[extra];
        for (c, cat) in cats.iter().enumerate() {
            if sheet.cells[c].is_some() {
                continue;
            }
            let (weight, cost) = adjustments(sheet, c);
            for &k in &t.keeps[d] {
                let candidate = weight * cat.keep_values[rerolls as usize - 1][k] - cost;
                if candidate > value + 1e-12 {
                    value = candidate;
                    best = Action::Reroll { keep: t.partial[k] };
                }
            }
        }
    }
    best
}

pub mod continuation;

pub fn choose(sheet: &Sheet, dice: Dice, rerolls: u8, model: Policy) -> Action {
    thread_local! {
        static EVALUATOR: std::cell::RefCell<Evaluator> = std::cell::RefCell::new(Evaluator::new(None));
    }
    EVALUATOR.with(|e| e.borrow_mut().choose(sheet, dice, rerolls, model))
}
