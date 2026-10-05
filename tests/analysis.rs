use knfsolv::{
    dice::Dice,
    persistence::Session,
    policy::Policy,
    solver::{self, AnalysisSession, Cancel, Method, Settings, WorkSlice},
    state::{Phase, Position},
};
use std::time::{Duration, Instant};

fn fixture() -> Position {
    Session::load(std::path::Path::new("examples/trail.json"))
        .unwrap()
        .position
}
fn settings() -> Settings {
    Settings {
        method: Method::Rollout,
        simulations: 400,
        time_ms: 0,
        policy: Policy::Greedy,
        ..Settings::default()
    }
}
fn evidence(report: &solver::Report) -> serde_json::Value {
    serde_json::json!({"alternatives": report.alternatives, "samples": report.samples, "best": report.best_action,
        "paired": report.baseline_comparison, "baseline": report.baseline_action, "nodes": report.nodes})
}

#[test]
fn paired_rounds_and_in_flight_games_resume_without_recounting_samples() {
    let p = fixture();
    let settings = settings();
    let mut whole = AnalysisSession::new(p.clone(), settings.clone()).unwrap();
    whole.advance(WorkSlice::samples(240));
    let reference = evidence(&whole.snapshot());
    let mut sliced = AnalysisSession::new(p.clone(), settings.clone()).unwrap();
    sliced.advance(WorkSlice {
        max_steps: 1,
        ..WorkSlice::samples(240)
    });
    sliced.advance(WorkSlice::samples(3 - sliced.samples()));
    let partial = sliced.snapshot();
    assert_eq!(partial.samples, 3);
    assert_eq!(
        partial.alternatives.iter().map(|a| a.samples).sum::<u64>(),
        3
    );
    assert!(
        partial
            .alternatives
            .iter()
            .all(|a| a.baseline_comparison.as_ref().unwrap().samples == 0),
        "unfinished rounds must not contribute paired samples"
    );
    let before = evidence(&partial);
    let time = sliced.computation_time();
    sliced.set_paused(true);
    sliced.advance(WorkSlice::samples(100));
    assert_eq!(evidence(&sliced.snapshot()), before);
    assert_eq!(sliced.computation_time(), time);
    sliced.set_paused(false);
    for _ in 0..237 {
        sliced.advance(WorkSlice::samples(1));
    }
    assert_eq!(evidence(&sliced.snapshot()), reference);
    let bounded = solver::analyze(
        &p,
        &Settings {
            simulations: 240,
            ..settings
        },
        Cancel::default(),
    )
    .unwrap();
    assert_eq!(
        evidence(&bounded),
        reference,
        "bounded callers use the same completed samples and comparisons"
    );
}

#[test]
fn a_paused_full_multiplayer_game_retains_its_position_and_rng_streams() {
    let mut p = Position::new(2);
    p.phase = Phase::Decision {
        dice: Dice::from_faces(&[1, 2, 3, 4, 6]).unwrap(),
        rerolls_left: 2,
    };
    let mut sliced = AnalysisSession::new(p.clone(), settings()).unwrap();
    sliced.advance(WorkSlice {
        max_steps: 5,
        ..WorkSlice::samples(1)
    });
    assert_eq!(sliced.samples(), 0);
    let cancel = Cancel::default();
    cancel.cancel();
    sliced.advance_cancellable(WorkSlice::samples(5), &cancel);
    assert_eq!(sliced.samples(), 0);
    sliced.advance(WorkSlice::samples(96));
    let mut whole = AnalysisSession::new(p, settings()).unwrap();
    whole.advance(WorkSlice::samples(96));
    assert_eq!(evidence(&sliced.snapshot()), evidence(&whole.snapshot()));
}

