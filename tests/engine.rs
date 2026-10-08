use knfsolv::{
    dice::{Dice, tables},
    persistence::Session,
    policy::{self, Policy},
    rules::{self, Action},
    solver::{self, Cancel, Method, Objective, Settings, solitaire::Cache, turn::TurnTable},
    state::{Phase, Position, Sheet},
};

fn dice(faces: &[u8]) -> Dice {
    Dice::from_faces(faces).unwrap()
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}
fn final_chance(target: u16) -> Position {
    let mut p = Position::new(2);
    for player in &mut p.players {
        player.sheet.cells = [Some(0); 13];
    }
    p.players[0].sheet.cells[12] = None;
    p.players[1].sheet.cells[8] = Some(25);
    p.players[1].sheet.cells[3] = Some(target - 25);
    p.turn_order = vec![1, 0];
    p.phase = Phase::Decision {
        dice: dice(&[6, 6, 6, 6, 2]),
        rerolls_left: 1,
    };
    p.validate().unwrap();
    p
}
fn exact_settings() -> Settings {
    Settings {
        method: Method::Exact,
        time_ms: 0,
        max_nodes: 200000,
        ..Settings::default()
    }
}

#[test]
fn pdf_scoring_and_straights() {
    let d = dice(&[5, 5, 5, 3, 3]);
    assert_eq!(rules::ordinary_score(d, 4), 15);
    assert_eq!(rules::ordinary_score(d, 2), 6);
    assert_eq!(rules::ordinary_score(d, 6), 21);
    assert_eq!(rules::ordinary_score(d, 8), 25);
    assert_eq!(rules::ordinary_score(dice(&[1, 2, 3, 4, 3]), 9), 30);
    assert_eq!(rules::ordinary_score(dice(&[2, 3, 4, 5, 6]), 10), 40);
    // House rule: five of a kind is three of a kind plus a pair, so a Full House too.
    assert_eq!(rules::ordinary_score(dice(&[3, 3, 3, 3, 3]), 8), 25);
    assert_eq!(rules::ordinary_score(dice(&[3, 3, 3, 3, 5]), 8), 0);
    assert_eq!(rules::ordinary_score(dice(&[3, 3, 3, 5, 6]), 8), 0);
    assert_eq!(rules::ordinary_score(dice(&[3, 3, 3, 3, 3]), 7), 15);
    assert_eq!(rules::ordinary_score(dice(&[3, 3, 3, 3, 3]), 11), 50);
}
#[test]
fn bonus_thresholds_and_additional_kniffel() {
    for (upper, expected) in [(62, 0), (63, 35), (64, 35)] {
        let mut s = Sheet::default();
        s.cells[..6].copy_from_slice(&[
            Some(upper - 60),
            Some(6),
            Some(9),
            Some(12),
            Some(15),
            Some(18),
        ]);
        s.validate().unwrap();
        assert_eq!(s.bonus(), expected);
    }
    let d = dice(&[2, 2, 2, 2, 2]);
    for c in (0..13).filter(|&c| c != 11) {
        let mut s = Sheet::default();
        s.cells[11] = Some(50);
        rules::write(&mut s, d, Action::Write { category: c }).unwrap();
        assert_eq!(s.cells[c], Some(rules::MAXIMA[c]));
        assert_eq!(s.extra_kniffels, 1);
        assert_eq!(s.total(), 100 + rules::MAXIMA[c]);
        s.validate().unwrap();
        assert!(rules::write(&mut s, d, Action::Write { category: c }).is_err());
        assert_eq!(s.extra_kniffels, 1);
    }
    let mut s = Sheet::default();
    s.cells[11] = Some(0);
    assert!(!rules::extra_eligible(&s, d));
    assert_eq!(rules::score(&s, d, 10), 0);
    // Crossing out is always allowed, even where the roll fits; writing needs a fitting roll.
    let mut crossed = s;
    rules::write(&mut crossed, d, Action::CrossOut { category: 12 }).unwrap();
    assert_eq!(crossed.cells[12], Some(0));
    assert!(rules::write(&mut s.clone(), d, Action::Write { category: 10 }).is_err());
    rules::write(&mut s, d, Action::CrossOut { category: 10 }).unwrap();
    assert_eq!(s.extra_kniffels, 0);

    let mut s = Sheet::default();
    s.cells[1..6].copy_from_slice(&[Some(6), Some(9), Some(12), Some(15), Some(18)]);
    let before = s.total();
    rules::write(
        &mut s,
        dice(&[1, 1, 1, 6, 6]),
        Action::Write { category: 0 },
    )
    .unwrap();
    assert_eq!(s.total() - before, 38);
    let before = s.total();
    rules::write(
        &mut s,
        dice(&[1, 2, 3, 4, 5]),
        Action::Write { category: 12 },
    )
    .unwrap();
    assert_eq!(s.total() - before, 15);
}
#[test]
fn five_of_a_kind_is_a_full_house_in_play_and_advice() {
    let fours = dice(&[4, 4, 4, 4, 4]);
    // With Kniffel crossed out there is no extra Kniffel, but Full House still pays 25.
    let mut s = Sheet::default();
    s.cells[11] = Some(0);
    assert!(rules::scoring_actions(&s, fours).contains(&Action::Write { category: 8 }));
    let mut crossed = s;
    rules::write(&mut crossed, fours, Action::CrossOut { category: 8 }).unwrap();
    assert_eq!(crossed.cells[8], Some(0));
    rules::write(&mut s, fours, Action::Write { category: 8 }).unwrap();
    assert_eq!((s.cells[8], s.extra_kniffels), (Some(25), 0));
    // With Kniffel still open, both rows accept it.
    let open = Sheet::default();
    assert_eq!(rules::score(&open, fours, 8), 25);
    assert_eq!(rules::score(&open, fours, 11), 50);

    // The exact solver writes the Full House instead of crossing it out.
    let mut p = Position::new(1);
    p.players[0].sheet.cells = [Some(0); 13];
    p.players[0].sheet.cells[8] = None;
    p.players[0].sheet.cells[12] = None;
    p.phase = Phase::Decision {
        dice: dice(&[2, 2, 2, 2, 2]),
        rerolls_left: 0,
    };
    p.validate().unwrap();
    let report = solver::analyze(&p, &exact_settings(), Cancel::default()).unwrap();
    assert_eq!(report.status, "exact_under_model");
    assert_eq!(report.best_action, Some(Action::Write { category: 8 }));
}
#[test]
fn every_entry_is_allowed_but_analysis_considers_full_scores_only() {
    let ones = dice(&[1, 1, 1, 1, 1]);
    let open = Sheet::default();
    // Fewer of the matching dice may count in the upper section; crossing out is always allowed.
    assert_eq!(rules::entry_points(&open, ones, 0), [5, 4, 3, 2, 1, 0]);
    assert_eq!(rules::entry_points(&open, ones, 1), [0]);
    assert_eq!(
        rules::entry_points(&open, dice(&[1, 1, 2, 5, 5]), 4),
        [10, 5, 0]
    );
    assert_eq!(
        rules::entry_points(&open, dice(&[2, 2, 3, 3, 3]), 8),
        [25, 0]
    );
    assert_eq!(rules::entry_points(&open, dice(&[2, 2, 3, 3, 3]), 9), [0]);
    let mut s = open;
    rules::write_points(&mut s, ones, 0, 3).unwrap();
    assert_eq!(s.cells[0], Some(3));
    assert!(rules::write_points(&mut open.clone(), ones, 0, 6).is_err());
    assert!(rules::write_points(&mut open.clone(), dice(&[1, 1, 2, 5, 5]), 4, 7).is_err());
    assert!(rules::write_points(&mut s, ones, 0, 2).is_err());

    // An extra Kniffel scores the maximum anywhere, any lower entry too, and earns its 50 points
    // whatever is entered.
    let mut eligible = open;
    eligible.cells[11] = Some(50);
    let fours = dice(&[4, 4, 4, 4, 4]);
    assert_eq!(rules::entry_points(&eligible, fours, 0), [5, 4, 3, 2, 1, 0]);
    assert_eq!(rules::entry_points(&eligible, fours, 9), [30, 0]);
    let mut crossed = eligible;
    rules::write(&mut crossed, fours, Action::CrossOut { category: 12 }).unwrap();
    assert_eq!((crossed.cells[12], crossed.extra_kniffels), (Some(0), 1));

    // Positions allow every cross-out; analysis only looks at full scores.
    let mut p = Position::new(1);
    p.phase = Phase::Decision {
        dice: dice(&[2, 2, 3, 3, 3]),
        rerolls_left: 0,
    };
    let cross = Action::CrossOut { category: 8 };
    assert!(p.is_legal(cross));
    assert!(!p.actions().contains(&cross));
    assert!(p.actions().contains(&Action::Write { category: 8 }));
    assert!(!p.is_legal(Action::Write { category: 9 }));
    p.enter(2, 3).unwrap();
    assert_eq!(p.players[0].sheet.cells[2], Some(3));
    assert_eq!(p.phase, Phase::BeforeFirstRoll);
}
#[test]
fn positions_saved_under_the_previous_rules_load_as_current() {
    let current = std::fs::read_to_string("examples/middle.json").unwrap();
    assert!(current.contains(rules::RULES_VERSION));
    let legacy = current.replace(rules::RULES_VERSION, rules::LEGACY_RULES_VERSION);
    let session = Session::from_json(&legacy).unwrap();
    assert_eq!(session.position.rules_version, rules::RULES_VERSION);
    assert_eq!(session, Session::from_json(&current).unwrap());
    let unknown = current.replace(rules::RULES_VERSION, "some_other_rules");
    assert!(Session::from_json(&unknown).is_err());
}
#[test]
fn probability_tables_match_independent_ordered_enumeration() {
    let t = tables();
    assert_eq!(t.full.len(), 252);
    assert_eq!(t.partial.len(), 462);
    for n in 0..=5 {
        assert_eq!(
            t.outcomes[n].iter().map(|o| o.multiplicity).sum::<u32>(),
            6_u32.pow(n as u32)
        );
        close(t.outcomes[n].iter().map(|o| o.probability).sum(), 1.);
    }
    let mut counts = std::collections::HashMap::new();
    for mut code in 0..7776 {
        let mut hist = [0_u8; 6];
        for _ in 0..5 {
            hist[code % 6] += 1;
            code /= 6;
        }
        *counts.entry(hist).or_insert(0_u32) += 1;
    }
    for o in &t.outcomes[5] {
        assert_eq!(counts[&o.dice.0], o.multiplicity);
    }
    for (i, d) in t.full.iter().enumerate() {
        assert!(t.keeps[i].len() <= 31);
        for &k in &t.keeps[i] {
            assert!(d.contains(t.partial[k]));
        }
    }
    for ts in &t.transitions {
        close(ts.iter().map(|(_, p)| p).sum(), 1.);
    }
}
#[test]
fn turn_dp_matches_direct_one_die_expectation() {
    let t = TurnTable::build(|d| vec![(Action::Write { category: 12 }, d.sum() as f64)]);
    let d = dice(&[6, 6, 6, 6, 2]);
    let i = tables().full_id(d);
    close(t.values[0][i], 26.);
    close(t.values[1][i], 27.5);
    assert_eq!(
        t.actions[1][i],
        Action::Reroll {
            keep: dice(&[6, 6, 6, 6])
        }
    );
    for i in 0..252 {
        assert!(t.values[1][i] >= t.values[0][i]);
        assert!(t.values[2][i] >= t.values[1][i]);
    }
    close(t.before_roll(), 5. * (4. * 4.25 + 5. + 6.) / 6.); // per die: with two rerolls, retain 5+; with one, retain 4+
}
#[test]
fn winning_objective_changes_the_move() {
    let settings = exact_settings();
    let lead = solver::analyze(&final_chance(25), &settings, Cancel::default()).unwrap();
    assert_eq!(lead.status, "exact_under_model");
    assert_eq!(lead.best_action, Some(Action::Write { category: 12 }));
    close(lead.forecast.unwrap().utility, 1.);
    let k = Action::Reroll {
        keep: dice(&[6, 6, 6, 6]),
    };
    close(
        lead.alternatives
            .iter()
            .find(|a| a.action == k)
            .unwrap()
            .metrics
            .unwrap()
            .utility,
        11. / 12.,
    );
    let trail = solver::analyze(&final_chance(29), &settings, Cancel::default()).unwrap();
    assert_eq!(trail.best_action, Some(k));
    close(trail.forecast.unwrap().utility, 0.25);
    close(trail.forecast.unwrap().outright_win, 1. / 6.);
    close(trail.forecast.unwrap().tied_first, 1. / 6.);
    let ev = Settings {
        objective: Objective::ExpectedScore,
        ..settings
    };
    let a = solver::analyze(&final_chance(25), &ev, Cancel::default()).unwrap();
    let b = solver::analyze(&final_chance(29), &ev, Cancel::default()).unwrap();
    assert_eq!(a.best_action, Some(k));
    close(a.forecast.unwrap().utility, 27.5);
    close(a.forecast.unwrap().utility, b.forecast.unwrap().utility);
}
#[test]
fn transitions_retain_then_release_dice_and_advance() {
    let mut p = Position::new(2);
    p.enter_roll(dice(&[1, 1, 2, 3, 4])).unwrap();
    p.apply(Action::Reroll {
        keep: dice(&[1, 1]),
    })
    .unwrap();
    assert!(p.enter_roll(dice(&[2, 2, 2, 3, 4])).is_err());
    p.enter_roll(dice(&[1, 1, 6, 6, 6])).unwrap();
    p.apply(Action::Reroll {
        keep: dice(&[6, 6, 6]),
    })
    .unwrap();
    p.enter_roll(dice(&[6, 6, 6, 6, 6])).unwrap();
    assert!(
        p.apply(Action::Reroll {
            keep: Dice::default()
        })
        .is_err()
    );
    p.apply(Action::Write { category: 11 }).unwrap();
    assert_eq!(p.active_player, 1);
    assert_eq!(p.players[0].sheet.cells[11], Some(50));
    p.validate().unwrap();
}
#[test]
fn validation_rejects_invalid_states_and_differentiates_zero() {
    let mut p = Position::new(2);
    p.players[0].sheet.cells[0] = Some(0);
    assert_eq!(p.players[0].sheet.open(), 12);
    assert!(p.validate().is_err());
    p.active_player = 1;
    p.validate().unwrap();
    p.turn_order = vec![1, 1];
    assert!(p.validate().is_err());
    p.turn_order = vec![0, 1];
    p.phase = Phase::Decision {
        dice: Dice([255; 6]),
        rerolls_left: 2,
    };
    assert!(p.validate().is_err());
    p.phase = Phase::BeforeFirstRoll;
    p.players[0].sheet.extra_kniffels = 12;
    assert!(p.validate().is_err());
    p.players[0].sheet.extra_kniffels = 0;
    p.players[1].sheet.cells[1] = Some(3);
    assert!(p.validate().is_err());
}
#[test]
fn all_policies_generate_legal_moves() {
    let mut p = Position::new(1);
    for model in [Policy::Opportunity, Policy::Greedy] {
        for &d in &tables().full {
            for r in 0..=2 {
                p.phase = Phase::Decision {
                    dice: d,
                    rerolls_left: r,
                };
                let a = policy::choose(&p.players[0].sheet, d, r, model);
                assert!(p.actions().contains(&a));
            }
        }
    }
}
#[test]
fn simulation_is_reproducible_and_agrees_with_exact_values() {
    let p = final_chance(29);
    let s = Settings {
        method: Method::Rollout,
        time_ms: 0,
        simulations: 4000,
        ..Settings::default()
    };
    let a = solver::analyze(&p, &s, Cancel::default()).unwrap();
    let b = solver::analyze(&p, &s, Cancel::default()).unwrap();
    assert_eq!(
        serde_json::to_value(&a.alternatives).unwrap(),
        serde_json::to_value(&b.alternatives).unwrap()
    );
    let exact = solver::analyze(&p, &exact_settings(), Cancel::default()).unwrap();
    for e in exact.alternatives {
        let mc = a
            .alternatives
            .iter()
            .find(|x| x.action == e.action)
            .unwrap();
        let [lo, hi] = mc.interval.unwrap();
        assert!(e.metrics.unwrap().utility >= lo && e.metrics.unwrap().utility <= hi);
    }
}
#[test]
fn unfinished_opponents_are_played_to_completion() {
    let mut p = final_chance(25);
    p.hypothetical = true;
    p.players[1].sheet.cells[12] = None;
    p.phase = Phase::Decision {
        dice: dice(&[6, 6, 6, 6, 2]),
        rerolls_left: 0,
    };
    let settings = Settings {
        policy: Policy::Greedy,
        ..exact_settings()
    };
    let r = solver::analyze(&p, &settings, Cancel::default()).unwrap();
    // Opponent's total becomes at least 30, so scoring 26 cannot win.
    close(r.forecast.unwrap().utility, 0.);
    assert_eq!(r.status, "exact_under_model");
    // Same current total, but Ones remains instead: target is 25 + number of ones.
    p.players[1].sheet.cells[12] = Some(0);
    p.players[1].sheet.cells[0] = None;
    let r = solver::analyze(&p, &settings, Cancel::default()).unwrap();
    let p0 = (5_f64 / 6.).powi(5);
    let p1 = 5. * (1. / 6.) * (5_f64 / 6.).powi(4);
    close(r.forecast.unwrap().utility, p0 + 0.5 * p1);
}

