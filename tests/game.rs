use knfsolv::{
    dice::Dice,
    game::{
        AnalysisMode, AutomaticStep, ComputerPolicy, DesktopGame, DiceSource, GameController,
        GameEventKind, JobPurpose, PlayerControl,
    },
    persistence::Session,
    rules::{self, Action},
    solver::{self, Cancel, Method},
    state::Phase,
};
use std::time::{Duration, Instant};

fn enter(game: &mut GameController, faces: &[u8]) {
    for &face in faces {
        game.choose_face(face, Instant::now()).unwrap();
    }
}
fn score(game: &GameController, category: usize) -> Action {
    let Phase::Decision { dice, .. } = game.session().position.phase else {
        panic!("Expected decision")
    };
    if rules::score(
        &game.session().position.players[game.session().position.active_player].sheet,
        dice,
        category,
    ) == 0
    {
        Action::CrossOut { category }
    } else {
        Action::Write { category }
    }
}
fn computer_game(source: DiceSource) -> GameController {
    let mut session = GameController::local(2, source, 314159)
        .unwrap()
        .session()
        .clone();
    session.desktop.as_mut().unwrap().controllers[1] = PlayerControl::Computer;
    GameController::new(session).unwrap()
}
fn finish_computer_turn(game: &mut GameController, now: &mut Instant, category: usize) {
    while game.control() == PlayerControl::Computer
        && !matches!(game.session().position.phase, Phase::Finished)
    {
        *now += Duration::from_secs(1);
        let job = game.automatic_job(*now).expect("Computer needs work");
        match job.step {
            AutomaticStep::Decide => {
                let action = score(game, category);
                assert!(
                    game.accept_computer_decision(job.token, action, *now)
                        .unwrap()
                );
            }
            _ => {
                assert!(game.execute_automatic(job, *now).unwrap());
            }
        }
    }
}

#[test]
fn old_sessions_open_in_analysis_and_metadata_defaults_are_compatible() {
    let old = Session::new(2);
    let json = serde_json::to_string(&old).unwrap();
    assert!(!json.contains("desktop"));
    let loaded: Session = serde_json::from_str(&json).unwrap();
    assert!(loaded.desktop.is_none());
    let mut game = GameController::new(loaded).unwrap();
    assert!(game.editing());
    enter(&mut game, &[5, 4, 3, 2, 1]);
    assert!(game.play(score(&game, 12), Instant::now()).is_err());
    game.set_editing(false);
    game.play(score(&game, 12), Instant::now()).unwrap();

    let desktop = DesktopGame::new(2, DiceSource::Physical, 7);
    let mut value = serde_json::to_value(desktop).unwrap();
    for key in ["advisor", "computer", "dice_source"] {
        value.as_object_mut().unwrap().remove(key);
    }
    let desktop: DesktopGame = serde_json::from_value(value).unwrap();
    assert_eq!(desktop.advisor.mode, AnalysisMode::Continuous);
    assert_eq!(desktop.computer.policy, ComputerPolicy::ContinuationV2);
    assert_eq!(desktop.dice_source, DiceSource::Physical);
    let mut session = Session::new(2);
    session.desktop = Some(desktop);
    session.validate().unwrap();
    session.desktop.as_mut().unwrap().controllers.pop();
    assert!(session.validate().is_err());
    session
        .desktop
        .as_mut()
        .unwrap()
        .controllers
        .push(PlayerControl::Human);
    session.desktop.as_mut().unwrap().version = 2;
    assert!(session.validate().is_err());
}