#[test]
fn limited_time_excludes_pauses_and_continuous_can_extend_a_limited_run() {
    let mut session = AnalysisSession::new(
        fixture(),
        Settings {
            simulations: 12,
            time_ms: 100,
            ..settings()
        },
    )
    .unwrap();
    session.set_continuous(false);
    session.advance(WorkSlice::samples(3));
    session.set_paused(true);
    let time = session.computation_time();
    std::thread::sleep(Duration::from_millis(150));
    session.advance(WorkSlice::samples(500));
    assert_eq!(session.computation_time(), time);
    assert!(!session.finished());
    session.set_paused(false);
    session.advance(WorkSlice::samples(50));
    assert!(session.finished());
    assert_eq!(session.samples(), 12);
    session.set_continuous(true);
    session.advance(WorkSlice::samples(12));
    assert_eq!(session.samples(), 24);
    assert!(!session.finished());
    let mut full = AnalysisSession::new(fixture(), settings()).unwrap();
    full.advance(WorkSlice::samples(24));
    assert_eq!(evidence(&full.snapshot()), evidence(&session.snapshot()));
}

#[test]
fn provisional_baseline_is_immediate_and_auto_exact_stops_computation() {
    let mut session = AnalysisSession::new(fixture(), Settings::default()).unwrap();
    let baseline = session.snapshot();
    assert_eq!(baseline.samples, 0);
    assert!(baseline.best_action.is_some());
    assert_eq!(baseline.confidence, "insufficient");
    assert!(baseline.selection_reason.contains("Provisional baseline"));
    session.advance(WorkSlice {
        max_time: Duration::from_millis(250),
        ..WorkSlice::default()
    });
    assert!(session.finished());
    let exact = session.snapshot();
    assert_eq!(exact.status, "exact_under_model");
    assert_eq!(exact.samples, 0);
    session.advance(WorkSlice::samples(100));
    assert_eq!(evidence(&session.snapshot()), evidence(&exact));
}

#[test]
fn bounded_exact_attempt_is_not_cut_to_one_work_slice() {
    // Two open categories in all, before the roll: solved exactly in milliseconds, but not in
    // the 1 ms slice an earlier version also imposed on the exact attempt.
    let mut p = Position::new(2);
    for (player, open) in p.players.iter_mut().zip([11, 12]) {
        player.sheet.cells = [Some(0); 13];
        player.sheet.cells[open] = None;
    }
    let mut session = AnalysisSession::new(p, Settings::default()).unwrap();
    session.advance(WorkSlice {
        max_time: Duration::from_millis(1),
        ..WorkSlice::default()
    });
    assert!(session.finished());
    assert_eq!(session.snapshot().status, "exact_under_model");
}

#[test]
fn auto_uses_a_fixed_policy_after_an_incomplete_bounded_exact_attempt() {
    let mut session = AnalysisSession::new(
        fixture(),
        Settings {
            max_nodes: 1,
            ..Settings::default()
        },
    )
    .unwrap();
    session.advance(WorkSlice::samples(80));
    let report = session.snapshot();
    assert!(report.method.contains("fixed-policy"));
    assert!(report.note.contains("exact attempt was incomplete"));
    assert!(report.nodes <= 1);
    assert_eq!(report.samples, 80);
    assert!(!session.finished());
}

#[test]
fn mcts_freezes_once_and_validation_evidence_survives_work_slices() {
    let settings = Settings {
        method: Method::Mcts,
        simulations: 100,
        max_nodes: 16,
        max_depth: 1,
        ..settings()
    };
    let mut whole = AnalysisSession::new(fixture(), settings.clone()).unwrap();
    whole.advance(WorkSlice::samples(500));
    let mut sliced = AnalysisSession::new(fixture(), settings).unwrap();
    for count in [20, 33, 97, 350] {
        sliced.advance(WorkSlice::samples(count));
    }
    assert_eq!(evidence(&whole.snapshot()), evidence(&sliced.snapshot()));
    let before = sliced.snapshot();
    assert_eq!(before.nodes, 1, "depth one freezes the root only");
    assert_eq!(
        before.samples - before.alternatives.iter().map(|a| a.samples).sum::<u64>(),
        50
    );
    sliced.advance(WorkSlice::samples(400));
    let after = sliced.snapshot();
    assert_eq!(after.nodes, before.nodes);
    assert_eq!(
        after.samples - after.alternatives.iter().map(|a| a.samples).sum::<u64>(),
        50,
        "training never resumes after validation starts"
    );
}