#[test]
fn equal_opponent_totals_with_different_open_rows_change_advice() {
    let mut p = final_chance(25);
    p.hypothetical = true;
    p.phase = Phase::Decision {
        dice: dice(&[6, 6, 6, 6, 4]),
        rerolls_left: 1,
    };
    p.players[1].sheet.cells[12] = None;
    let s = Settings {
        policy: Policy::Greedy,
        ..exact_settings()
    };
    let chance = solver::analyze(&p, &s, Cancel::default()).unwrap();
    p.players[1].sheet.cells[12] = Some(0);
    p.players[1].sheet.cells[0] = None;
    let ones = solver::analyze(&p, &s, Cancel::default()).unwrap();
    assert_eq!(chance.status, "exact_under_model");
    assert_eq!(ones.status, "exact_under_model");
    assert_eq!(
        chance.best_action,
        Some(Action::Reroll {
            keep: dice(&[6, 6, 6, 6])
        })
    );
    assert_eq!(ones.best_action, Some(Action::Write { category: 12 }));
}

#[test]
fn expected_score_simulation_does_not_depend_on_other_sheets() {
    let mut p = final_chance(25);
    p.hypothetical = true;
    p.players[1].sheet.cells[12] = None;
    let s = Settings {
        objective: Objective::ExpectedScore,
        method: Method::Rollout,
        time_ms: 0,
        simulations: 1000,
        ..Settings::default()
    };
    let a = solver::analyze(&p, &s, Cancel::default()).unwrap();
    p.players[1].sheet.cells[0] = None;
    let b = solver::analyze(&p, &s, Cancel::default()).unwrap();
    for x in &a.alternatives {
        let y = b
            .alternatives
            .iter()
            .find(|y| y.action == x.action)
            .unwrap();
        close(x.metrics.unwrap().utility, y.metrics.unwrap().utility);
    }
}

