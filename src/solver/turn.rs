//! Exact finite-horizon dice DP relative to the supplied scoring continuation.
use crate::{
    dice::{Dice, tables},
    rules::Action,
};

pub struct TurnTable {
    pub values: [Vec<f64>; 3],
    pub actions: [Vec<Action>; 3],
    pub keep_values: [Vec<f64>; 2],
}
impl TurnTable {
    pub fn build(mut stop: impl FnMut(Dice) -> Vec<(Action, f64)>) -> Self {
        let t = tables();
        let mut values = std::array::from_fn(|_| vec![f64::NEG_INFINITY; 252]);
        let mut actions = std::array::from_fn(|_| vec![Action::RollAll; 252]);
        let mut keep_values = std::array::from_fn(|_| vec![0.0; 462]);
        for (i, &dice) in t.full.iter().enumerate() {
            for (a, value) in stop(dice) {
                if value > values[0][i] {
                    values[0][i] = value;
                    actions[0][i] = a;
                }
            }
        }
        for r in 1..=2 {
            for (k, transitions) in t.transitions.iter().enumerate() {
                keep_values[r - 1][k] =
                    transitions.iter().map(|&(d, p)| p * values[r - 1][d]).sum();
            }
            for d in 0..252 {
                values[r][d] = values[0][d];
                actions[r][d] = actions[0][d];
                for &k in &t.keeps[d] {
                    if keep_values[r - 1][k] > values[r][d] + 1e-12 {
                        values[r][d] = keep_values[r - 1][k];
                        actions[r][d] = Action::Reroll { keep: t.partial[k] };
                    }
                }
            }
        }
        Self {
            values,
            actions,
            keep_values,
        }
    }
    pub fn before_roll(&self) -> f64 {
        tables().outcomes[5]
            .iter()
            .enumerate()
            .map(|(i, o)| o.probability * self.values[2][i])
            .sum()
    }
}
