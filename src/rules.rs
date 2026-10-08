use crate::{Error, Result, dice::Dice, state::Sheet};
use serde::{Deserialize, Serialize};

/// The Schmidt rules with the house conventions of docs/rules.md, including five of a kind as a
/// Full House.
pub const RULES_VERSION: &str = "schmidt_pdf_2024_house_v2";
/// Positions saved before five of a kind counted as a Full House. Their sheets are valid under the
/// current rules, so they load as current positions.
pub const LEGACY_RULES_VERSION: &str = "schmidt_pdf_2024_standard_v1";
pub const NAMES: [&str; 13] = [
    "Ones",
    "Twos",
    "Threes",
    "Fours",
    "Fives",
    "Sixes",
    "Three of a kind",
    "Four of a kind",
    "Full House",
    "Small Straight",
    "Large Straight",
    "Kniffel",
    "Chance",
];
pub const MAXIMA: [u16; 13] = [5, 10, 15, 20, 25, 30, 30, 30, 25, 30, 40, 50, 30];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    RollAll,
    Reroll { keep: Dice },
    Write { category: usize },
    CrossOut { category: usize },
}
impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::RollAll => write!(f, "Roll all five dice"),
            Self::Reroll { keep } => write!(
                f,
                "Keep {keep}; reroll {} {}",
                5 - keep.len(),
                if keep.len() == 4 { "die" } else { "dice" }
            ),
            Self::Write { category } => write!(
                f,
                "Write in {}",
                NAMES.get(category).unwrap_or(&"invalid category")
            ),
            Self::CrossOut { category } => write!(
                f,
                "Cross out {}",
                NAMES.get(category).unwrap_or(&"invalid category")
            ),
        }
    }
}
/// Sorted attainable scores for a category, including crossing it out with zero.
pub fn category_scores(category: usize) -> &'static [u16] {
    static SCORES: std::sync::OnceLock<[Vec<u16>; 13]> = std::sync::OnceLock::new();
    &SCORES.get_or_init(|| {
        std::array::from_fn(|c| {
            let mut scores = vec![0];
            scores.extend(
                crate::dice::tables()
                    .full
                    .iter()
                    .map(|&dice| ordinary_score(dice, c)),
            );
            scores.sort_unstable();
            scores.dedup();
            scores
        })
    })[category]
}

pub fn ordinary_score(d: Dice, c: usize) -> u16 {
    match c {
        0..=5 => (c as u16 + 1) * d.0[c] as u16,
        6 if d.0.iter().any(|&n| n >= 3) => d.sum(),
        7 if d.0.iter().any(|&n| n >= 4) => d.sum(),
        // House rule: five of a kind is a three of a kind plus a pair, so a Full House too.
        8 if (d.0.contains(&3) && d.0.contains(&2)) || d.0.contains(&5) => 25,
        9 if (0..=2).any(|s| d.0[s..s + 4].iter().all(|&n| n > 0)) => 30,
        10 if (0..=1).any(|s| d.0[s..s + 5].iter().all(|&n| n > 0)) => 40,
        11 if d.0.contains(&5) => 50,
        12 => d.sum(),
        _ => 0,
    }
}
/// Reads a saved rules version, upgrading [`LEGACY_RULES_VERSION`] to the current one.
pub fn saved_rules_version<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    let version = String::deserialize(deserializer)?;
    Ok(if version == LEGACY_RULES_VERSION {
        RULES_VERSION.into()
    } else {
        version
    })
}
pub fn extra_eligible(sheet: &Sheet, d: Dice) -> bool {
    sheet.cells[11] == Some(50) && d.0.contains(&5)
}
pub fn score(sheet: &Sheet, d: Dice, c: usize) -> u16 {
    if extra_eligible(sheet, d) {
        MAXIMA[c]
    } else {
        ordinary_score(d, c)
    }
}
pub fn scoring_actions(sheet: &Sheet, d: Dice) -> Vec<Action> {
    (0..13)
        .filter(|&c| sheet.cells[c].is_none())
        .map(|category| {
            if score(sheet, d, category) == 0 {
                Action::CrossOut { category }
            } else {
                Action::Write { category }
            }
        })
        .collect()
}
pub fn write(sheet: &mut Sheet, d: Dice, action: Action) -> Result<()> {
    let (category, cross) = match action {
        Action::Write { category } => (category, false),
        Action::CrossOut { category } => (category, true),
        _ => return Err(Error::Invalid("A scoring action is required".into())),
    };
    if category >= 13 || sheet.cells[category].is_some() {
        return Err(Error::Invalid(
            "Category is already filled or invalid".into(),
        ));
    }
    let points = score(sheet, d, category);
    if cross != (points == 0) {
        return Err(Error::Invalid(
            "This entry is not legal for these dice".into(),
        ));
    }
    if extra_eligible(sheet, d) {
        sheet.extra_kniffels += 1;
    }
    sheet.cells[category] = Some(points);
    Ok(())
}