#[test]
fn randomized_reachable_matches_remain_valid_through_completion() {
    use rand::{Rng, SeedableRng};
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(76);
    for n in [1, 2, 4, 8] {
        let mut p = Position::new(n);
        while !matches!(p.phase, Phase::Finished) {
            p.validate().unwrap();
            match p.phase {
                Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } => {
                    let kept = if let Phase::AwaitingRoll { kept, .. } = p.phase {
                        kept
                    } else {
                        Dice::default()
                    };
                    let mut faces = kept.faces();
                    while faces.len() < 5 {
                        faces.push(rng.gen_range(1..=6));
                    }
                    p.enter_roll(dice(&faces)).unwrap();
                }
                Phase::Decision { dice, rerolls_left } => {
                    let a = policy::choose(
                        &p.players[p.active_player].sheet,
                        dice,
                        rerolls_left,
                        Policy::Opportunity,
                    );
                    p.apply(a).unwrap();
                }
                Phase::Finished => unreachable!(),
            }
        }
        p.validate().unwrap();
        assert_eq!(p.open(), 0);
    }
}
#[test]
fn pending_roll_and_opponent_turn_return_forecasts_not_hero_moves() {
    let mut p = final_chance(29);
    p.apply(Action::Reroll {
        keep: dice(&[6, 6, 6, 6]),
    })
    .unwrap();
    let r = solver::analyze(&p, &exact_settings(), Cancel::default()).unwrap();
    assert!(r.best_action.is_none());
    close(r.forecast.unwrap().utility, 0.25);
    let s = Settings {
        perspective: 1,
        ..exact_settings()
    };
    let r = solver::analyze(&final_chance(29), &s, Cancel::default()).unwrap();
    assert!(r.best_action.is_none());
    assert!(r.forecast.is_some());
}
#[test]
fn terminal_fractional_ties_and_exact_budget_status() {
    let mut p = Position::new(3);
    for x in &mut p.players {
        x.sheet.cells = [Some(0); 13];
    }
    p.phase = Phase::Finished;
    let r = solver::analyze(&p, &Settings::default(), Cancel::default()).unwrap();
    close(r.forecast.unwrap().utility, 1. / 3.);
    close(r.forecast.unwrap().tied_first, 1.);
    let s = Settings {
        max_nodes: 1,
        ..exact_settings()
    };
    let r = solver::analyze(&final_chance(29), &s, Cancel::default()).unwrap();
    assert_eq!(r.status, "incomplete");
    assert!(r.best_action.is_none());
    let cancel = Cancel::default();
    cancel.cancel();
    let r = solver::analyze(&Position::new(2), &Settings::default(), cancel).unwrap();
    assert_eq!(r.status, "incomplete");
    assert!(r.best_action.is_none());
}
#[test]
fn solitaire_cache_is_exact_resumable_and_validated() {
    let sheet = final_chance(25).players[0].sheet;
    let mut cache = Cache::default();
    let v = cache
        .precompute(&sheet, 0, 100, Cancel::default())
        .unwrap()
        .unwrap();
    let reference = TurnTable::build(|d| vec![(Action::Write { category: 12 }, d.sum() as f64)]);
    close(v, reference.before_roll());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("cache.json");
    cache.save(&path).unwrap();
    let mut loaded = Cache::load(&path).unwrap();
    close(
        loaded
            .precompute(&sheet, 0, 100, Cancel::default())
            .unwrap()
            .unwrap(),
        v,
    );
    let mut json: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(&path).unwrap()).unwrap();
    json["checksum"] = 0.into();
    knfsolv::persistence::save_json(&path, &json).unwrap();
    assert!(Cache::load(&path).is_err());
}
#[test]
fn session_roundtrip() {
    let mut s = Session::new(2);
    s.position = final_chance(25);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("position.json");
    s.save(&path).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert_eq!(loaded.position, s.position);
}