#[test]
fn physical_rolls_corrections_and_settings_are_controller_commands() {
    let mut game = GameController::local(2, DiceSource::Physical, 1).unwrap();
    enter(&mut game, &[6, 2, 6]);
    let mut settings = game.session().clone();
    settings.settings.max_depth = 2;
    game.edit_session(settings).unwrap();
    assert_eq!(game.dice().faces, [6, 2, 6, 0, 0]);
    enter(&mut game, &[4, 5]);
    game.toggle_keep(2);
    game.reroll_selected(Instant::now()).unwrap();
    assert_eq!(game.dice().faces, [0, 0, 6, 0, 0]);
    assert_eq!(game.dice().locked, [false, false, true, false, false]);
    enter(&mut game, &[1, 2]);
    game.clear_die(Instant::now()).unwrap();
    assert_eq!(game.dice().faces, [1, 0, 6, 0, 0]);
    enter(&mut game, &[3, 4, 5]);
    assert_eq!(game.dice().faces, [1, 3, 6, 4, 5]);
    assert!(game.undo());
    assert_eq!(game.dice().faces, [0, 0, 6, 0, 0]);
    assert!(game.redo());
    assert_eq!(game.dice().faces, [1, 3, 6, 4, 5]);

    let now = Instant::now();
    let before = game.session().clone();
    let count = game.undo_len();
    game.select_die(0, false);
    game.choose_face(2, now).unwrap();
    game.select_die(1, false);
    game.choose_face(4, now + Duration::from_millis(100))
        .unwrap();
    assert!(game.play(Action::Write { category: 12 }, now).is_err());
    game.flush_correction(now + Duration::from_millis(349))
        .unwrap();
    assert_eq!(game.session(), &before);
    game.flush_correction(now + Duration::from_millis(350))
        .unwrap();
    assert_eq!(game.undo_len(), count + 1);
    assert!(game.undo());
    assert_eq!(game.session(), &before);
    assert_eq!(
        game.session().desktop.as_ref().unwrap().rng.word_position,
        0
    );
}

#[test]
fn digital_rolls_require_a_command_and_undo_redo_replays_recorded_outcomes() {
    let now = Instant::now();
    let mut game = GameController::local(2, DiceSource::Digital, 42).unwrap();
    assert_eq!(game.automatic_job(now + Duration::from_secs(100)), None);
    assert!(matches!(
        game.session().position.phase,
        Phase::BeforeFirstRoll
    ));
    assert!(game.choose_face(6, now).is_err());
    let before = game.session().clone();
    game.roll_digital(now).unwrap();
    let first = game.session().clone();
    let faces = game.dice().faces;
    assert!(faces.iter().all(|f| (1..=6).contains(f)));
    assert!(matches!(
        game.history()[0].kind,
        GameEventKind::Roll {
            generated: true,
            ..
        }
    ));
    game.toggle_keep(0);
    game.reroll_selected(now).unwrap();
    assert_eq!(game.dice().faces[0], faces[0]);
    let rerolled = game.session().clone();
    let rerolled_faces = game.dice().faces;
    let history = game.history().to_vec();
    assert_eq!(
        game.undo_len(),
        2,
        "digital reroll choice and outcome are one operation"
    );
    assert!(game.undo());
    assert_eq!(game.session(), &first);
    assert_eq!(game.dice().faces, faces);
    assert!(game.redo());
    assert_eq!(game.session(), &rerolled);
    assert_eq!(game.dice().faces, rerolled_faces);
    assert_eq!(game.history(), history);
    assert!(game.undo());
    assert!(game.undo());
    assert_eq!(game.session(), &before);
    // Even a fresh roll command after Undo resumes the recorded RNG stream.
    game.roll_digital(now).unwrap();
    assert_eq!(game.session(), &first);
    assert!(!game.can_redo());
}

#[test]
fn save_reload_resumes_rng_and_begins_a_new_undo_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("digital.json");
    let now = Instant::now();
    let mut game = GameController::local(2, DiceSource::Digital, 1234567).unwrap();
    game.roll_digital(now).unwrap();
    game.play(score(&game, 12), now).unwrap();
    game.save(&path).unwrap();
    let stale = game.token(JobPurpose::Coaching);
    let mut replay = GameController::new(Session::load(&path).unwrap()).unwrap();
    assert!(!replay.editing());
    assert!(!replay.can_undo());
    game.roll_digital(now).unwrap();
    replay.roll_digital(now).unwrap();
    assert_eq!(game.session(), replay.session());
    assert_eq!(game.dice().faces, replay.dice().faces);
    game.load(&path).unwrap();
    assert!(!game.accepts(stale));
    assert!(!game.can_undo());
    assert!(!game.can_redo());
    assert!(game.history().is_empty());
    let stale = game.token(JobPurpose::Coaching);
    game.replace_session(game.session().clone()).unwrap();
    assert!(
        !game.accepts(stale),
        "reloading identical state still invalidates jobs"
    );
}