#[test]
fn continuous_mcts_training_and_dynamic_caches_remain_bounded() {
    let settings = Settings {
        method: Method::Mcts,
        simulations: 20_000,
        max_nodes: 8,
        max_depth: 2,
        ..settings()
    };
    let mut session = AnalysisSession::new(fixture(), settings).unwrap();
    session.advance(WorkSlice::samples(3500));
    let first = session.snapshot();
    let trained = first.samples - first.alternatives.iter().map(|a| a.samples).sum::<u64>();
    assert!(trained <= 2000);
    let nodes = first.nodes;
    assert!(nodes <= 8);
    for _ in 0..3 {
        session.advance(WorkSlice::samples(500));
        let report = session.snapshot();
        assert_eq!(report.nodes, nodes);
        assert_eq!(
            report.samples - report.alternatives.iter().map(|a| a.samples).sum::<u64>(),
            trained
        );
        let (decisions, singles) = session.cache_entries();
        assert!(decisions <= 8 && singles <= 8);
    }
}

#[test]
fn a_wall_time_slice_and_cancellation_leave_a_resumable_session() {
    let mut p = Position::new(8);
    p.phase = Phase::Decision {
        dice: Dice::from_faces(&[1, 2, 3, 4, 5]).unwrap(),
        rerolls_left: 2,
    };
    let mut session = AnalysisSession::new(p, Settings::default()).unwrap();
    let start = Instant::now();
    session.advance(WorkSlice {
        max_time: Duration::from_millis(10),
        ..WorkSlice::default()
    });
    assert!(start.elapsed() < Duration::from_millis(500));
    assert!(!session.finished());
    let evidence = evidence(&session.snapshot());
    let cancel = Cancel::default();
    cancel.cancel();
    session.advance_cancellable(WorkSlice::default(), &cancel);
    assert_eq!(self::evidence(&session.snapshot()), evidence);
}

#[test]
fn continuous_continuation_caches_evict_without_changing_sampled_evidence() {
    let mut p = Position::new(1);
    p.phase = Phase::Decision {
        dice: Dice::from_faces(&[6, 5, 4, 3, 2]).unwrap(),
        rerolls_left: 2,
    };
    let s = Settings {
        method: Method::Rollout,
        policy: Policy::Opportunity,
        max_nodes: 3,
        ..settings()
    };
    let mut limited_cache = AnalysisSession::new(p.clone(), s.clone()).unwrap();
    for _ in 0..4 {
        limited_cache.advance(WorkSlice::samples(4));
        let (decisions, singles) = limited_cache.cache_entries();
        assert!(decisions > 0 && decisions <= 3 && singles <= 3);
    }
    let mut larger_cache = AnalysisSession::new(
        p,
        Settings {
            max_nodes: 100,
            ..s
        },
    )
    .unwrap();
    larger_cache.advance(WorkSlice::samples(16));
    assert_eq!(
        evidence(&limited_cache.snapshot()),
        evidence(&larger_cache.snapshot()),
        "cache eviction cannot change the frozen policy"
    );
}