#[test]
fn partial_multistate_cache_roundtrips_without_float_checksum_drift() {
    let mut cache = Cache::default();
    assert!(
        cache
            .precompute(&Sheet::default(), 0, 64, Cancel::default())
            .unwrap()
            .is_none()
    );
    assert_eq!(cache.len(), 64);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("partial.json");
    cache.save(&path).unwrap();
    let mut loaded = Cache::load(&path).unwrap();
    assert_eq!(loaded.len(), 64);
    assert!(
        loaded
            .precompute(&Sheet::default(), 0, 128, Cancel::default())
            .unwrap()
            .is_none()
    );
    assert_eq!(loaded.len(), 128);
    loaded.save(&path).unwrap();
    assert_eq!(Cache::load(&path).unwrap().len(), 128);
}
#[test]
fn tree_search_validates_on_fresh_samples() {
    let s = Settings {
        method: Method::Mcts,
        time_ms: 0,
        simulations: 1000,
        ..Settings::default()
    };
    let p = final_chance(29);
    let a = solver::analyze(&p, &s, Cancel::default()).unwrap();
    let b = solver::analyze(&p, &s, Cancel::default()).unwrap();
    assert_eq!(a.status, "estimated");
    assert!(a.method.contains("MCTS"));
    assert!(a.nodes > 0);
    assert_eq!(a.alternatives.iter().map(|a| a.samples).sum::<u64>(), 500);
    assert_eq!(
        serde_json::to_value(&a.alternatives).unwrap(),
        serde_json::to_value(&b.alternatives).unwrap()
    );
}