#[test]
fn solver_samples_and_settings_never_advance_the_gameplay_rng() {
    let now = Instant::now();
    let mut session = GameController::local(1, DiceSource::Digital, 99)
        .unwrap()
        .session()
        .clone();
    session.position.players[0].sheet.cells = [Some(0); 13];
    session.position.players[0].sheet.cells[12] = None;
    session.settings.method = Method::Rollout;
    session.settings.time_ms = 0;
    session.settings.simulations = 50;
    let mut game = GameController::new(session).unwrap();
    game.roll_digital(now).unwrap();
    let mut replay = GameController::new(game.session().clone()).unwrap();
    let rng = game.session().desktop.as_ref().unwrap().rng.clone();
    let _ = solver::analyze(
        &game.session().position,
        &game.session().settings,
        Cancel::default(),
    )
    .unwrap();
    assert_eq!(game.session().desktop.as_ref().unwrap().rng, rng);
    let mut next = game.session().clone();
    next.settings.seed = 999999;
    next.settings.max_depth = 4;
    game.edit_session(next).unwrap();
    game.reroll_selected(now).unwrap();
    replay.reroll_selected(now).unwrap();
    assert_eq!(game.dice().faces, replay.dice().faces);
    assert_eq!(
        game.session().desktop.as_ref().unwrap().rng,
        replay.session().desktop.as_ref().unwrap().rng
    );
}

#[test]
fn pending_physical_rolls_save_and_reload_without_losing_retained_dice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("physical.json");
    let mut game = GameController::local(2, DiceSource::Physical, 12).unwrap();
    enter(&mut game, &[6, 2, 6, 4, 5]);
    game.toggle_keep(2);
    game.reroll_selected(Instant::now()).unwrap();
    game.save(&path).unwrap();
    enter(&mut game, &[2]);
    assert!(
        game.save(&path).is_err(),
        "partial physical drafts are never silently saved"
    );
    game.load(&path).unwrap();
    assert_eq!(game.dice().locked.iter().filter(|&&v| v).count(), 1);
    enter(&mut game, &[1, 2, 3, 4]);
    assert!(
        matches!(game.session().position.phase, Phase::Decision { dice, rerolls_left: 1 } if dice.contains(Dice::from_faces(&[6]).unwrap()))
    );
    game.session().validate().unwrap();
}

#[test]
fn computer_undo_returns_to_previous_human_decision_and_redo_restores_all_actions() {
    let mut game = computer_game(DiceSource::Physical);
    let mut now = Instant::now();
    enter(&mut game, &[6, 6, 6, 6, 2]);
    let human_decision = game.session().clone();
    game.play(score(&game, 12), now).unwrap();
    assert_eq!(game.control(), PlayerControl::Computer);
    assert_eq!(game.dice_source(), DiceSource::Digital);
    assert!(game.choose_face(6, now).is_err());
    finish_computer_turn(&mut game, &mut now, 12);
    assert_eq!(game.session().position.active_player, 0);
    let after = game.session().clone();
    let history = game.history().to_vec();
    assert!(game.undo());
    assert_eq!(game.session(), &human_decision);
    assert_eq!(game.history().len(), 1);
    assert!(game.redo());
    assert_eq!(game.session(), &after);
    assert_eq!(game.history(), history);
    assert_eq!(
        game.automatic_job(now),
        None,
        "human first roll never happens automatically"
    );
}

