//! Reproducible evaluation, kept out of routine unit tests. Run with --ignored --nocapture.
use knfsolv::{
    dice::Dice,
    persistence::Session,
    policy::{self, Policy},
    solver::{self, Cancel, Method, Settings},
    state::{Phase, Position},
};
use rand::{Rng, SeedableRng};

#[test]
#[ignore = "release-build held-out baseline evaluation"]
fn compare_continuation_baseline_with_legacy_on_held_out_games() {
    let games = std::env::var("KNFSOLV_BASELINE_GAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let mut evaluator = policy::Evaluator::new(None);
    for players in [2, 3, 4] {
        let mut scores = [0.; 2];
        let mut wins = [0.; 2];
        let mut bonuses = [0_u64; 2];
        let mut delta_sum = 0.;
        let mut delta_sq = 0.;
        for game in 0..games {
            let hero = game % players;
            let mut paired_scores = [0.; 2];
            for version in 0..2 {
                let mut p = Position::new(players);
                while !matches!(p.phase, Phase::Finished) {
                    match p.phase {
                        Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } => {
                            let (mut faces, rerolls) = match p.phase {
                                Phase::AwaitingRoll {
                                    kept,
                                    rerolls_after,
                                } => (kept.faces(), rerolls_after),
                                _ => (vec![], 2),
                            };
                            let mut rng =
                                rand_chacha::ChaCha8Rng::seed_from_u64(900000 + game as u64);
                            rng.set_stream(
                                p.active_player as u64 * 64
                                    + (13 - p.players[p.active_player].sheet.open()) as u64 * 4
                                    + (2 - rerolls) as u64,
                            );
                            while faces.len() < 5 {
                                faces.push(rng.gen_range(1..=6));
                            }
                            p.enter_roll(Dice::from_faces(&faces).unwrap()).unwrap();
                        }
                        Phase::Decision { dice, rerolls_left } => {
                            let sheet = &p.players[p.active_player].sheet;
                            let action = if version == 1 && p.active_player == hero {
                                evaluator.choose(sheet, dice, rerolls_left, Policy::Opportunity)
                            } else {
                                policy::legacy_choose(
                                    sheet,
                                    dice,
                                    rerolls_left,
                                    Policy::Opportunity,
                                )
                            };
                            p.apply(action).unwrap();
                        }
                        Phase::Finished => unreachable!(),
                    }
                }
                p.validate().unwrap();
                let metrics = solver::terminal(
                    &p,
                    &Settings {
                        perspective: hero,
                        ..Settings::default()
                    },
                );
                paired_scores[version] = metrics.expected_score;
                scores[version] += metrics.expected_score;
                wins[version] += metrics.utility;
                bonuses[version] += u64::from(p.players[hero].sheet.bonus() > 0);
            }
            let delta = paired_scores[1] - paired_scores[0];
            delta_sum += delta;
            delta_sq += delta * delta;
        }
        let mean = delta_sum / games as f64;
        let se =
            ((delta_sq - delta_sum * delta_sum / games as f64) / (games - 1) as f64 / games as f64)
                .sqrt();
        println!(
            "{players} players, {games} held-out games, rotated hero: legacy score {:.2}, continuation score {:.2}; paired gain {mean:.2} ± {:.2} (approx 95% fixed-sample interval); shared-first {:.3} -> {:.3}; bonus {:.3} -> {:.3}",
            scores[0] / games as f64,
            scores[1] / games as f64,
            1.96 * se,
            wins[0] / games as f64,
            wins[1] / games as f64,
            bonuses[0] as f64 / games as f64,
            bonuses[1] as f64 / games as f64
        );
    }
}