#[test]
fn cancellation_interrupts_full_game_search() {
    policy::warm_up();
    let cancel = Cancel::default();
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || {
        solver::analyze(
            &Position::new(8),
            &Settings {
                time_ms: 0,
                simulations: 100000000,
                ..Settings::default()
            },
            worker_cancel,
        )
        .unwrap()
    });
    std::thread::sleep(std::time::Duration::from_millis(10));
    let start = std::time::Instant::now();
    cancel.cancel();
    let report = worker.join().unwrap();
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    assert_eq!(report.stop_reason, "Cancelled");
}

#[test]
fn incomplete_root_coverage_is_not_a_recommendation() {
    let s = Settings {
        method: Method::Rollout,
        time_ms: 0,
        simulations: 1,
        ..Settings::default()
    };
    let report = solver::analyze(&final_chance(29), &s, Cancel::default()).unwrap();
    assert_eq!(report.status, "incomplete");
    assert!(report.best_action.is_none());
    assert!(report.alternatives.iter().any(|a| a.metrics.is_none()));
}

#[test]
fn very_small_sample_budget_uses_declared_baseline() {
    let mut p = Position::new(2);
    let d = dice(&[1, 2, 3, 4, 6]);
    p.phase = Phase::Decision {
        dice: d,
        rerolls_left: 2,
    };
    let s = Settings {
        method: Method::Mcts,
        time_ms: 0,
        simulations: 128,
        ..Settings::default()
    };
    let report = solver::analyze(&p, &s, Cancel::default()).unwrap();
    assert_eq!(report.status, "estimated");
    assert_eq!(
        report.best_action,
        Some(policy::choose(
            &p.players[0].sheet,
            d,
            2,
            Policy::Opportunity
        ))
    );
    assert!(report.note.contains("falls back"));
}