#[test]
fn computer_presentation_pause_load_undo_and_purpose_guards_are_nonblocking() {
    let mut game = computer_game(DiceSource::Physical);
    let now = Instant::now();
    enter(&mut game, &[1; 5]);
    game.play(score(&game, 0), now).unwrap();
    assert!(
        game.automatic_job(now + Duration::from_millis(399))
            .is_none()
    );
    let roll = game
        .automatic_job(now + Duration::from_millis(400))
        .unwrap();
    assert_eq!(roll.step, AutomaticStep::Roll);
    assert!(
        game.execute_automatic(roll, now + Duration::from_millis(400))
            .unwrap()
    );
    assert!(
        !game
            .execute_automatic(roll, now + Duration::from_millis(400))
            .unwrap()
    );
    let decision = game
        .automatic_job(now + Duration::from_millis(800))
        .unwrap();
    let action = score(&game, 0);
    let coaching = game.token(JobPurpose::Coaching);
    assert!(
        !game
            .accept_computer_decision(coaching, action, now)
            .unwrap()
    );
    assert!(
        game.play(action, now).is_err(),
        "human command cannot execute a computer action"
    );
    assert!(
        game.accept_computer_decision(decision.token, Action::Write { category: 999 }, now)
            .is_err()
    );
    assert!(
        game.accept_computer_decision(decision.token, action, now + Duration::from_millis(800))
            .unwrap()
    );
    assert!(
        game.automatic_job(now + Duration::from_millis(1199))
            .is_none()
    );
    let apply = game
        .automatic_job(now + Duration::from_millis(1200))
        .unwrap();
    assert_eq!(apply.step, AutomaticStep::Apply(action));
    game.set_computer_paused(true);
    assert!(game.automatic_job(now + Duration::from_secs(1)).is_none());
    assert!(
        !game
            .execute_automatic(apply, now + Duration::from_secs(2))
            .unwrap()
    );
    game.set_computer_paused(false);
    assert!(
        !game
            .accept_computer_decision(decision.token, action, now)
            .unwrap()
    );
    let decision = game.automatic_job(now).unwrap();
    assert!(
        game.accept_computer_decision(decision.token, action, now)
            .unwrap()
    );
    game.set_editing(true);
    assert!(game.automatic_job(now + Duration::from_secs(1)).is_none());
    game.set_editing(false);
    assert!(
        !game
            .accept_computer_decision(decision.token, action, now)
            .unwrap()
    );
    let decision = game.automatic_job(now).unwrap();
    let checkpoint = game.session().clone();
    game.replace_session(checkpoint).unwrap();
    assert!(
        !game
            .accept_computer_decision(decision.token, action, now)
            .unwrap()
    );
    assert!(
        !game.can_undo(),
        "load begins a new history even on a computer turn"
    );
    let decision = game.automatic_job(now).unwrap();
    assert!(!game.undo(), "no human decision exists in the new history");
    assert!(
        !game
            .accept_computer_decision(decision.token, action, now)
            .unwrap()
    );
}

#[test]
fn computer_turn_can_be_saved_loaded_and_undone_while_waiting_to_apply() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("computer.json");
    let mut game = computer_game(DiceSource::Digital);
    let mut now = Instant::now();
    game.roll_digital(now).unwrap();
    let human = game.session().clone();
    game.play(score(&game, 0), now).unwrap();
    now += Duration::from_secs(1);
    game.execute_automatic(game.automatic_job(now).unwrap(), now)
        .unwrap();
    now += Duration::from_secs(1);
    let decision = game.automatic_job(now).unwrap();
    game.accept_computer_decision(decision.token, score(&game, 0), now)
        .unwrap();
    game.save(&path).unwrap();
    assert!(game.undo());
    assert_eq!(game.session(), &human);
    assert!(
        !game
            .accept_computer_decision(decision.token, Action::CrossOut { category: 0 }, now)
            .unwrap()
    );
    game.load(&path).unwrap();
    assert_eq!(game.control(), PlayerControl::Computer);
    assert_eq!(
        game.automatic_job(now).unwrap().step,
        AutomaticStep::Decide,
        "load recomputes, never resurrects a queued move"
    );
    finish_computer_turn(&mut game, &mut now, 0);
    assert_eq!(game.session().position.active_player, 0);
}

