//! Approximate policy evaluation from completed-game category distributions.
//! It is not an exact multiplayer value function. Dice decisions remain exact
//! within a turn relative to these continuation values (including category switching).
use crate::{
    Error, Result,
    dice::{Dice, tables},
    rules::{self, Action},
    solver::{solitaire::Cache, turn::TurnTable},
    state::Sheet,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::{Arc, OnceLock},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub version: u32,
    pub rules: String,
    pub games: u64,
    pub seed: u64,
    pub iterations: usize,
    /// Histogram for each category, separated by extra-Kniffel eligibility.
    pub counts: Vec<Vec<Vec<u64>>>,
    pub eligible_turns: u64,
    pub extra_events: u64,
}
impl Model {
    pub fn validate(&self) -> Result<()> {
        if self.version != 2
            || self.rules != rules::RULES_VERSION
            || self.games == 0
            || self.iterations == 0
            || self.extra_events > self.eligible_turns
            || self.counts.len() != 2
            || self.counts.iter().any(|group| {
                group.len() != 13
                    || group.iter().enumerate().any(|(c, counts)| {
                        counts.len() != rules::MAXIMA[c] as usize + 1
                            || counts.iter().all(|&n| n == 0)
                    })
            })
        {
            return Err(Error::Invalid(
                "Invalid continuation model version, rules or histograms".into(),
            ));
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let model: Self = serde_json::from_reader(std::fs::File::open(path)?)?;
        model.validate()?;
        Ok(model)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        crate::persistence::save_json(path, self)
    }
    pub fn bundled() -> Arc<Self> {
        static MODEL: OnceLock<Arc<Model>> = OnceLock::new();
        MODEL
            .get_or_init(|| {
                let model: Model =
                    serde_json::from_str(include_str!("../../data/continuation-v2.json"))
                        .expect("bundled model JSON");
                model.validate().expect("bundled model validation");
                Arc::new(model)
            })
            .clone()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    mask: u16,
    upper: u16,
    eligible: bool,
}
impl Key {
    fn of(sheet: &Sheet) -> Self {
        Self {
            mask: sheet.mask(),
            upper: sheet.upper().min(63),
            eligible: sheet.cells[11] == Some(50),
        }
    }
}
type Decisions = [[Action; 252]; 3];
pub struct Evaluator {
    means: [[f64; 13]; 2],
    // Keep the 64 KiB lookup table off the worker stack while owned searches move.
    bonus: Box<[[[f64; 64]; 64]; 2]>,
    extra_rate: f64,
    decisions: HashMap<Key, Box<Decisions>>,
    order: VecDeque<Key>,
    singles: HashMap<Key, f64>,
    single_order: VecDeque<Key>,
    cache_limit: usize,
}
impl Evaluator {
    pub fn new(model: Arc<Model>) -> Self {
        let mut means = [[0.; 13]; 2];
        let mut bonus = Box::new([[[0.; 64]; 64]; 2]);
        for eligible in 0..2 {
            let pmfs: Vec<Vec<f64>> = model.counts[eligible]
                .iter()
                .enumerate()
                .map(|(c, counts)| {
                    let n = counts.iter().map(|&v| v as f64).sum::<f64>();
                    let p: Vec<f64> = counts.iter().map(|&v| v as f64 / n).collect();
                    means[eligible][c] = p
                        .iter()
                        .enumerate()
                        .map(|(score, p)| score as f64 * p)
                        .sum();
                    p
                })
                .collect();
            for (mask, probabilities) in bonus[eligible].iter_mut().enumerate() {
                let mut distribution = [0.; 64];
                distribution[0] = 1.;
                for (c, pmf) in pmfs.iter().enumerate().take(6) {
                    if mask & (1 << c) == 0 {
                        continue;
                    }
                    let mut next = [0.; 64];
                    for (sum, &p) in distribution.iter().enumerate() {
                        for (score, &q) in pmf.iter().enumerate() {
                            next[(sum + score).min(63)] += p * q;
                        }
                    }
                    distribution = next;
                }
                for (upper, probability) in probabilities.iter_mut().enumerate().take(63) {
                    *probability = distribution[63 - upper..].iter().sum();
                }
            }
        }
        Self {
            means,
            bonus,
            extra_rate: model.extra_events as f64 / model.eligible_turns.max(1) as f64,
            decisions: HashMap::new(),
            order: VecDeque::new(),
            singles: HashMap::new(),
            single_order: VecDeque::new(),
            cache_limit: 8192,
        }
    }
    pub(super) fn set_cache_limit(&mut self, limit: usize) {
        self.cache_limit = limit.max(1);
    }
    pub(super) fn cache_entries(&self) -> (usize, usize) {
        (self.decisions.len(), self.singles.len())
    }
    /// Expected *additional* score: awarded bonuses/totals are never counted twice.
    fn approximate(&self, sheet: &Sheet) -> f64 {
        let k = Key::of(sheet);
        let eligible = usize::from(k.eligible);
        let points: f64 = (0..13)
            .filter(|&c| k.mask & (1 << c) != 0)
            .map(|c| self.means[eligible][c])
            .sum();
        points
            + 35. * self.bonus[eligible][(k.mask & 63) as usize][k.upper as usize]
            + if k.eligible {
                50. * sheet.open() as f64 * self.extra_rate
            } else {
                0.
            }
    }
    fn remaining(&mut self, sheet: &Sheet) -> f64 {
        if sheet.open() == 0 {
            return 0.;
        }
        if sheet.open() > 1 {
            return self.approximate(sheet);
        }
        let key = Key::of(sheet);
        if let Some(&value) = self.singles.get(&key) {
            return value;
        }
        let table = TurnTable::build(|dice| {
            rules::scoring_actions(sheet, dice)
                .into_iter()
                .map(|a| {
                    let mut next = *sheet;
                    rules::write(&mut next, dice, a).unwrap();
                    (a, (next.total() - sheet.total()) as f64)
                })
                .collect()
        });
        let value = table.before_roll();
        if self.singles.len() >= self.cache_limit {
            self.singles.remove(&self.single_order.pop_front().unwrap());
        }
        self.single_order.push_back(key);
        self.singles.insert(key, value);
        value
    }
    pub fn turn(&mut self, sheet: &Sheet) -> TurnTable {
        // Compute V(next) only once per possible score/eligibility outcome, not per histogram.
        let mut values = HashMap::new();
        TurnTable::build(|dice| {
            rules::scoring_actions(sheet, dice)
                .into_iter()
                .map(|a| {
                    let mut next = *sheet;
                    rules::write(&mut next, dice, a).unwrap();
                    let value = *values
                        .entry(Key::of(&next))
                        .or_insert_with(|| self.remaining(&next));
                    (a, (next.total() - sheet.total()) as f64 + value)
                })
                .collect()
        })
    }
    pub fn choose(&mut self, sheet: &Sheet, dice: Dice, r: u8, exact: Option<&Cache>) -> Action {
        let key = Key::of(sheet);
        if !self.decisions.contains_key(&key) {
            let turn = exact
                .and_then(|cache| cache.turn(sheet))
                .unwrap_or_else(|| self.turn(sheet));
            let decisions = Box::new(std::array::from_fn(|r| {
                std::array::from_fn(|d| turn.actions[r][d])
            }));
            if self.decisions.len() >= self.cache_limit {
                self.decisions.remove(&self.order.pop_front().unwrap());
            }
            self.order.push_back(key);
            self.decisions.insert(key, decisions);
        }
        self.decisions[&key][r as usize][tables().full_id(dice)]
    }
}

/// Policy iteration: evaluate completed games, refit marginal distributions, then
/// improve decisions with flexible within-turn DP. First iteration uses legacy v1.
pub fn train(games: u64, seed: u64, iterations: usize) -> Result<Model> {
    use rand::{Rng, SeedableRng};
    if games == 0 || iterations == 0 {
        return Err(Error::Invalid(
            "Games and iterations must be positive".into(),
        ));
    }
    let mut previous = None;
    for iteration in 0..iterations {
        let mut evaluator = previous.map(|m| Evaluator::new(Arc::new(m)));
        let mut model = Model {
            version: 2,
            rules: rules::RULES_VERSION.into(),
            games,
            seed,
            iterations: iteration + 1,
            counts: (0..2)
                .map(|_| {
                    rules::MAXIMA
                        .iter()
                        .map(|&max| vec![0; max as usize + 1])
                        .collect()
                })
                .collect(),
            eligible_turns: 0,
            extra_events: 0,
        };
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed.wrapping_add(iteration as u64));
        for _ in 0..games {
            let mut sheet = Sheet::default();
            while sheet.open() > 0 {
                let eligible = usize::from(sheet.cells[11] == Some(50));
                let mut dice = Dice::default();
                let mut r = 2;
                loop {
                    while dice.len() < 5 {
                        dice.0[rng.gen_range(0..6)] += 1;
                    }
                    let action = match &mut evaluator {
                        Some(e) => e.choose(&sheet, dice, r, None),
                        None => super::legacy_choose(&sheet, dice, r, super::Policy::Opportunity),
                    };
                    match action {
                        Action::Reroll { keep } => {
                            dice = keep;
                            r -= 1;
                        }
                        Action::Write { category } | Action::CrossOut { category } => {
                            let old_extra = sheet.extra_kniffels;
                            rules::write(&mut sheet, dice, action)?;
                            model.counts[eligible][category]
                                [sheet.cells[category].unwrap() as usize] += 1;
                            model.eligible_turns += eligible as u64;
                            model.extra_events += (sheet.extra_kniffels - old_extra) as u64;
                            break;
                        }
                        Action::RollAll => unreachable!(),
                    }
                }
            }
        }
        // Rare eligibility/category combinations borrow pooled observations, never zero mass.
        for c in 0..13 {
            for e in 0..2 {
                if model.counts[e][c].iter().sum::<u64>() < 20 {
                    model.counts[e][c] = (0..=rules::MAXIMA[c] as usize)
                        .map(|score| {
                            model.counts[0][c][score]
                                + model.counts[1][c][score]
                                + u64::from(score == 0)
                        })
                        .collect();
                }
            }
        }
        previous = Some(model);
    }
    Ok(previous.unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver::Cancel;

    #[test]
    fn flexible_opening_policy_keeps_fives_but_lower_pairs_can_be_correct() {
        let mut e = Evaluator::new(Model::bundled());
        let dice = Dice::from_faces(&[1, 1, 2, 5, 5]).unwrap();
        assert_eq!(
            e.choose(&Sheet::default(), dice, 2, None),
            Action::Reroll {
                keep: Dice::from_faces(&[5, 5]).unwrap()
            }
        );
        let mut sheet = Sheet {
            cells: [Some(0); 13],
            extra_kniffels: 0,
        };
        sheet.cells[0] = None;
        assert_eq!(
            e.choose(&sheet, dice, 2, None),
            Action::Reroll {
                keep: Dice::from_faces(&[1, 1]).unwrap()
            }
        );
    }

    #[test]
    fn last_two_rows_match_exact_solitaire_across_every_roll_and_reroll_count() {
        let mut e = Evaluator::new(Model::bundled());
        for (a, b, eligible) in [(0, 4, false), (5, 12, false), (8, 11, false), (0, 12, true)] {
            let mut sheet = Sheet {
                cells: [Some(0); 13],
                extra_kniffels: 0,
            };
            sheet.cells[1] = Some(6);
            sheet.cells[2] = Some(12);
            sheet.cells[3] = Some(16);
            sheet.cells[5] = Some(24);
            if eligible {
                sheet.cells[11] = Some(50);
            }
            sheet.cells[a] = None;
            sheet.cells[b] = None;
            sheet.validate().unwrap();
            let mut cache = Cache::default();
            cache
                .precompute(&sheet, 0, 20000, Cancel::default())
                .unwrap()
                .unwrap();
            let exact = cache.turn(&sheet).unwrap();
            let actual = e.turn(&sheet);
            for r in 0..3 {
                for d in 0..252 {
                    assert!((exact.values[r][d] - actual.values[r][d]).abs() < 1e-9);
                    assert_eq!(exact.actions[r][d], actual.actions[r][d]);
                }
            }
        }
    }

    #[test]
    fn model_roundtrip_training_and_bonus_accounting() {
        let a = train(24, 999, 1).unwrap();
        let b = train(24, 999, 1).unwrap();
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap()
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.json");
        a.save(&path).unwrap();
        Model::load(&path).unwrap();
        let mut invalid = a.clone();
        invalid.version = 1;
        assert!(invalid.validate().is_err());
        let e = Evaluator::new(Model::bundled());
        let mut sheet = Sheet {
            cells: [Some(0); 13],
            extra_kniffels: 0,
        };
        assert_eq!(e.approximate(&sheet), 0.);
        sheet.cells[0] = None;
        let impossible = e.approximate(&sheet);
        sheet.cells[1] = Some(10);
        sheet.cells[2] = Some(15);
        sheet.cells[3] = Some(12);
        sheet.cells[5] = Some(24); // 61
        assert!(e.approximate(&sheet) > impossible);
        sheet.cells[3] = Some(16); // bonus already awarded
        assert!((e.approximate(&sheet) - impossible).abs() < 1e-9);
    }
}