#[test]
fn mcts_depth_bounds_tree_and_uses_full_game_baseline_after_cutoff() {
    let mut p = final_chance(29);
    p.phase = Phase::Decision {
        dice: dice(&[6, 6, 6, 6, 2]),
        rerolls_left: 2,
    };
    let s = Settings {
        method: Method::Mcts,
        time_ms: 0,
        simulations: 1000,
        max_depth: 1,
        ..Settings::default()
    };
    let shallow = solver::analyze(&p, &s, Cancel::default()).unwrap();
    assert_eq!(
        shallow.nodes, 1,
        "Only the root decision can be expanded at depth 1"
    );
    let rollout = solver::analyze(
        &p,
        &Settings {
            method: Method::Rollout,
            simulations: 500,
            ..s.clone()
        },
        Cancel::default(),
    )
    .unwrap();
    // Identical independent validation samples: the forced root decision consumes
    // depth 1, so every subsequent decision must use the fixed baseline.
    for a in &shallow.alternatives {
        let b = rollout
            .alternatives
            .iter()
            .find(|b| a.action == b.action)
            .unwrap();
        close(a.metrics.unwrap().utility, b.metrics.unwrap().utility);
        close(
            a.metrics.unwrap().expected_score,
            b.metrics.unwrap().expected_score,
        );
        assert_eq!(a.samples, b.samples);
    }
    let deeper = solver::analyze(&p, &Settings { max_depth: 2, ..s }, Cancel::default()).unwrap();
    assert!(deeper.nodes > shallow.nodes);
    let legacy = Session::load(std::path::Path::new("examples/trail.json")).unwrap();
    assert_eq!(legacy.settings.max_depth, 0);
}