#[test]
fn complete_physical_digital_and_mixed_games_without_a_window() {
    for source in [DiceSource::Physical, DiceSource::Digital] {
        for players in [2, 8] {
            let mut game = GameController::local(players, source, 654).unwrap();
            let now = Instant::now();
            for category in 0..13 {
                for actor in 0..players {
                    assert_eq!(game.session().position.active_player, actor);
                    match source {
                        DiceSource::Physical => enter(&mut game, &[1, 2, 3, 4, 5]),
                        DiceSource::Digital => game.roll_digital(now).unwrap(),
                    }
                    game.toggle_keep(2);
                    game.reroll_selected(now).unwrap();
                    if source == DiceSource::Physical {
                        enter(&mut game, &[6, 6, 6, 6]);
                    }
                    game.play(score(&game, category), now).unwrap();
                    game.session().validate().unwrap();
                }
            }
            assert!(matches!(game.session().position.phase, Phase::Finished));
            assert!(game.automatic_job(now).is_none());
            assert!(game.roll_digital(now).is_err());
            assert!(game.undo());
            assert!(matches!(
                game.session().position.phase,
                Phase::Decision { .. }
            ));
            assert!(game.redo());
            assert!(matches!(game.session().position.phase, Phase::Finished));
        }
        let mut game = computer_game(source);
        let mut now = Instant::now();
        for category in 0..13 {
            match source {
                DiceSource::Physical => enter(&mut game, &[6, 5, 4, 3, 2]),
                DiceSource::Digital => game.roll_digital(now).unwrap(),
            }
            game.play(score(&game, category), now).unwrap();
            finish_computer_turn(&mut game, &mut now, category);
        }
        game.session().validate().unwrap();
        assert!(matches!(game.session().position.phase, Phase::Finished));
    }
}

#[test]
fn illegal_commands_do_not_change_position_rng_or_history() {
    let mut game = GameController::local(2, DiceSource::Digital, 777).unwrap();
    let now = Instant::now();
    let before = game.session().clone();
    assert!(game.play(Action::Write { category: 12 }, now).is_err());
    assert_eq!(game.session(), &before);
    assert!(game.history().is_empty());
    game.roll_digital(now).unwrap();
    let before = game.session().clone();
    let history = game.history().to_vec();
    for action in [
        Action::RollAll,
        Action::Write { category: 999 },
        Action::Reroll {
            keep: Dice([255; 6]),
        },
    ] {
        assert!(game.play(action, now).is_err());
        assert_eq!(game.session(), &before);
        assert_eq!(game.history(), history);
    }
}

#[test]
fn pause_survives_undo_redo_and_fast_play_removes_only_presentation_delay() {
    let mut game = computer_game(DiceSource::Digital);
    let now = Instant::now();
    let mut next = game.session().clone();
    next.desktop.as_mut().unwrap().computer.fast_play = true;
    game.edit_session(next).unwrap();
    game.roll_digital(now).unwrap();
    game.play(score(&game, 0), now).unwrap();
    let roll = game.automatic_job(now).unwrap();
    game.execute_automatic(roll, now).unwrap();
    let decision = game.automatic_job(now).unwrap();
    game.accept_computer_decision(decision.token, score(&game, 0), now)
        .unwrap();
    let pending = game.automatic_job(now).unwrap();
    game.set_computer_paused(true);
    assert!(game.undo());
    assert_eq!(game.control(), PlayerControl::Human);
    assert!(game.redo());
    assert_eq!(game.control(), PlayerControl::Computer);
    assert!(game.session().desktop.as_ref().unwrap().computer.paused);
    assert!(game.automatic_job(now).is_none());
    game.set_computer_paused(false);
    assert!(!game.execute_automatic(pending, now).unwrap());
    assert_eq!(game.automatic_job(now).unwrap().step, AutomaticStep::Decide);
}

