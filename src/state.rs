use crate::{
    Error, Result,
    dice::{Dice, tables},
    rules::{self, Action},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    pub cells: [Option<u16>; 13],
    #[serde(default)]
    pub extra_kniffels: u8,
}
impl Sheet {
    pub fn upper(&self) -> u16 {
        self.cells[..6].iter().flatten().sum()
    }
    pub fn bonus(&self) -> u16 {
        if self.upper() >= 63 { 35 } else { 0 }
    }
    pub fn total(&self) -> u16 {
        self.cells.iter().flatten().sum::<u16>() + self.bonus() + 50 * self.extra_kniffels as u16
    }
    pub fn open(&self) -> usize {
        self.cells.iter().filter(|x| x.is_none()).count()
    }
    pub fn mask(&self) -> u16 {
        self.cells
            .iter()
            .enumerate()
            .fold(0, |m, (i, c)| m | ((c.is_none() as u16) << i))
    }
    pub fn validate(&self) -> Result<()> {
        for (c, entry) in self.cells.iter().enumerate() {
            if let Some(n) = entry
                && *n != 0
                && !tables()
                    .full
                    .iter()
                    .any(|&d| rules::ordinary_score(d, c) == *n)
            {
                return Err(Error::Invalid(format!(
                    "{} cannot score {n}",
                    rules::NAMES[c]
                )));
            }
        }
        if self.extra_kniffels as usize > 13 - self.open() - usize::from(self.cells[11].is_some())
            || (self.extra_kniffels > 0 && self.cells[11] != Some(50))
        {
            return Err(Error::Invalid(
                "Extra Kniffels require 50 in Kniffel and enough completed turns".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Player {
    pub name: String,
    pub sheet: Sheet,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Phase {
    BeforeFirstRoll,
    Decision { dice: Dice, rerolls_left: u8 },
    AwaitingRoll { kept: Dice, rerolls_after: u8 },
    Finished,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub schema_version: u32,
    #[serde(deserialize_with = "rules::saved_rules_version")]
    pub rules_version: String,
    pub players: Vec<Player>,
    /// Indices into players, first player in the match first.
    pub turn_order: Vec<usize>,
    pub active_player: usize,
    pub phase: Phase,
    #[serde(default)]
    pub hypothetical: bool,
}
impl Position {
    /// Infer the next seat from completed turns, without changing the position.
    /// `None` means every sheet is complete. Zero scores count as completed turns.
    pub fn infer_next_player(&self) -> Result<Option<usize>> {
        let n = self.players.len();
        let mut order = self.turn_order.clone();
        order.sort_unstable();
        if !(1..=8).contains(&n) || order != (0..n).collect::<Vec<_>>() {
            return Err(Error::Invalid(
                "Turn order must contain each player exactly once".into(),
            ));
        }
        let counts: Vec<_> = self
            .turn_order
            .iter()
            .map(|&p| 13 - self.players[p].sheet.open())
            .collect();
        let minimum = *counts.iter().min().unwrap();
        if minimum == 13 {
            return Ok(None);
        }
        let next = counts.iter().position(|&count| count == minimum).unwrap();
        if counts
            .iter()
            .enumerate()
            .any(|(i, &count)| count != minimum + usize::from(i < next))
        {
            return Err(Error::Invalid("Inconsistent score sheets: completed-turn counts must follow the configured turn order. Correct the entries to resume analysis.".into()));
        }
        Ok(Some(self.turn_order[next]))
    }

    pub fn new(n: usize) -> Self {
        Self {
            schema_version: 1,
            rules_version: rules::RULES_VERSION.into(),
            players: (0..n)
                .map(|i| Player {
                    name: format!("Player {}", i + 1),
                    sheet: Sheet::default(),
                })
                .collect(),
            turn_order: (0..n).collect(),
            active_player: 0,
            phase: Phase::BeforeFirstRoll,
            hypothetical: false,
        }
    }
    pub fn open(&self) -> usize {
        self.players.iter().map(|p| p.sheet.open()).sum()
    }
    pub fn validate(&self) -> Result<()> {
        let invalid = |s: &str| Error::Invalid(s.into());
        let n = self.players.len();
        if self.schema_version != 1 || self.rules_version != rules::RULES_VERSION {
            return Err(invalid("Unsupported position or rules version"));
        }
        if !(1..=8).contains(&n) || self.active_player >= n {
            return Err(invalid("Need 1–8 players and a valid active player"));
        }
        let mut order = self.turn_order.clone();
        order.sort_unstable();
        if order != (0..n).collect::<Vec<_>>() {
            return Err(invalid(
                "Turn order must contain each player index exactly once",
            ));
        }
        for p in &self.players {
            p.sheet
                .validate()
                .map_err(|e| Error::Invalid(format!("{}: {e}", p.name)))?;
        }
        match self.phase {
            Phase::Decision { dice, rerolls_left } if dice.len() != 5 || rerolls_left > 2 => {
                return Err(invalid("A decision needs five dice and 0–2 rerolls"));
            }
            Phase::AwaitingRoll {
                kept,
                rerolls_after,
            } if kept.len() >= 5 || rerolls_after > 1 => {
                return Err(invalid(
                    "A pending reroll needs 0–4 kept dice and 0–1 subsequent rerolls",
                ));
            }
            _ => (),
        }
        if (self.open() == 0) != matches!(self.phase, Phase::Finished) {
            return Err(invalid("Finished phase must agree with completed sheets"));
        }
        if self.open() > 0 && self.players[self.active_player].sheet.open() == 0 {
            return Err(invalid("The active player has no open categories"));
        }
        if !self.hypothetical && self.open() > 0 {
            let active_index = self
                .turn_order
                .iter()
                .position(|&i| i == self.active_player)
                .unwrap();
            let filled = 13 - self.players[self.active_player].sheet.open();
            for (i, &p) in self.turn_order.iter().enumerate() {
                let expected = filled + usize::from(i < active_index);
                if 13 - self.players[p].sheet.open() != expected {
                    return Err(invalid(
                        "Sheets and active player do not match turn order; correct them or enable hypothetical mode",
                    ));
                }
            }
        }
        Ok(())
    }
    /// The moves worth considering, as analysis sees them; see [`rules::scoring_actions`].
    pub fn actions(&self) -> Vec<Action> {
        match self.phase {
            Phase::BeforeFirstRoll => vec![Action::RollAll],
            Phase::Decision { dice, rerolls_left } => {
                let mut actions =
                    rules::scoring_actions(&self.players[self.active_player].sheet, dice);
                if rerolls_left > 0 {
                    actions.extend(tables().keeps[tables().full_id(dice)].iter().map(|&k| {
                        Action::Reroll {
                            keep: tables().partial[k],
                        }
                    }));
                }
                actions
            }
            _ => vec![],
        }
    }
    /// Whether `action` may be played: any of [`Self::actions`], or crossing out any open
    /// category, which is always allowed.
    pub fn is_legal(&self, action: Action) -> bool {
        match (self.phase, action) {
            (Phase::Decision { .. }, Action::CrossOut { category }) => {
                category < 13 && self.players[self.active_player].sheet.cells[category].is_none()
            }
            _ => self.actions().contains(&action),
        }
    }
    /// Enters `points` in `category` for the player to move, one of [`rules::entry_points`],
    /// and moves on to the next player.
    pub fn enter(&mut self, category: usize, points: u16) -> Result<()> {
        let Phase::Decision { dice, .. } = self.phase else {
            return Err(Error::Invalid("Action is not legal in this phase".into()));
        };
        rules::write_points(
            &mut self.players[self.active_player].sheet,
            dice,
            category,
            points,
        )?;
        self.advance();
        Ok(())
    }
    pub fn apply(&mut self, action: Action) -> Result<()> {
        match (self.phase, action) {
            (Phase::BeforeFirstRoll, Action::RollAll) => (), // external roll still required
            (Phase::Decision { dice, rerolls_left }, Action::Reroll { keep })
                if rerolls_left > 0 && keep.len() < 5 && dice.contains(keep) =>
            {
                self.phase = Phase::AwaitingRoll {
                    kept: keep,
                    rerolls_after: rerolls_left - 1,
                };
            }
            (
                Phase::Decision { dice, .. },
                a @ (Action::Write { .. } | Action::CrossOut { .. }),
            ) => {
                rules::write(&mut self.players[self.active_player].sheet, dice, a)?;
                self.advance();
            }
            _ => return Err(Error::Invalid("Action is not legal in this phase".into())),
        }
        Ok(())
    }
    pub fn advance(&mut self) {
        if self.open() == 0 {
            self.phase = Phase::Finished;
            return;
        }
        let index = self
            .turn_order
            .iter()
            .position(|&p| p == self.active_player)
            .unwrap();
        for offset in 1..=self.players.len() {
            let next = self.turn_order[(index + offset) % self.players.len()];
            if self.players[next].sheet.open() > 0 {
                self.active_player = next;
                self.phase = Phase::BeforeFirstRoll;
                return;
            }
        }
    }
    pub fn enter_roll(&mut self, dice: Dice) -> Result<()> {
        if dice.len() != 5 {
            return Err(Error::Invalid(
                "Enter all five dice, including retained dice".into(),
            ));
        }
        let r = match self.phase {
            Phase::BeforeFirstRoll => 2,
            Phase::AwaitingRoll {
                kept,
                rerolls_after,
            } if dice.contains(kept) => rerolls_after,
            _ => {
                return Err(Error::Invalid(
                    "No roll pending, or retained dice are missing".into(),
                ));
            }
        };
        self.phase = Phase::Decision {
            dice,
            rerolls_left: r,
        };
        Ok(())
    }
}