#[test]
fn uncertain_sampled_leader_is_not_silently_replaced_by_baseline() {
    let p = final_chance(29);
    let settings = Settings {
        conservative_advice: false,
        method: Method::Rollout,
        policy: Policy::Greedy,
        simulations: 1000,
        time_ms: 0,
        ..Settings::default()
    };
    let sampled = solver::analyze(&p, &settings, Cancel::default()).unwrap();
    assert_eq!(sampled.best_action, sampled.sampled_best_action);
    assert_ne!(sampled.best_action, sampled.baseline_action);
    assert_eq!(sampled.recommendation_basis, "sampled_best");
    assert_eq!(sampled.confidence, "low");
    let paired = sampled.baseline_comparison.as_ref().unwrap();
    assert!(paired.samples > 0);
    assert!(paired.mean_gain > 0.);
    assert!(paired.interval[0] <= 0. && paired.interval[1] >= 0.);
    assert!(sampled.text().contains("Highest sampled estimate"));
    let conservative = solver::analyze(
        &p,
        &Settings {
            conservative_advice: true,
            ..settings
        },
        Cancel::default(),
    )
    .unwrap();
    assert_eq!(conservative.best_action, conservative.baseline_action);
    assert_eq!(
        conservative.sampled_best_action,
        sampled.sampled_best_action
    );
    assert_eq!(
        conservative.recommendation_basis,
        "baseline_uncertain_advantage"
    );
    assert!(
        conservative
            .selection_reason
            .contains("not the sampled leader")
    );
    assert_eq!(
        serde_json::to_value(&conservative.alternatives).unwrap(),
        serde_json::to_value(&sampled.alternatives).unwrap()
    );
    let counts: std::collections::HashSet<_> =
        sampled.alternatives.iter().map(|a| a.samples).collect();
    assert!(
        counts.len() > 1,
        "root samples should be adaptively allocated"
    );
}