#[test]
fn depth_tracks_tree_exploration_and_retains_frozen_and_paused_progress() {
    use solver::SearchPhase;
    let mut p = Position::new(2);
    p.phase = Phase::Decision {
        dice: Dice::from_faces(&[1, 2, 3, 4, 5]).unwrap(),
        rerolls_left: 2,
    };
    let mut search = AnalysisSession::new(
        p.clone(),
        Settings {
            method: Method::Mcts,
            max_depth: 1,
            simulations: 100,
            ..settings()
        },
    )
    .unwrap();
    assert!(search.snapshot().progress.is_none());
    search.advance(WorkSlice {
        max_steps: 1,
        ..WorkSlice::samples(1)
    });
    let training = search.snapshot().progress.unwrap();
    assert_eq!(training.phase, SearchPhase::MctsTraining);
    assert_eq!(training.max_decision_depth, 1);
    search.advance(WorkSlice::samples(70));
    let frozen = search.snapshot().progress.unwrap();
    assert_eq!(frozen.phase, SearchPhase::MctsValidation);
    assert_eq!(
        frozen.max_decision_depth, 1,
        "baseline continuation must not increase depth"
    );
    search.set_paused(true);
    search.advance(WorkSlice::samples(10));
    assert_eq!(search.snapshot().progress, Some(frozen));
    search.set_paused(false);
    search.advance(WorkSlice::samples(10));
    assert_eq!(search.snapshot().progress, Some(frozen));
    let fresh = AnalysisSession::new(p.clone(), settings()).unwrap();
    assert_eq!(
        fresh.snapshot().progress.unwrap().phase,
        SearchPhase::Rollout
    );
    let mut legacy = serde_json::to_value(fresh.snapshot()).unwrap();
    legacy.as_object_mut().unwrap().remove("progress");
    assert!(
        serde_json::from_value::<solver::Report>(legacy)
            .unwrap()
            .progress
            .is_none()
    );

    // Opponent decisions and chance outcomes precede the first analyzed decision.
    let mut other = AnalysisSession::new(
        p,
        Settings {
            perspective: 1,
            method: Method::Mcts,
            max_depth: 1,
            ..settings()
        },
    )
    .unwrap();
    other.advance(WorkSlice {
        max_steps: 1,
        ..WorkSlice::samples(1)
    });
    assert!(other.snapshot().progress.is_none());
    other.advance(WorkSlice::samples(1));
    assert_eq!(other.snapshot().progress.unwrap().max_decision_depth, 1);
}

#[test]
fn exact_depth_and_auto_fallback_report_the_current_phase() {
    use solver::SearchPhase;
    let mut p = Position::new(2);
    for player in &mut p.players {
        player.sheet.cells = [Some(0); 13];
        player.sheet.cells[0] = None;
    }
    p.phase = Phase::Decision {
        dice: Dice::from_faces(&[1, 2, 3, 4, 5]).unwrap(),
        rerolls_left: 0,
    };
    let exact = Settings {
        method: Method::Exact,
        max_nodes: 10_000,
        ..settings()
    };
    let report = solver::analyze(&p, &exact, Cancel::default()).unwrap();
    assert_eq!(report.status, "exact_under_model");
    assert_eq!(
        report.progress.unwrap().max_decision_depth,
        1,
        "opponent's whole turn does not increment depth"
    );
    let mut single = Position::new(1);
    single.players[0].sheet.cells = [Some(0); 13];
    single.players[0].sheet.cells[0] = None;
    single.phase = Phase::Decision {
        dice: Dice::from_faces(&[1, 2, 3, 4, 5]).unwrap(),
        rerolls_left: 2,
    };
    let own_decisions = solver::analyze(&single, &exact, Cancel::default()).unwrap();
    assert_eq!(own_decisions.status, "exact_under_model");
    assert_eq!(own_decisions.progress.unwrap().max_decision_depth, 3);
    let incomplete = solver::analyze(
        &p,
        &Settings {
            max_nodes: 1,
            ..exact.clone()
        },
        Cancel::default(),
    )
    .unwrap();
    assert_eq!(incomplete.progress.unwrap().phase, SearchPhase::Exact);
    assert_eq!(incomplete.status, "incomplete");
    let mut auto = AnalysisSession::new(
        p,
        Settings {
            method: Method::Auto,
            max_nodes: 1,
            ..exact
        },
    )
    .unwrap();
    assert!(auto.snapshot().progress.is_none());
    auto.advance(WorkSlice::samples(2));
    let fallback = auto.snapshot();
    assert_eq!(fallback.progress.unwrap().phase, SearchPhase::Rollout);
    assert!(fallback.note.contains("Previous exact attempt: depth 1"));
}