#[test]
#[ignore = "release-build statistical evaluation"]
fn report_endgame_regret_and_match_results() {
    let mut fixtures = vec![
        Session::load(std::path::Path::new("examples/lead.json"))
            .unwrap()
            .position,
        Session::load(std::path::Path::new("examples/trail.json"))
            .unwrap()
            .position,
    ];
    for c in [0, 12] {
        let mut p = fixtures[0].clone();
        p.hypothetical = true;
        p.players[1].sheet.cells[c] = None;
        p.phase = Phase::Decision {
            dice: Dice::from_faces(&[4, 6, 6, 6, 6]).unwrap(),
            rerolls_left: 1,
        };
        fixtures.push(p);
    }
    let mut reference = vec![];
    for p in &fixtures {
        let settings = Settings {
            method: Method::Exact,
            policy: Policy::Greedy,
            time_ms: 0,
            max_nodes: 200000,
            ..Settings::default()
        };
        let exact = solver::analyze(p, &settings, Cancel::default()).unwrap();
        assert_eq!(exact.status, "exact_under_model");
        reference.push(exact);
    }
    for method in [Method::Rollout, Method::Mcts] {
        let mut regrets = vec![];
        for seed in 0..20 {
            for (p, exact) in fixtures.iter().zip(&reference) {
                let s = Settings {
                    method,
                    policy: Policy::Greedy,
                    time_ms: 0,
                    simulations: 1000,
                    seed,
                    ..Settings::default()
                };
                let r = solver::analyze(p, &s, Cancel::default()).unwrap();
                let a = r.best_action.unwrap();
                let value = exact
                    .alternatives
                    .iter()
                    .find(|x| x.action == a)
                    .unwrap()
                    .metrics
                    .unwrap()
                    .utility;
                regrets.push((exact.forecast.unwrap().utility - value).max(0.));
            }
        }
        println!(
            "{method:?}: {} fixture/seed pairs, mean regret {:.8}, max regret {:.8}",
            regrets.len(),
            regrets.iter().sum::<f64>() / regrets.len() as f64,
            regrets.iter().copied().fold(0., f64::max)
        );
    }
    // Small smoke evaluation of actual analyzed games; rotate the analyzed seat.
    // This sample is intentionally too small for a general playing-strength claim.
    let match_samples = std::env::var("KNFSOLV_QUALITY_SAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2048);
    for n in [2, 3] {
        let mut utility = 0.;
        let mut score = 0.;
        let games = std::env::var("KNFSOLV_QUALITY_GAMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(12);
        for game in 0..games {
            let mut p = Position::new(n);
            let hero = game % n;
            let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1000 + game as u64);
            let mut decision = 0;
            while !matches!(p.phase, Phase::Finished) {
                match p.phase {
                    Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } => {
                        let mut faces = if let Phase::AwaitingRoll { kept, .. } = p.phase {
                            kept.faces()
                        } else {
                            vec![]
                        };
                        while faces.len() < 5 {
                            faces.push(rng.gen_range(1..=6));
                        }
                        p.enter_roll(Dice::from_faces(&faces).unwrap()).unwrap();
                    }
                    Phase::Decision { dice, rerolls_left } => {
                        let a = if p.active_player == hero {
                            let s = Settings {
                                perspective: hero,
                                method: Method::Mcts,
                                policy: Policy::Opportunity,
                                time_ms: 0,
                                simulations: match_samples,
                                seed: decision,
                                max_nodes: 1000,
                                ..Settings::default()
                            };
                            decision += 1;
                            solver::analyze(&p, &s, Cancel::default())
                                .unwrap()
                                .best_action
                                .unwrap()
                        } else {
                            policy::choose(
                                &p.players[p.active_player].sheet,
                                dice,
                                rerolls_left,
                                Policy::Opportunity,
                            )
                        };
                        p.apply(a).unwrap();
                    }
                    Phase::Finished => unreachable!(),
                }
            }
            let metrics = solver::terminal(
                &p,
                &Settings {
                    perspective: hero,
                    ..Settings::default()
                },
            );
            utility += metrics.utility;
            score += metrics.expected_score;
        }
        println!(
            "{n} players: {games} games, MCTS ({match_samples} samples/decision) vs opportunity heuristic, rotated seats: mean shared-first utility {:.4}, mean score {:.2}; exploratory sample only",
            utility / games as f64,
            score / games as f64
        );
    }
}

/// Labels are gated on two adjacent comparisons; 1,000 independent seeds each,
/// replayed with swapped seats (2,000 matches per comparison). No budget tuning.
#[test]
#[ignore = "release-build Stage 5 difficulty acceptance, 4000 complete matches"]
fn desktop_difficulty_ordering() {
    const PAIRS: usize = 1000;
    let mut evaluator = policy::Evaluator::new(None);
    for (weak, strong) in [
        (Policy::Greedy, Policy::LegacyOpportunity),
        (Policy::LegacyOpportunity, Policy::Opportunity),
    ] {
        let mut credit = 0.0;
        let mut score_diff = 0.0;
        let mut squared = 0.0;
        for pair in 0..PAIRS {
            let mut pair_credit = 0.0;
            let mut pair_diff = 0.0;
            for strong_seat in 0..2 {
                let mut p = Position::new(2);
                while p.phase != Phase::Finished {
                    match p.phase {
                        Phase::Decision { dice, rerolls_left } => {
                            let model = if p.active_player == strong_seat {
                                strong
                            } else {
                                weak
                            };
                            let action = evaluator.choose(
                                &p.players[p.active_player].sheet,
                                dice,
                                rerolls_left,
                                model,
                            );
                            p.apply(action).unwrap();
                        }
                        Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } => {
                            let (mut faces, roll) = match p.phase {
                                Phase::AwaitingRoll {
                                    kept,
                                    rerolls_after,
                                } => (kept.faces(), 2 - rerolls_after),
                                _ => (vec![], 0),
                            };
                            let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(
                                0x5354414745350000 + pair as u64,
                            );
                            rng.set_stream(
                                p.active_player as u64 * 64
                                    + (13 - p.players[p.active_player].sheet.open()) as u64 * 4
                                    + roll as u64,
                            );
                            while faces.len() < 5 {
                                faces.push(rng.gen_range(1..=6));
                            }
                            p.enter_roll(Dice::from_faces(&faces).unwrap()).unwrap();
                        }
                        Phase::Finished => unreachable!(),
                    }
                }
                p.validate().unwrap();
                let diff = p.players[strong_seat].sheet.total() as f64
                    - p.players[1 - strong_seat].sheet.total() as f64;
                pair_diff += diff / 2.0;
                pair_credit += if diff > 0.0 {
                    0.5
                } else if diff == 0.0 {
                    0.25
                } else {
                    0.0
                };
            }
            credit += pair_credit;
            score_diff += pair_diff;
            squared += pair_diff * pair_diff;
        }
        let mean = score_diff / PAIRS as f64;
        let se = ((squared - score_diff * score_diff / PAIRS as f64)
            / (PAIRS - 1) as f64
            / PAIRS as f64)
            .sqrt();
        let win = credit / PAIRS as f64;
        // Pair averages are bounded in [0,1]. Bonferroni protects both comparisons.
        let radius = ((2.0_f64 / 0.025).ln() / (2.0 * PAIRS as f64)).sqrt();
        println!(
            "{strong:?} vs {weak:?}: {PAIRS} held-out pairs / {} swapped-seat games; paired score difference {mean:.3} ± {:.3} (approx 95%); win credit {win:.4}, simultaneous 95% Hoeffding CI [{:.4}, {:.4}]",
            PAIRS * 2,
            1.96 * se,
            (win - radius).max(0.0),
            (win + radius).min(1.0)
        );
        assert!(
            mean > 0.0 && win - radius > 0.5,
            "Difficulty ordering failed: do not publish strength labels"
        );
    }
}
