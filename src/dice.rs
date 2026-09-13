use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Dice(pub [u8; 6]);

impl Dice {
    pub fn from_faces(faces: &[u8]) -> Result<Self> {
        if faces.len() > 5 || faces.iter().any(|x| !(1..=6).contains(x)) {
            return Err(Error::Invalid(
                "Enter up to five dice, each from 1 to 6".into(),
            ));
        }
        let mut d = Self::default();
        for &f in faces {
            d.0[f as usize - 1] += 1;
        }
        Ok(d)
    }
    pub fn len(self) -> usize {
        self.0.iter().map(|&n| n as usize).sum()
    }
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
    pub fn sum(self) -> u16 {
        self.0
            .iter()
            .enumerate()
            .map(|(i, &n)| (i as u16 + 1) * n as u16)
            .sum()
    }
    pub fn contains(self, other: Self) -> bool {
        (0..6).all(|i| self.0[i] >= other.0[i])
    }
    pub fn plus(self, other: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] + other.0[i]))
    }
    pub fn code(self) -> usize {
        self.0.iter().rev().fold(0, |a, &b| a * 6 + b as usize)
    }
    pub fn faces(self) -> Vec<u8> {
        (0..6)
            .flat_map(|i| std::iter::repeat_n(i as u8 + 1, self.0[i] as usize))
            .collect()
    }
}
impl std::fmt::Display for Dice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() {
            return write!(f, "none");
        }
        write!(
            f,
            "{}",
            self.faces()
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}

#[derive(Debug)]
pub struct Outcome {
    pub dice: Dice,
    pub multiplicity: u32,
    pub probability: f64,
}
pub struct Tables {
    pub outcomes: [Vec<Outcome>; 6],
    pub full: Vec<Dice>,
    pub partial: Vec<Dice>,
    pub keeps: Vec<Vec<usize>>,
    /// For every partial histogram: full-roll index and probability after rolling the rest.
    pub transitions: Vec<Vec<(usize, f64)>>,
    full_index: Vec<usize>,
    partial_index: Vec<usize>,
}
fn enumerate(left: u8, face: usize, d: &mut [u8; 6], out: &mut Vec<Dice>) {
    if face == 5 {
        d[face] = left;
        out.push(Dice(*d));
        return;
    }
    for n in 0..=left {
        d[face] = n;
        enumerate(left - n, face + 1, d, out);
    }
}
impl Tables {
    fn build() -> Self {
        const FACT: [u32; 6] = [1, 1, 2, 6, 24, 120];
        let outcomes = std::array::from_fn(|n| {
            let mut ds = Vec::new();
            enumerate(n as u8, 0, &mut [0; 6], &mut ds);
            ds.into_iter()
                .map(|dice| {
                    let multiplicity =
                        FACT[n] / dice.0.iter().map(|&x| FACT[x as usize]).product::<u32>();
                    Outcome {
                        dice,
                        multiplicity,
                        probability: multiplicity as f64 / 6_u32.pow(n as u32) as f64,
                    }
                })
                .collect::<Vec<_>>()
        });
        let full: Vec<_> = outcomes[5].iter().map(|o| o.dice).collect();
        let partial: Vec<_> = outcomes.iter().flatten().map(|o| o.dice).collect();
        let mut full_index = vec![usize::MAX; 46656];
        let mut partial_index = full_index.clone();
        for (i, d) in full.iter().enumerate() {
            full_index[d.code()] = i;
        }
        for (i, d) in partial.iter().enumerate() {
            partial_index[d.code()] = i;
        }
        let keeps = full
            .iter()
            .map(|d| {
                partial
                    .iter()
                    .enumerate()
                    .filter_map(|(i, k)| (k.len() < 5 && d.contains(*k)).then_some(i))
                    .collect()
            })
            .collect();
        let transitions = partial
            .iter()
            .map(|k| {
                outcomes[5 - k.len()]
                    .iter()
                    .map(|o| (full_index[k.plus(o.dice).code()], o.probability))
                    .collect()
            })
            .collect();
        Self {
            outcomes,
            full,
            partial,
            keeps,
            transitions,
            full_index,
            partial_index,
        }
    }
    pub fn full_id(&self, d: Dice) -> usize {
        self.full_index[d.code()]
    }
    pub fn partial_id(&self, d: Dice) -> usize {
        self.partial_index[d.code()]
    }
}
pub fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(Tables::build)
}
