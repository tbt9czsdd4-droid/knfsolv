//! Optional exact expected-additional-score table. Partial caches contain only completed values.
use super::{Budget, Cancel, turn::TurnTable};
use crate::{Error, Result, dice::tables, rules, state::Sheet};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};
use web_time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Key {
    mask: u16,
    upper: u8,
    eligible: bool,
}
impl Key {
    fn of(s: &Sheet) -> Self {
        Self {
            mask: s.mask(),
            upper: s.upper().min(63) as u8,
            eligible: s.cells[11] == Some(50),
        }
    }
}
#[derive(Default)]
pub struct Cache {
    values: HashMap<Key, f64>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskCache {
    version: u32,
    rules: String,
    algorithm: String,
    entries: Vec<(Key, f64)>,
    checksum: u64,
}
fn checksum(entries: &[(Key, f64)]) -> u64 {
    serde_json::to_vec(entries)
        .unwrap()
        .into_iter()
        .fold(14695981039346656037, |h, b| {
            (h ^ b as u64).wrapping_mul(1099511628211)
        })
}
impl Cache {
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    fn get(&self, s: &Sheet) -> Option<f64> {
        if s.open() == 0 {
            Some(0.)
        } else {
            self.values.get(&Key::of(s)).copied()
        }
    }
    pub fn load(path: &Path) -> Result<Self> {
        let disk: DiskCache = serde_json::from_reader(std::fs::File::open(path)?)?;
        if disk.version != 1
            || disk.rules != rules::RULES_VERSION
            || disk.algorithm != "solitaire_ev_v1"
            || disk.checksum != checksum(&disk.entries)
            || disk.entries.iter().any(|(k, v)| {
                k.mask > 8191
                    || k.upper > 63
                    || (k.eligible && k.mask & (1 << 11) != 0)
                    || !v.is_finite()
                    || !(0.0..=2000.).contains(v)
            })
        {
            return Err(Error::Invalid(
                "Incompatible or corrupt solitaire cache".into(),
            ));
        }
        Ok(Self {
            values: disk.entries.into_iter().collect(),
        })
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut entries: Vec<_> = self.values.iter().map(|(k, v)| (*k, *v)).collect();
        entries.sort_by_key(|(k, _)| *k);
        let disk = DiskCache {
            version: 1,
            rules: rules::RULES_VERSION.into(),
            algorithm: "solitaire_ev_v1".into(),
            checksum: checksum(&entries),
            entries,
        };
        crate::persistence::save_json(path, &disk)
    }
    pub fn turn(&self, sheet: &Sheet) -> Option<TurnTable> {
        let mut stop = Vec::with_capacity(252);
        for &dice in &tables().full {
            let mut entries = vec![];
            for action in rules::scoring_actions(sheet, dice) {
                let mut next = *sheet;
                rules::write(&mut next, dice, action).unwrap();
                entries.push((
                    action,
                    (next.total() - sheet.total()) as f64 + self.get(&next)?,
                ));
            }
            stop.push(entries);
        }
        Some(TurnTable::build(|dice| {
            stop[tables().full_id(dice)].clone()
        }))
    }
    fn solve(&mut self, sheet: &Sheet, budget: &Budget, max: usize) -> Option<f64> {
        if budget.stopped() {
            return None;
        }
        if let Some(value) = self.get(sheet) {
            return Some(value);
        }
        if self.values.len() >= max {
            return None;
        }
        for &dice in &tables().full {
            for a in rules::scoring_actions(sheet, dice) {
                let mut next = *sheet;
                rules::write(&mut next, dice, a).unwrap();
                self.solve(&next, budget, max)?;
            }
        }
        let value = self.turn(sheet)?.before_roll();
        if self.values.len() >= max {
            return None;
        }
        self.values.insert(Key::of(sheet), value);
        Some(value)
    }
    pub fn precompute(
        &mut self,
        sheet: &Sheet,
        time_ms: u64,
        max_states: usize,
        cancel: Cancel,
    ) -> Result<Option<f64>> {
        sheet.validate()?;
        let budget = Budget {
            start: Instant::now(),
            time_ms,
            cancel,
        };
        Ok(self.solve(sheet, &budget, max_states))
    }
}