#[test]
fn configuration_changes_and_new_games_invalidate_computer_jobs() {
    let mut game = computer_game(DiceSource::Digital);
    let now = Instant::now();
    game.roll_digital(now).unwrap();
    let human = game.session().clone();
    game.play(score(&game, 0), now).unwrap();
    let roll = game.automatic_job(now + Duration::from_secs(1)).unwrap();
    game.execute_automatic(roll, now + Duration::from_secs(1))
        .unwrap();
    for edit in 0..5 {
        let stale = game.token(JobPurpose::ComputerDecision);
        let action = score(&game, 0);
        let mut next = game.session().clone();
        match edit {
            0 => next.settings.max_depth += 1,
            1 => next.settings.objective = solver::Objective::ExpectedScore,
            2 => next.desktop.as_mut().unwrap().computer.policy = ComputerPolicy::Greedy,
            3 => next.settings.opponent_policies = vec![knfsolv::policy::Policy::Greedy; 2],
            _ => next.desktop.as_mut().unwrap().advisor.paused = true,
        }
        game.edit_session(next).unwrap();
        assert!(!game.accept_computer_decision(stale, action, now).unwrap());
    }
    assert!(game.undo());
    assert_eq!(
        game.session(),
        &human,
        "settings changed on a computer turn must not break human-decision Undo"
    );
    game.redo();
    let stale = game.token(JobPurpose::ComputerDecision);
    let next = GameController::local(2, DiceSource::Physical, 890)
        .unwrap()
        .session()
        .clone();
    game.replace_session(next).unwrap();
    assert!(!game.accepts(stale));
    assert!(!game.can_undo());
    assert!(!game.can_redo());
    assert!(game.history().is_empty());
}

#[test]
fn cli_and_tui_preserve_desktop_metadata_and_use_shared_analysis_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.json");
    let mut game = GameController::local(2, DiceSource::Digital, 98765).unwrap();
    game.roll_digital(Instant::now()).unwrap();
    game.save(&path).unwrap();
    let metadata = game.session().desktop.clone();
    let mut terminal = knfsolv::tui::App::new(Session::load(&path).unwrap(), Some(path.clone()));
    terminal.command("depth 3").unwrap();
    terminal.command("save").unwrap();
    let restored = Session::load(&path).unwrap();
    assert_eq!(restored.desktop, metadata);
    assert_eq!(restored.settings.max_depth, 3);
    assert_eq!(restored.position, game.session().position);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_knfsolv"))
        .arg("validate")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn local_setup_rotates_order_validates_names_and_preserves_saved_configuration() {
    use knfsolv::game::LocalGameConfig;
    for n in 2..=8 {
        for starting_player in 0..n {
            let config = LocalGameConfig {
                names: (0..n).map(|i| format!(" Seat {i} ")).collect(),
                starting_player,
                dice_source: DiceSource::Digital,
            };
            let session = config.session(42).unwrap();
            assert_eq!(session.position.active_player, starting_player);
            assert_eq!(session.position.turn_order[0], starting_player);
            assert_eq!(session.position.players[0].name, "Seat 0");
            assert_eq!(session.position.phase, Phase::BeforeFirstRoll);
            assert_eq!(session.desktop.as_ref().unwrap().rng.word_position, 0);
            let loaded: Session =
                serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
            assert_eq!(loaded, session);
            loaded.validate().unwrap();
        }
    }
    for names in [
        vec![],
        vec!["Only".into()],
        vec!["N".into(); 9],
        vec!["A".into(), "  ".into()],
    ] {
        assert!(
            LocalGameConfig {
                names,
                ..Default::default()
            }
            .session(0)
            .is_err()
        );
    }
    assert!(
        LocalGameConfig {
            starting_player: 2,
            ..Default::default()
        }
        .session(0)
        .is_err()
    );
}