#[test]
fn continuation_settings_roundtrip_and_legacy_defaults() {
    let mut session = Session::load(std::path::Path::new("examples/trail.json")).unwrap();
    assert!(session.settings.conservative_advice);
    assert!(session.settings.baseline_model.is_none());
    session.settings.conservative_advice = true;
    session.settings.baseline_model = Some("data/continuation-v2.json".into());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    session.save(&path).unwrap();
    let restored = Session::load(&path).unwrap();
    assert!(restored.settings.conservative_advice);
    assert_eq!(
        restored.settings.baseline_model,
        session.settings.baseline_model
    );
    let mut evaluator = policy::Evaluator::configured(
        None,
        Some(std::path::Path::new("data/continuation-v2.json")),
    );
    assert!(!evaluator.note.contains("ignored"));
    let d = dice(&[1, 1, 2, 5, 5]);
    assert_eq!(
        evaluator.choose(&knfsolv::state::Sheet::default(), d, 2, Policy::Opportunity),
        Action::Reroll {
            keep: dice(&[5, 5])
        }
    );
}

#[test]
fn category_score_options_match_attainable_values() {
    use knfsolv::{dice::tables, rules, state::Sheet};
    for category in 0..13 {
        let scores = rules::category_scores(category);
        assert_eq!(scores[0], 0);
        assert!(scores.windows(2).all(|w| w[0] < w[1]));
        for score in 0..=rules::MAXIMA[category] + 1 {
            let attainable = score == 0
                || tables()
                    .full
                    .iter()
                    .any(|&d| rules::ordinary_score(d, category) == score);
            assert_eq!(scores.contains(&score), attainable);
            let mut sheet = Sheet::default();
            sheet.cells[category] = Some(score);
            assert_eq!(sheet.validate().is_ok(), attainable);
        }
    }
    assert_eq!(rules::category_scores(0), &[0, 1, 2, 3, 4, 5]);
    assert_eq!(rules::category_scores(1), &[0, 2, 4, 6, 8, 10]);
    assert_eq!(rules::category_scores(8), &[0, 25]);
    assert_eq!(rules::category_scores(9), &[0, 30]);
    assert_eq!(rules::category_scores(10), &[0, 40]);
    assert_eq!(rules::category_scores(11), &[0, 50]);
    assert!(!rules::category_scores(7).contains(&4));
}