#[test]
fn configured_local_matches_finish_from_each_dice_source_and_results_follow_undo() {
    use knfsolv::game::LocalGameConfig;
    for n in [2, 8] {
        for dice_source in [DiceSource::Physical, DiceSource::Digital] {
            let config = LocalGameConfig {
                names: (0..n).map(|i| format!("Seat {i}")).collect(),
                starting_player: n - 1,
                dice_source,
            };
            let mut game = GameController::local(2, DiceSource::Physical, 0).unwrap();
            enter(&mut game, &[1; 5]);
            game.start_local(&config, 42).unwrap();
            assert!(!game.can_undo());
            assert!(game.history().is_empty());
            assert!(game.results().is_none());
            for category in 0..13 {
                for turn in 0..n {
                    assert_eq!(game.session().position.active_player, (n - 1 + turn) % n);
                    assert_eq!(game.session().position.phase, Phase::BeforeFirstRoll);
                    assert_eq!(game.dice().faces, [0; 5], "human turns must wait for input");
                    match dice_source {
                        DiceSource::Physical => enter(&mut game, &[6; 5]),
                        DiceSource::Digital => game.roll_digital(Instant::now()).unwrap(),
                    }
                    game.play(score(&game, category), Instant::now()).unwrap();
                }
            }
            let results = game.results().unwrap();
            let high = results.iter().map(|r| r.total).max().unwrap();
            assert!(results.iter().all(|r| r.winner == (r.total == high)));
            if dice_source == DiceSource::Physical {
                assert!(results.iter().all(|r| r.winner));
            }
            game.undo();
            assert!(game.results().is_none());
            assert!(matches!(
                game.session().position.phase,
                Phase::Decision { .. }
            ));
            game.redo();
            assert_eq!(game.results().unwrap(), results);
            let restored = GameController::new(
                serde_json::from_str(&serde_json::to_string(game.session()).unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(restored.results().unwrap(), results);
        }
    }
}

#[test]
fn final_results_include_upper_and_extra_bonuses_and_all_tied_winners() {
    let mut session = Session::new(3);
    for (i, player) in session.position.players.iter_mut().enumerate() {
        player.sheet.cells = [Some(0); 13];
        if i < 2 {
            player.sheet.cells[..6].copy_from_slice(&[
                Some(3),
                Some(6),
                Some(9),
                Some(12),
                Some(15),
                Some(18),
            ]);
            player.sheet.cells[11] = Some(50);
            player.sheet.extra_kniffels = 1;
        }
    }
    session.position.phase = Phase::Finished;
    let results = GameController::new(session).unwrap().results().unwrap();
    for r in &results[..2] {
        assert_eq!(
            (r.upper, r.bonus, r.extra_points, r.total, r.winner),
            (63, 35, 50, 198, true)
        );
    }
    assert_eq!((results[2].total, results[2].winner), (0, false));
}

#[test]
fn computer_setup_and_coaching_use_declared_baselines_and_separate_budgets() {
    use knfsolv::{
        game::{ComputerConfig, ComputerGameConfig},
        policy::Policy,
        solver::Settings,
    };
    for policy in [
        ComputerPolicy::Greedy,
        ComputerPolicy::LegacyOpportunity,
        ComputerPolicy::ContinuationV2,
    ] {
        let config = ComputerGameConfig {
            human_starts: false,
            dice_source: DiceSource::Physical,
            computer: ComputerConfig {
                policy,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut game = GameController::new(config.session(42).unwrap()).unwrap();
        assert_eq!(game.session().position.active_player, 1);
        assert_eq!(game.session().position.turn_order, [1, 0]);
        assert_eq!(game.session().settings.model(1), policy.model());
        assert_eq!(game.coaching_settings().model(1), policy.model());
        assert_eq!(game.computer_settings().model(1), policy.model());
        let mut session = game.session().clone();
        session.desktop.as_mut().unwrap().computer.search = Some(Settings {
            simulations: 7,
            max_depth: 3,
            ..Default::default()
        });
        game.edit_session(session).unwrap();
        assert_eq!(game.coaching_settings().model(1), Policy::Opportunity);
        assert_eq!(game.computer_settings().simulations, 7);
        assert_eq!(game.computer_settings().max_depth, 3);
        assert_eq!(game.session().settings.simulations, 20000);
        let token = game.token(JobPurpose::ComputerDecision);
        game.set_fast_play(true, Instant::now());
        assert!(
            game.accepts(token),
            "presentation speed must not retrain an active search"
        );
    }
}

#[test]
fn declared_legacy_policy_preserves_original_decisions_and_round_trips() {
    use knfsolv::policy::{self, Policy};
    let mut evaluator = policy::Evaluator::new(None);
    for dice in &knfsolv::dice::tables().full {
        for rerolls in 0..=2 {
            let sheet = knfsolv::state::Sheet::default();
            assert_eq!(
                evaluator.choose(&sheet, *dice, rerolls, Policy::LegacyOpportunity),
                policy::legacy_choose(&sheet, *dice, rerolls, Policy::Opportunity)
            );
        }
    }
    assert_eq!(
        serde_json::to_string(&Policy::LegacyOpportunity).unwrap(),
        "\"legacy_opportunity\""
    );
    assert_eq!(
        serde_json::from_str::<Policy>("\"legacy_opportunity\"").unwrap(),
        Policy::LegacyOpportunity
    );
}

#[test]
fn infer_turns_from_counts_and_order_without_mutation() {
    use knfsolv::state::Position;
    for n in 1..=8 {
        let mut p = Position::new(n);
        p.turn_order.rotate_left(n - 1);
        for round in 0..13 {
            for rank in 0..n {
                let before = p.clone();
                assert_eq!(p.infer_next_player().unwrap(), Some(p.turn_order[rank]));
                assert_eq!(p, before);
                p.players[p.turn_order[rank]].sheet.cells[round] = Some(0);
            }
        }
        assert_eq!(p.infer_next_player().unwrap(), None);
    }
    let mut p = Position::new(3);
    p.players[1].sheet.cells[6] = Some(0);
    assert!(p.infer_next_player().is_err());
    p.players[0].sheet.cells[6] = Some(18);
    assert_eq!(p.infer_next_player().unwrap(), Some(2));
    p.players[0].sheet.cells[0] = Some(0);
    assert!(p.infer_next_player().is_err());
}

#[test]
fn editor_turn_changes_dice_and_history_are_atomic() {
    let mut game = GameController::new(Session::new(2)).unwrap();
    enter(&mut game, &[1, 2, 3, 4, 5]);
    let decision = game.session().clone();
    let mut next = decision.clone();
    next.position.players[0].sheet.cells[6] = Some(18);
    game.edit_session(next).unwrap();
    assert_eq!(game.session().position.active_player, 1);
    assert_eq!(game.session().settings.perspective, 1);
    assert_eq!(game.session().position.phase, Phase::BeforeFirstRoll);
    assert_eq!(game.dice().faces, [0; 5]);
    assert!(game.undo());
    assert_eq!(game.session(), &decision);
    assert_eq!(game.dice().faces, [1, 2, 3, 4, 5]);
    assert!(game.redo());
    enter(&mut game, &[6, 6, 6, 2, 3]);
    let phase = game.session().position.phase;
    let mut next = game.session().clone();
    next.position.players[0].sheet.cells[6] = Some(0);
    game.edit_session(next).unwrap();
    assert_eq!(game.session().position.phase, phase);
    assert_eq!(game.dice().faces, [6, 6, 6, 2, 3]);
    game.analyze_before_roll().unwrap();
    assert_eq!(game.dice().faces, [0; 5]);
    assert!(game.undo());
    assert_eq!(game.session().position.phase, phase);
    assert!(game.redo());
    enter(&mut game, &[2, 3]);
    game.analyze_before_roll().unwrap();
    assert_eq!(game.dice().faces, [0; 5]);
    assert!(game.undo());
    assert_eq!(game.dice().faces, [2, 3, 0, 0, 0]);
    assert!(game.redo());
    let mut next = game.session().clone();
    next.position.turn_order.swap(0, 1);
    game.edit_session(next).unwrap();
    assert!(game.session().validate().is_err());
    assert!(!game.session().position.hypothetical);
    let mut next = game.session().clone();
    next.position.players[1].sheet.cells[0] = Some(0);
    game.edit_session(next).unwrap();
    assert_eq!(game.session().position.active_player, 1);
    game.session().validate().unwrap();
}

#[test]
fn editor_completion_and_hypothetical_selection() {
    let mut game = GameController::new(Session::new(2)).unwrap();
    let mut next = game.session().clone();
    next.position.hypothetical = true;
    next.position.active_player = 1;
    next.settings.perspective = 0;
    next.position.players[1].sheet.cells[0] = Some(0);
    game.edit_session(next).unwrap();
    assert_eq!(game.session().position.active_player, 1);
    assert_eq!(game.session().settings.perspective, 0);
    let mut next = game.session().clone();
    for player in &mut next.position.players {
        player.sheet.cells.fill(Some(0));
    }
    game.edit_session(next).unwrap();
    assert_eq!(game.session().position.phase, Phase::Finished);
    assert!(game.results().unwrap().iter().all(|p| p.winner));
    assert!(game.undo());
    assert_eq!(game.session().position.phase, Phase::BeforeFirstRoll);
}
