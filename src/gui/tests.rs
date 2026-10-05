use super::*;
use crate::{
    coaching::AnalysisUpdate,
    dice::Dice,
    solver::{self, Cancel},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[test]
fn global_preferences_override_loads_and_undo_without_dirtying_games() {
    let dir = tempfile::tempdir().unwrap();
    let prefs_path = dir.path().join("preferences.json");
    let game_path = dir.path().join("game.json");
    let mut session = ComputerGameConfig::default().session(42).unwrap();
    session.settings.max_depth = 99;
    session.settings.perspective = 1;
    session.desktop.as_mut().unwrap().computer.search = Some(Settings {
        method: Method::Mcts,
        max_depth: 7,
        ..Default::default()
    });
    session.save(&game_path).unwrap();
    let mut p = crate::preferences::Preferences::default();
    p.engine.max_depth = 3;
    p.engine.opponent_policies = vec![Policy::Greedy; 4];
    p.analysis_paused = true;
    p.theme = crate::preferences::Theme::Light;
    crate::persistence::save_json(&prefs_path, &p).unwrap();
    let mut app = Desktop::startup_with(
        session.clone(),
        Some(game_path.clone()),
        crate::persistence::FileStorage(prefs_path.clone()),
        native_worker,
    );
    assert_eq!(app.game.session().settings.max_depth, 3);
    assert_eq!(app.game.session().settings.perspective, 1);
    assert_eq!(app.game.session().settings.opponent_policies.len(), 2);
    assert_eq!(app.game.computer_settings().max_depth, 7);
    assert!(!app.dirty());
    let mut next = app.game.session().clone();
    next.position.players[0].name = "Edited".into();
    app.commit(next);
    let undo_len = app.game.undo_len();
    let mut engine = app.game.session().settings.clone();
    engine.max_depth = 5;
    app.engine_changed(engine);
    assert_eq!(app.game.undo_len(), undo_len);
    assert!(app.report.is_none());
    app.undo(false);
    assert_eq!(
        app.game.session().position.players[0].name,
        session.position.players[0].name
    );
    assert_eq!(app.game.session().settings.max_depth, 5);
    assert!(app.analysis_paused);
    assert!(!app.dirty());
    app.undo(true);
    assert_eq!(app.game.session().settings.max_depth, 5);
    app.save().unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&game_path).unwrap()).unwrap();
    assert_eq!(value["settings"], serde_json::json!({"perspective":1}));
    assert!(value["desktop"].get("advisor").is_none());
    assert_eq!(
        Session::load(&game_path).unwrap().desktop.unwrap().computer,
        session.desktop.as_ref().unwrap().computer
    );
    app.execute(Pending::Load(game_path.clone()));
    assert_eq!(app.game.session().settings.max_depth, 5);
    assert!(!app.dirty());
    let ctx = egui::Context::default();
    ctx.set_theme(egui::ThemePreference::Light);
    app.persist_preferences(&ctx);
    drop(app); // Pending debounce is flushed on normal application destruction.
    let restarted = Desktop::startup_with(
        Session::load(&game_path).unwrap(),
        Some(game_path),
        crate::persistence::FileStorage(prefs_path),
        native_worker,
    );
    assert_eq!(restarted.game.session().settings.max_depth, 5);
    assert!(restarted.analysis_paused);
    assert_eq!(
        restarted.preferences.unwrap().theme,
        crate::preferences::Theme::Light
    );
}

#[test]
fn engine_shortcut_and_progress_labels_cover_waiting_training_validation_and_exact() {
    use crate::solver::{SearchPhase, SearchProgress};
    let mut app = card_fixture();
    let ctx = egui::Context::default();
    for (phase, label) in [
        (SearchPhase::MctsTraining, "Tree depth 3"),
        (SearchPhase::MctsValidation, "Tree depth 3 · validating"),
        (SearchPhase::Exact, "Exact depth 3"),
        (SearchPhase::Rollout, "Full-game rollouts"),
    ] {
        app.report.as_mut().unwrap().progress = Some(SearchProgress {
            phase,
            max_decision_depth: 3,
        });
        let output = advice_render(&mut app, &ctx, vec![]);
        assert!(texts(&output).contains(&label.to_owned()));
    }
    app.engine_tab = false;
    advice_click(&mut app, &ctx, "Engine settings");
    assert!(app.show_settings && app.engine_tab);
    app.report = None;
    let output = advice_render(&mut app, &ctx, vec![]);
    assert!(texts(&output).contains(&"Preparing…".to_owned()));
    for mut waiting in [
        Desktop::new(Session::new(2), None),
        Desktop::new(
            ComputerGameConfig {
                human_starts: false,
                ..Default::default()
            }
            .session(0)
            .unwrap(),
            None,
        ),
    ] {
        let output = advice_render(&mut waiting, &egui::Context::default(), vec![]);
        assert!(texts(&output).contains(&"Engine settings".to_owned()));
    }
}

fn startup(session: Session, path: Option<PathBuf>) -> Desktop {
    let dir = tempfile::tempdir().unwrap();
    let preferences = crate::persistence::FileStorage(dir.keep().join("preferences.json"));
    Desktop::startup_with(session, path, preferences, native_worker)
}
fn native_worker(wake: Wake) -> Box<dyn Computation> {
    Box::new(crate::coaching::ComputationWorker::new(wake))
}
#[test]
fn background_is_opaque_and_covers_viewport_in_both_themes() {
    for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
        let ctx = egui::Context::default();
        ctx.set_visuals(visuals.clone());
        let mut app = Desktop::new(Session::new(2), None);
        assert_eq!(eframe::App::clear_color(&app, &visuals)[3], 1.0);
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 900.0));
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::epaint::Shape::Rect(rect)
                if rect.rect.contains_rect(viewport) && rect.fill == visuals.panel_fill.to_opaque()
        )), "the entire viewport must have an opaque background");
    }
}

#[test]
fn actual_dice_keep_reroll_score_and_undo() {
    let mut app = Desktop::new(Session::new(2), None);
    for face in [2, 6, 6, 6, 6] {
        app.choose_face(face, Instant::now());
    }
    app.apply(Action::Reroll {
        keep: Dice::from_faces(&[6, 6, 6, 6]).unwrap(),
    })
    .unwrap();
    assert_eq!(app.game.dice().locked, [false, true, true, true, true]);
    assert_eq!(app.game.dice().faces, [0, 6, 6, 6, 6]);
    app.choose_face(6, Instant::now());
    assert!(matches!(
        app.game.session().position.phase,
        Phase::Decision {
            rerolls_left: 1,
            ..
        }
    ));
    app.apply(Action::Write { category: 11 }).unwrap();
    assert_eq!(
        app.game.session().position.players[0].sheet.cells[11],
        Some(50)
    );
    assert_eq!(app.game.session().position.active_player, 1);
    app.game.session().validate().unwrap();
    app.undo(false);
    assert_eq!(app.game.session().position.active_player, 0);
    app.undo(true);
    assert_eq!(app.game.session().position.active_player, 1);
}

#[test]
fn corrections_debounce_block_moves_and_preserve_order() {
    let mut app = Desktop::new(Session::new(1), None);
    let now = Instant::now();
    for face in [5, 4, 3, 2, 1] {
        app.choose_face(face, now);
    }
    assert_eq!(app.game.undo_len(), 1);
    assert_eq!(app.game.dice().faces, [5, 4, 3, 2, 1]);
    app.game.select_die(0, false);
    app.choose_face(1, now);
    app.game.select_die(4, false);
    app.choose_face(5, now + Duration::from_millis(100));
    assert!(app.analyze(&egui::Context::default()).is_err());
    assert!(app.apply(Action::Write { category: 12 }).is_err());
    app.flush_correction(now + Duration::from_millis(300));
    assert!(app.dice_draft());
    app.flush_correction(now + Duration::from_millis(350));
    assert!(!app.dice_draft());
    assert_eq!(app.game.dice().faces, [1, 4, 3, 2, 5]);
    assert_eq!(
        app.game.undo_len(),
        1,
        "permuting equal dice does not change the position"
    );

    app.game.select_die(2, false);
    app.choose_face(6, now);
    app.game.select_die(3, false);
    app.choose_face(6, now + Duration::from_millis(100));
    app.flush_correction(now + Duration::from_millis(350));
    assert_eq!(app.game.undo_len(), 2, "one completed correction group");
    app.undo(false);
    assert_eq!(app.game.dice().faces, [1, 4, 3, 2, 5]);
    app.undo(true);
    assert_eq!(app.game.dice().faces, [1, 4, 6, 6, 5]);
}

#[test]
fn stale_results_and_settings_changes_invalidate_advice() {
    let mut session = Session::new(1);
    session.position.players[0].sheet.cells = [Some(0); 13];
    session.position.phase = Phase::Finished;
    let report = solver::analyze(&session.position, &session.settings, Cancel::default()).unwrap();
    let mut app = Desktop::new(session, None);
    app.install_preferences(crate::preferences::Preferences::default());
    let old_key = app.analysis_key();
    app.requested = Some(old_key);
    app.worker.inject(AnalysisUpdate {
        key: old_key,
        report: Ok(report),
        finished: true,
        paused: false,
    });
    let mut engine = app.game.session().settings.clone();
    engine.max_depth = 3;
    app.engine_changed(engine);
    assert_ne!(app.analysis_key(), old_key);
    app.poll();
    assert!(app.report.is_none());
    app.undo(false);
    assert_eq!(app.game.session().settings.max_depth, 3);
}

#[test]
fn partial_entry_survives_settings_and_backspace_skips_retained_slots() {
    let mut app = Desktop::new(Session::new(2), None);
    let now = Instant::now();
    for face in [6, 1, 6] {
        app.choose_face(face, now);
    }
    let mut next = app.game.session().clone();
    next.settings.max_depth = 3;
    next.settings.simulations = 100;
    app.commit(next);
    assert_eq!(app.game.dice().faces, [6, 1, 6, 0, 0]);
    for face in [4, 5] {
        app.choose_face(face, now);
    }
    app.game.toggle_keep(2);
    app.apply(Action::Reroll {
        keep: Dice::from_faces(&[6]).unwrap(),
    })
    .unwrap();
    assert_eq!(app.game.dice().faces, [0, 0, 6, 0, 0]);
    assert_eq!(app.game.dice().locked, [false, false, true, false, false]);
    app.choose_face(2, now);
    app.choose_face(3, now);
    app.clear_die(now);
    assert_eq!(app.game.dice().faces, [2, 0, 6, 0, 0]);
    app.game.select_die(2, false); // Even keyboard selection cannot overwrite a lock.
    for face in [4, 5, 1] {
        app.choose_face(face, now);
    }
    assert_eq!(app.game.dice().faces, [2, 4, 6, 5, 1]);
    app.undo(false);
    assert_eq!(
        app.game.dice().faces,
        [0, 0, 6, 0, 0],
        "Undo removes the entire entered reroll"
    );
    app.undo(false);
    assert_eq!(app.game.dice().faces, [6, 1, 6, 4, 5]);
    assert!(app.game.dice().keep[2]);
}

#[test]
fn unfinished_corrections_cannot_score_or_be_silently_saved() {
    let mut app = Desktop::new(Session::new(2), None);
    for face in [1, 2, 3, 4, 5] {
        app.choose_face(face, Instant::now());
    }
    app.saved = snapshot(app.game.session());
    app.clear_die(Instant::now());
    assert!(app.dirty());
    app.flush_correction(Instant::now() + Duration::from_secs(1));
    assert!(app.apply(Action::Write { category: 12 }).is_err());
    assert!(app.prepare_save().is_err());
    app.undo(false);
    assert_eq!(app.game.dice().faces, [1, 2, 3, 4, 5]);
    assert!(!app.dirty());
    app.game.set_editing(true);
    assert!(app.apply(Action::Write { category: 12 }).is_err());
    app.game.set_editing(false);
    app.apply(Action::CrossOut { category: 5 }).unwrap();
    assert_eq!(
        app.game.session().position.players[0].sheet.cells[5],
        Some(0)
    );
    assert_eq!(app.game.session().position.active_player, 1);
    app.undo(false);
    assert_eq!(app.game.dice().faces, [1, 2, 3, 4, 5]);
    assert_eq!(app.game.session().position.players[0].sheet.cells[5], None);
}

fn card_fixture() -> Desktop {
    let mut session = Session::new(2);
    session.position.phase = Phase::Decision {
        dice: Dice::from_faces(&[1, 2, 3, 4, 5]).unwrap(),
        rerolls_left: 2,
    };
    let mut terminal = Session::new(2);
    for player in &mut terminal.position.players {
        player.sheet.cells = [Some(0); 13];
    }
    terminal.position.phase = Phase::Finished;
    let mut report =
        solver::analyze(&terminal.position, &terminal.settings, Cancel::default()).unwrap();
    report.status = "estimated".into();
    report.recommendation_basis = "baseline_pending_validation".into();
    report.confidence = "insufficient".into();
    report.best_action = Some(Action::CrossOut { category: 11 });
    report.alternatives = [
        Action::Write { category: 12 },
        Action::Reroll {
            keep: Dice::from_faces(&[1, 2]).unwrap(),
        },
        Action::CrossOut { category: 11 },
        Action::Reroll {
            keep: Dice::default(),
        },
        Action::Write { category: 9 },
    ]
    .into_iter()
    .map(|action| Alternative {
        action,
        description: String::new(),
        metrics: Some(Metrics {
            utility: 0.5,
            outright_win: 0.25,
            tied_first: 0.5,
            expected_score: 200.25,
        }),
        samples: 42,
        interval: Some([0.2, 0.8]),
        baseline_comparison: None,
    })
    .collect();
    report.alternatives[2].metrics = None;
    report.sampled_best_action = Some(report.alternatives[0].action);
    let mut app = Desktop::new(session, None);
    app.analysis_paused = true;
    app.report = Some(report);
    app
}
fn advice_render(
    app: &mut Desktop,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 1800.),
            )),
            events,
            ..Default::default()
        },
        |ui| app.advice_ui(ui),
    )
}
fn texts(output: &egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}
fn advice_click(app: &mut Desktop, ctx: &egui::Context, label: &str) {
    let output = advice_render(app, ctx, vec![]);
    let pos = label_center(&output, label);
    for pressed in [true, false] {
        advice_render(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}
fn click_alternative(app: &mut Desktop, ctx: &egui::Context, action: Action) {
    advice_render(app, ctx, vec![]);
    let output = advice_render(app, ctx, vec![]);
    let title_y = label_center(&output, &app.move_title(action)).y;
    let pos = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t)
                if t.galley.text() == "Play this move" && t.pos.y > title_y =>
            {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .expect("alternative button");
    for pressed in [true, false] {
        advice_render(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}
#[test]
fn alternative_buttons_score_zero_and_expanded_moves_with_undo() {
    for (action, category, points) in [
        (Action::Write { category: 12 }, 12, 15),
        (Action::CrossOut { category: 11 }, 11, 0),
        (Action::Write { category: 9 }, 9, 30),
    ] {
        let mut app = card_fixture();
        if category == 11 {
            app.report.as_mut().unwrap().best_action = Some(Action::Write { category: 12 });
        }
        app.show_all_moves = true;
        let before = app.game.session().clone();
        let ctx = egui::Context::default();
        click_alternative(&mut app, &ctx, action);
        assert_eq!(
            app.game.session().position.players[0].sheet.cells[category],
            Some(points)
        );
        assert_eq!(app.game.session().position.active_player, 1);
        assert!(app.report.is_none(), "played move invalidates advice");
        app.undo(false);
        assert_eq!(app.game.session().position, before.position);
        app.undo(true);
        assert_eq!(
            app.game.session().position.players[0].sheet.cells[category],
            Some(points)
        );
    }
}
#[test]
fn alternative_rerolls_apply_their_keeps_for_both_dice_sources() {
    for source in [DiceSource::Physical, DiceSource::Digital] {
        let mut app = card_fixture();
        let mut session = app.game.session().clone();
        session.desktop.as_mut().unwrap().dice_source = source;
        session.position.phase = Phase::Decision {
            dice: Dice::from_faces(&[2, 2, 3, 4, 5]).unwrap(),
            rerolls_left: 2,
        };
        app.game = GameController::new(session).unwrap();
        let action = Action::Reroll {
            keep: Dice::from_faces(&[2, 2]).unwrap(),
        };
        app.report.as_mut().unwrap().alternatives[1].action = action;
        app.game.toggle_keep(4); // Manual keeps deliberately differ from the card.
        let ctx = egui::Context::default();
        click_alternative(&mut app, &ctx, action);
        assert_eq!(&app.game.dice().faces[..2], &[2, 2]);
        match source {
            DiceSource::Physical => {
                assert!(matches!(
                    app.game.session().position.phase,
                    Phase::AwaitingRoll { .. }
                ));
                assert_eq!(app.game.dice().locked, [true, true, false, false, false]);
                assert_eq!(app.game.dice().faces, [2, 2, 0, 0, 0]);
            }
            DiceSource::Digital => {
                assert!(matches!(
                    app.game.session().position.phase,
                    Phase::Decision {
                        rerolls_left: 1,
                        ..
                    }
                ));
                assert!(!app.game.dice().faces.contains(&0));
            }
        }
        let after = snapshot(app.game.session());
        app.undo(false);
        assert_eq!(app.game.dice().faces, [2, 2, 3, 4, 5]);
        app.undo(true);
        assert_eq!(snapshot(app.game.session()), after);
    }
}
#[test]
fn alternatives_are_disabled_during_editing_and_incomplete_input() {
    for editing in [true, false] {
        let mut app = card_fixture();
        if editing {
            app.game.set_editing(true);
        } else {
            app.clear_die(Instant::now());
        }
        let before = snapshot(app.game.session());
        let ctx = egui::Context::default();
        if editing {
            click_alternative(&mut app, &ctx, Action::Write { category: 12 });
        } else {
            assert!(
                !texts(&advice_render(&mut app, &ctx, vec![])).contains(&"Play this move".into())
            );
        }
        assert_eq!(snapshot(app.game.session()), before);
    }
}
#[test]
fn move_button_focus_tracks_action_when_estimates_reorder() {
    let mut app = card_fixture();
    app.show_all_moves = true;
    let ctx = egui::Context::default();
    advice_render(&mut app, &ctx, vec![]);
    let action = Action::Write { category: 12 };
    let response_id = egui::Id::new(("move_response", action));
    let (id, _) = ctx.data(|d| d.get_temp::<(egui::Id, egui::Rect)>(response_id).unwrap());
    ctx.memory_mut(|m| m.request_focus(id));
    app.report.as_mut().unwrap().alternatives.reverse();
    advice_render(&mut app, &ctx, vec![]);
    assert_eq!(
        ctx.data(|d| d.get_temp::<(egui::Id, egui::Rect)>(response_id).unwrap().0),
        id
    );
    assert_eq!(ctx.memory(|m| m.focused()), Some(id));
    advice_render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(
        app.game.session().position.players[0].sheet.cells[12],
        Some(15)
    );
}

#[test]
fn pip_dice_are_labeled_and_keyboard_activatable() {
    for face in 1..=6 {
        let ctx = egui::Context::default();
        let mut response = None;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            response = Some(die_button(
                ui,
                face,
                48.,
                false,
                false,
                format!("Enter face {face}"),
            ));
        });
        assert_eq!(
            output
                .shapes
                .iter()
                .filter(|s| matches!(s.shape, egui::epaint::Shape::Circle(_)))
                .count(),
            usize::from(face)
        );
        let response = response.unwrap();
        ctx.memory_mut(|m| m.request_focus(response.id));
        let output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Space,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                assert!(
                    die_button(ui, face, 48., false, false, format!("Enter face {face}")).clicked()
                );
            },
        );
        assert!(
            format!("{:?}", output.platform_output.events).contains(&format!("Enter face {face}"))
        );
    }
}
#[test]
fn digital_controls_hide_picker_and_roll_has_a_large_target() {
    let mut app = Desktop::new(Session::new(2), None);
    let mut session = app.game.session().clone();
    session.desktop.as_mut().unwrap().dice_source = DiceSource::Digital;
    app.game = GameController::new(session).unwrap();
    let ctx = egui::Context::default();
    let output = render(&mut app, &ctx, vec![]);
    let pos = label_center(&output, "Roll");
    assert!(!texts(&output).contains(&"Clear".into()));
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Rect(r)
        if r.rect.contains(pos) && r.rect.width() >= 160. && r.rect.height() >= 44. && r.rect.height() < 60.)));
    app.game.set_editing(true);
    let output = render(&mut app, &ctx, vec![]);
    assert!(texts(&output).contains(&"Clear".into()));
}

#[test]
fn move_cards_disclosures_and_order_survive_report_updates() {
    let mut app = card_fixture();
    let ctx = egui::Context::default();
    ctx.global_style_mut(|s| s.animation_time = 0.);
    advice_render(&mut app, &ctx, vec![]);
    let output = advice_render(&mut app, &ctx, vec![]);
    let labels = texts(&output);
    assert_eq!(labels.iter().filter(|t| *t == "Alternative").count(), 3);
    assert!(output.shapes.iter().any(|s| matches!(
        &s.shape, egui::epaint::Shape::Rect(r)
            if r.stroke.width == 2.0 && r.rect.contains(label_center(&output, "Recommended"))
    )));
    assert_eq!(labels.iter().filter(|t| *t == "Play suggestion").count(), 1);
    assert!(labels.contains(&"Estimating…".into()));
    assert!(labels.iter().any(|t| t.contains("Provisional baseline")));
    assert!(labels.contains(&"Keep none".into()));
    assert!(!labels.iter().any(|t| t.contains("Optimization objective")));
    assert!(
        label_center(&output, "Score Kniffel · 0 points now").y
            < label_center(&output, "Score Chance · 15 points now").y
    );
    assert!(
        label_center(&output, "Score Chance · 15 points now").y
            < label_center(&output, "Reroll 3 dice").y
    );
    advice_click(&mut app, &ctx, "Show all moves");
    advice_click(&mut app, &ctx, "Analysis details");
    let mut update = app.report.clone().unwrap();
    update.samples += 100;
    update.alternatives[2].metrics = Some(Metrics {
        outright_win: 0.,
        tied_first: 1.,
        expected_score: 123.,
        utility: 0.5,
    });
    app.report = Some(update);
    let output = advice_render(&mut app, &ctx, vec![]);
    let labels = texts(&output);
    assert_eq!(labels.iter().filter(|t| *t == "Alternative").count(), 4);
    assert!(labels.contains(&"Show fewer".into()));
    assert!(labels.contains(&"100.0%".into()));
    assert!(
        labels
            .iter()
            .any(|t| t.contains("Simultaneous 95% sampling interval · Shared-first utility"))
    );
    advice_click(&mut app, &ctx, "Show fewer");
    assert!(!app.show_all_moves);
    app.report.as_mut().unwrap().recommendation_basis = "exact".into();
    for status in ["exact_under_model", "incomplete", "provisional"] {
        app.report.as_mut().unwrap().status = status.into();
        let labels = texts(&advice_render(&mut app, &ctx, vec![]));
        assert!(
            labels
                .iter()
                .any(|t| t.contains(&format!("Paused · {}", status.replace('_', " "))))
        );
    }
}
#[test]
fn move_probabilities_and_interval_objectives_are_distinct() {
    assert_eq!(statistic_values(None), ["Estimating…"; 4]);
    assert_eq!(
        statistic_values(Some(Metrics {
            outright_win: 0.23456,
            tied_first: 0.125,
            expected_score: 210.25,
            utility: 0.3
        })),
        ["23.5%", "12.5%", "64.0%", "210.2"]
    );
    let mut app = card_fixture();
    let ctx = egui::Context::default();
    ctx.global_style_mut(|s| s.animation_time = 0.);
    advice_render(&mut app, &ctx, vec![]);
    advice_click(&mut app, &ctx, "Analysis details");
    for (objective, expected) in [
        (
            Objective::SharedFirst,
            "Shared-first utility (ties split credit)",
        ),
        (Objective::StrictFirst, "Outright-first probability"),
        (
            Objective::ExpectedScore,
            "Expected final game score (points)",
        ),
    ] {
        app.report.as_mut().unwrap().settings.objective = objective;
        let labels = texts(&advice_render(&mut app, &ctx, vec![]));
        assert!(
            labels.iter().any(|t| t
                == &format!("Simultaneous 95% sampling interval · {expected}: [0.2000, 0.8000]"))
        );
    }
}
#[test]
fn expanded_advice_recommendation_fits_compact_and_wide_themes() {
    for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
        for size in [[800., 600.], [1180., 900.], [360., 300.]] {
            let ctx = egui::Context::default();
            ctx.set_visuals(visuals.clone());
            let mut app = card_fixture();
            for _ in 0..2 {
                render_size(&mut app, &ctx, vec![], size);
            }
            let output = render_size(&mut app, &ctx, vec![], size);
            if size[0] >= 800. {
                for label in [
                    "Recommended",
                    "Win",
                    "Tie",
                    "Lose",
                    "Expected final game score",
                    "Play suggestion",
                ] {
                    let pos = label_center(&output, label);
                    assert!(pos.x < size[0] && pos.y < size[1], "{label} at {pos:?}");
                    assert!(
                        output.shapes.iter().any(|s| match &s.shape {
                            egui::epaint::Shape::Text(t) if t.galley.text() == label =>
                                s.clip_rect.contains(pos),
                            _ => false,
                        }),
                        "{label} is clipped"
                    );
                }
            }
        }
    }
}

fn render(app: &mut Desktop, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    render_size(app, ctx, events, [800.0, 600.0])
}
fn render_size(
    app: &mut Desktop,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    size: [f32; 2],
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size.into())),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    )
}

fn picker_center(ctx: &egui::Context, face: u8) -> egui::Pos2 {
    ctx.data(|d| {
        d.get_temp::<egui::Rect>(egui::Id::new(("picker_rect", face)))
            .unwrap()
            .center()
    })
}

fn label_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) if t.galley.text() == label => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing rendered label: {label}"))
}

fn click(app: &mut Desktop, ctx: &egui::Context, pos: egui::Pos2) -> egui::FullOutput {
    click_size(app, ctx, pos, [800.0, 600.0])
}
fn click_size(
    app: &mut Desktop,
    ctx: &egui::Context,
    pos: egui::Pos2,
    size: [f32; 2],
) -> egui::FullOutput {
    render_size(
        app,
        ctx,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        size,
    );
    render_size(
        app,
        ctx,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        size,
    )
}

#[test]
fn five_palette_clicks_commit_and_one_score_click_advances() {
    let ctx = egui::Context::default();
    let mut app = Desktop::new(Session::new(2), None);
    render(&mut app, &ctx, vec![]);
    for face in [5, 4, 6, 2, 3] {
        render(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, picker_center(&ctx, face));
    }
    assert_eq!(app.game.dice().faces, [5, 4, 6, 2, 3]);
    assert!(matches!(
        app.game.session().position.phase,
        Phase::Decision { .. }
    ));
    assert_eq!(app.game.undo_len(), 1);
    let output = render(&mut app, &ctx, vec![]);
    let zero = label_center(&output, "0"); // Ones: one click crosses out.
    assert!(zero.y < 600.0);
    click(&mut app, &ctx, zero);
    assert_eq!(
        app.game.session().position.players[0].sheet.cells[0],
        Some(0)
    );
    assert_eq!(app.game.session().position.active_player, 1);
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Undo"));
    assert_eq!(app.game.dice().faces, [5, 4, 6, 2, 3]);
    assert_eq!(app.game.session().position.active_player, 0);
}

#[test]
fn number_keys_enter_a_complete_roll_and_keep_advice_visible() {
    for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
        let ctx = egui::Context::default();
        ctx.set_visuals(visuals);
        let mut app = Desktop::new(Session::new(2), None);
        let events = [
            egui::Key::Num6,
            egui::Key::Num5,
            egui::Key::Num4,
            egui::Key::Num3,
            egui::Key::Num2,
        ]
        .into_iter()
        .map(|key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        })
        .collect();
        render(&mut app, &ctx, events);
        let output = render(&mut app, &ctx, vec![]);
        assert_eq!(app.game.dice().faces, [6, 5, 4, 3, 2]);
        assert_eq!(app.game.undo_len(), 1);
        for label in ["Dice", "Advice", "Pause analysis", "Score sheets"] {
            let center = label_center(&output, label);
            assert!(
                center.x < 800.0 && center.y < 600.0,
                "{label} is outside the viewport"
            );
        }
    }
}

#[test]
fn recommendation_stays_separate_from_keeps_and_plays_in_one_click() {
    let mut session = Session::new(1);
    session.position.players[0].sheet.cells = [Some(0); 13];
    session.position.players[0].sheet.cells[12] = None;
    session.position.phase = Phase::Decision {
        dice: Dice::from_faces(&[6; 5]).unwrap(),
        rerolls_left: 0,
    };
    let report = solver::analyze(&session.position, &session.settings, Cancel::default()).unwrap();
    assert_eq!(report.best_action, Some(Action::Write { category: 12 }));
    let mut app = Desktop::new(session, None);
    app.report = Some(report);
    let ctx = egui::Context::default();
    for size in [[800.0, 600.0], [1180.0, 900.0]] {
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size.into())),
                    ..Default::default()
                },
                |ui| app.show(ui),
            ));
        }
        let output = output.unwrap();
        let heading = label_center(&output, "Advice");
        let play = label_center(&output, "Play suggestion");
        assert!(play.x < size[0] && play.y < size[1]);
        assert!(
            play.y > heading.y,
            "advice must use a vertical layout in wide windows too"
        );
    }
    render(&mut app, &ctx, vec![]);
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Play suggestion"));
    assert_eq!(
        app.game.session().position.players[0].sheet.cells[12],
        Some(30)
    );
    assert!(matches!(app.game.session().position.phase, Phase::Finished));
    app.undo(false);
    assert_eq!(app.game.session().position.players[0].sheet.cells[12], None);

    // An advice marker cannot change the user's retained selection.
    let mut report = solver::analyze(
        &app.game.session().position,
        &app.game.session().settings,
        Cancel::default(),
    )
    .unwrap();
    report.best_action = Some(Action::Reroll {
        keep: Dice::from_faces(&[6, 6]).unwrap(),
    });
    app.report = Some(report);
    assert_eq!(app.suggested_keeps(), [true, true, false, false, false]);
    assert_eq!(app.game.dice().keep, [false; 5]);
}

#[test]
fn saving_before_load_does_not_overwrite_load_target() {
    let dir = tempfile::tempdir().unwrap();
    let current = dir.path().join("current.json");
    let target = dir.path().join("target.json");
    Session::new(2).save(&current).unwrap();
    Session::new(4).save(&target).unwrap();
    let mut app = Desktop::new(Session::load(&current).unwrap(), Some(current.clone()));
    let mut next = app.game.session().clone();
    next.position.players[0].name = "Changed".into();
    app.commit(next);
    app.path = target.display().to_string();
    app.request(Pending::Load(target.clone()));
    assert!(app.pending.is_some());
    app.save_before_pending().unwrap();
    let pending = app.pending.take().unwrap();
    app.execute(pending);
    assert_eq!(
        Session::load(&current).unwrap().position.players[0].name,
        "Changed"
    );
    assert_eq!(Session::load(&target).unwrap().position.players.len(), 4);
    assert_eq!(app.game.session().position.players.len(), 4);
    assert!(!app.dirty());
}

#[test]
fn digital_roll_button_uses_controller_and_never_rolls_on_turn_start() {
    let session = GameController::local(2, DiceSource::Digital, 123)
        .unwrap()
        .session()
        .clone();
    let mut app = Desktop::new(session, None);
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let output = render(&mut app, &ctx, vec![]);
    assert!(matches!(
        app.game.session().position.phase,
        Phase::BeforeFirstRoll
    ));
    assert!(app.game.history().is_empty());
    click(&mut app, &ctx, label_center(&output, "Roll"));
    assert!(matches!(
        app.game.session().position.phase,
        Phase::Decision { .. }
    ));
    assert_eq!(app.game.history().len(), 1);
    assert_eq!(app.game.undo_len(), 1);
    let faces = app.game.dice().faces;
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Undo"));
    assert!(matches!(
        app.game.session().position.phase,
        Phase::BeforeFirstRoll
    ));
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Redo"));
    assert_eq!(app.game.dice().faces, faces);
}

#[test]
fn loading_a_legacy_session_clears_undo_and_rejects_queued_advice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.json");
    let mut session = Session::new(1);
    session.position.players[0].sheet.cells = [Some(0); 13];
    session.position.phase = Phase::Finished;
    session.save(&path).unwrap();
    let report = solver::analyze(&session.position, &session.settings, Cancel::default()).unwrap();
    let mut app = Desktop::new(Session::new(2), None);
    for face in [6; 5] {
        app.choose_face(face, Instant::now());
    }
    let key = app.analysis_key();
    app.worker.inject(AnalysisUpdate {
        key,
        report: Ok(report),
        finished: true,
        paused: false,
    });
    app.execute(Pending::Load(path));
    app.poll();
    assert!(app.game.editing());
    assert!(!app.game.can_undo());
    assert!(!app.game.can_redo());
    assert!(app.report.is_none());
    assert_ne!(app.analysis_key(), key);
}

#[test]
fn editor_forecasts_without_dice_and_rejects_stale_work() {
    let mut session = Session::new(2);
    session.settings.method = Method::Rollout;
    session.settings.simulations = 16;
    session.settings.time_ms = 0;
    let mut app = Desktop::new(session, Some("position.json".into()));
    let ctx = egui::Context::default();
    app.analysis_controls(AnalysisMode::Limited, true);
    app.ensure_analysis(&ctx);
    assert!(app.requested.is_none());
    let mut next = app.game.session().clone();
    next.position.players[0].sheet.cells[6] = Some(18);
    app.commit(next);
    assert!(app.analysis_eligible());
    assert!(app.analysis_paused);
    app.analysis_controls(AnalysisMode::Limited, false);
    app.ensure_analysis(&ctx);
    let old = app.requested.unwrap();
    assert_eq!(old.token.actor, 1);
    wait_report(&mut app, |a| a.analysis_done);
    let report = app.report.clone().unwrap();
    assert_eq!(report.settings.perspective, 1);
    assert_eq!(report.settings.simulations, 16);
    assert!(report.samples <= 16);
    let labels = texts(&advice_render(&mut app, &ctx, vec![]));
    assert!(
        labels
            .iter()
            .any(|t| t == "Before-roll forecast · Player 2")
    );
    assert!(labels.iter().any(|t| t == "Expected final game score"));
    assert!(
        !labels
            .iter()
            .any(|t| t.contains("Roll all") || t.contains("Play this"))
    );
    app.analysis_controls(AnalysisMode::Continuous, false);
    wait_report(&mut app, |a| {
        a.report.as_ref().is_some_and(|r| r.samples > 16)
    });
    app.analysis_controls(AnalysisMode::Continuous, true);
    assert_eq!(app.requested, Some(old));
    app.choose_face(2, Instant::now());
    assert!(!app.analysis_eligible());
    app.worker.inject(AnalysisUpdate {
        paused: false,
        key: old,
        report: Ok(report),
        finished: true,
    });
    app.poll();
    assert!(app.report.is_none());
    for face in [2, 3, 4, 5] {
        app.choose_face(face, Instant::now());
    }
    assert!(app.analysis_eligible());
    assert!(!app.before_roll_forecast());
    app.game.analyze_before_roll().unwrap();
    app.refresh_revision();
    assert!(app.analysis_eligible());
    for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
        ctx.set_theme(theme);
        for size in [[1180., 900.], [800., 600.], [360., 300.]] {
            render_size(&mut app, &ctx, vec![], size);
            render_size(&mut app, &ctx, vec![], size);
        }
    }
    let mut next = app.game.session().clone();
    next.position.players[0].sheet.cells[0] = Some(0);
    app.commit(next);
    assert!(!app.analysis_eligible());
    app.ensure_analysis(&ctx);
    assert!(app.requested.is_none());
    app.undo(false);
    assert!(app.analysis_eligible());
    app.undo(true);
    assert!(!app.analysis_eligible());
}

fn wheel_event(unit: egui::MouseWheelUnit, y: f32) -> egui::Event {
    egui::Event::MouseWheel {
        unit,
        delta: egui::vec2(0., y),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }
}
fn score_cell_center(ctx: &egui::Context, player: usize, category: usize) -> egui::Pos2 {
    ctx.data(|d| {
        d.get_temp::<egui::Rect>(egui::Id::new(("score_cell_rect", player, category)))
            .unwrap()
            .center()
    })
}
fn wheel_at(app: &mut Desktop, ctx: &egui::Context, player: usize, category: usize, y: f32) {
    let pos = score_cell_center(ctx, player, category);
    render(
        app,
        ctx,
        vec![
            egui::Event::PointerMoved(pos),
            wheel_event(egui::MouseWheelUnit::Line, y),
        ],
    );
}

#[test]
fn score_wheel_bounds_cover_every_category() {
    for category in 0..13 {
        let scores = rules::category_scores(category);
        let mut value = None;
        for &expected in scores {
            value = wheel_score(category, value, 1, ScoreWheelLimits::StopAtEnds);
            assert_eq!(value, Some(expected));
        }
        assert_eq!(
            wheel_score(category, value, 1, ScoreWheelLimits::StopAtEnds),
            value
        );
        assert_eq!(
            wheel_score(category, value, 1, ScoreWheelLimits::WrapAround),
            None
        );
        for expected in scores.iter().rev().skip(1).copied().map(Some).chain([None]) {
            value = wheel_score(category, value, -1, ScoreWheelLimits::StopAtEnds);
            assert_eq!(value, expected);
        }
        assert_eq!(
            wheel_score(category, None, -1, ScoreWheelLimits::StopAtEnds),
            None
        );
        assert_eq!(
            wheel_score(category, None, -1, ScoreWheelLimits::WrapAround),
            scores.last().copied()
        );
    }
}

#[test]
fn score_wheel_raw_input_accumulation_consumption_and_resets() {
    let ctx = egui::Context::default();
    let mut wheel = ScoreWheel::default();
    let mut run = |time, cell, events| {
        let mut result = vec![];
        let _ = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                wheel.begin_frame(ui.ctx());
                result = wheel.steps(ui.ctx(), cell);
            },
        );
        result
    };
    let point = |y| wheel_event(egui::MouseWheelUnit::Point, y);
    assert!(run(0., Some((0, 0)), vec![point(25.)]).is_empty());
    assert_eq!(run(0.1, Some((0, 0)), vec![point(55.)]), [1, 1]);
    assert!(run(0.2, Some((0, 0)), vec![]).is_empty());
    assert_eq!(ctx.input(|i| i.smooth_scroll_delta.y), 0.);
    assert!(run(0.3, Some((0, 0)), vec![point(30.)]).is_empty());
    assert!(run(0.4, Some((0, 1)), vec![point(20.)]).is_empty());
    assert!(run(0.5, Some((0, 1)), vec![point(-20.)]).is_empty());
    assert_eq!(run(0.6, Some((0, 1)), vec![point(-20.)]), [-1]);
    assert!(run(1., Some((0, 1)), vec![point(30.)]).is_empty());
    assert!(run(2., Some((0, 1)), vec![point(20.)]).is_empty());
    assert!(run(2.1, None, vec![]).is_empty());
    assert!(run(2.2, Some((0, 1)), vec![point(20.)]).is_empty());
    let modified = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0., 1.),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::CTRL,
    };
    assert!(run(2.3, Some((0, 1)), vec![modified]).is_empty());
    assert_eq!(
        run(
            2.4,
            Some((0, 1)),
            vec![
                wheel_event(egui::MouseWheelUnit::Line, 3.),
                wheel_event(egui::MouseWheelUnit::Line, -3.)
            ]
        ),
        [1, -1]
    );
}

#[test]
fn score_wheel_editor_commits_history_turns_and_manual_entry() {
    let mut app = Desktop::new(Session::new(2), Some("position.json".into()));
    app.analysis_paused = true;
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    let pos = score_cell_center(&ctx, 0, 0);
    let revision = app.game.revision();
    let stale_key = app.analysis_key();
    wheel_at(&mut app, &ctx, 0, 0, -1.);
    assert_eq!(app.game.revision(), revision);
    wheel_at(&mut app, &ctx, 0, 0, 1.);
    assert_eq!(
        app.game.session().position.players[0].sheet.cells[0],
        Some(0)
    );
    assert_eq!(app.game.session().position.active_player, 1);
    app.worker.inject(AnalysisUpdate {
        key: stale_key,
        report: Ok(card_fixture().report.unwrap()),
        finished: true,
        paused: false,
    });
    app.poll();
    assert!(app.report.is_none());
    let history = app.game.undo_len();
    for face in [1, 2, 3, 4, 5] {
        app.choose_face(face, Instant::now());
    }
    let decision = app.game.session().position.phase;
    wheel_at(&mut app, &ctx, 0, 0, 1.);
    assert_eq!(
        app.game.session().position.players[0].sheet.cells[0],
        Some(1)
    );
    assert_eq!(app.game.session().position.phase, decision);
    assert_eq!(app.game.undo_len(), history + 2);
    app.undo(false);
    assert_eq!(
        app.game.session().position.players[0].sheet.cells[0],
        Some(0)
    );
    app.undo(true);
    assert_eq!(
        app.game.session().position.players[0].sheet.cells[0],
        Some(1)
    );
    wheel_at(&mut app, &ctx, 0, 0, -1.);
    wheel_at(&mut app, &ctx, 0, 0, -1.);
    assert_eq!(app.game.session().position.phase, Phase::BeforeFirstRoll);
    assert_eq!(app.game.dice().faces, [0; 5]);
    assert_eq!(app.game.session().position.active_player, 0);
    // A consumed gesture and its smoothing tail must not move the sheet.
    for _ in 0..10 {
        render(&mut app, &ctx, vec![]);
    }
    assert_eq!(score_cell_center(&ctx, 0, 0), pos);
    let history = app.game.undo_len();
    app.score_wheel_limits = ScoreWheelLimits::WrapAround;
    wheel_at(&mut app, &ctx, 0, 0, -1.);
    assert_eq!(
        app.game.session().position.players[0].sheet.cells[0],
        Some(5)
    );
    assert_eq!(app.game.undo_len(), history + 1);
    let pos = score_cell_center(&ctx, 0, 0);
    click(&mut app, &ctx, pos);
    assert!(app.edit.is_some());
    let before = app.game.session().clone();
    wheel_at(&mut app, &ctx, 0, 0, 1.);
    assert_eq!(app.game.session(), &before);
}

#[test]
fn score_wheel_scope_validation_settings_and_layouts() {
    for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
        for size in [[1180., 900.], [800., 600.], [360., 300.]] {
            let ctx = egui::Context::default();
            ctx.set_theme(theme);
            let mut app = Desktop::new(Session::new(2), Some("position.json".into()));
            app.analysis_paused = true;
            for _ in 0..2 {
                render_size(&mut app, &ctx, vec![], size);
            }
            let original = score_cell_center(&ctx, 0, 0);
            let pointer = if size[0] >= 950. {
                egui::pos2(600., 800.)
            } else {
                egui::pos2(size[0] - 40., size[1] - 40.)
            };
            render_size(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pointer),
                    wheel_event(egui::MouseWheelUnit::Line, -3.),
                ],
                size,
            );
            for _ in 0..15 {
                render_size(&mut app, &ctx, vec![], size);
            }
            if size[0] < 950. {
                assert!(score_cell_center(&ctx, 0, 0).y < original.y);
            }
            assert!(
                app.game.session().position.players[0]
                    .sheet
                    .cells
                    .iter()
                    .all(Option::is_none)
            );
        }
    }
    let ctx = egui::Context::default();
    let mut app = Desktop::new(Session::new(2), Some("position.json".into()));
    app.analysis_paused = true;
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    wheel_at(&mut app, &ctx, 1, 0, 1.);
    assert!(!app.analysis_eligible());
    assert!(!app.game.session().position.hypothetical);
    app.undo(false);
    assert!(app.analysis_eligible());
    let mut next = app.game.session().clone();
    next.position.hypothetical = true;
    next.position.players[0].sheet.cells[11] = Some(50);
    next.position.players[0].sheet.cells[0] = Some(5);
    next.position.players[0].sheet.extra_kniffels = 1;
    app.commit(next);
    render(&mut app, &ctx, vec![]);
    // Reopening would leave too few completed turns for the recorded extra Kniffel.
    app.score_wheel_limits = ScoreWheelLimits::WrapAround;
    let before = app.game.session().clone();
    wheel_at(&mut app, &ctx, 0, 0, 1.);
    assert_eq!(app.game.session(), &before);
    assert!(app.message.contains("Extra Kniffels"));
    let history = app.game.undo_len();
    let saved = snapshot(app.game.session());
    app.install_preferences(crate::preferences::Preferences::default());
    app.score_wheel_limits = ScoreWheelLimits::WrapAround;
    app.persist_preferences(&ctx);
    assert_eq!(
        app.preferences.as_ref().unwrap().score_wheel_limits,
        ScoreWheelLimits::WrapAround
    );
    assert_eq!(app.game.undo_len(), history);
    assert_eq!(snapshot(app.game.session()), saved);
    // Local historical editing keeps ordinary wheel scrolling.
    let session = GameController::local(2, DiceSource::Physical, 1)
        .unwrap()
        .session()
        .clone();
    let mut local = Desktop::new(session, None);
    local.game.set_editing(true);
    render(&mut local, &ctx, vec![]);
    render(&mut local, &ctx, vec![]);
    let before = local.game.session().clone();
    wheel_at(&mut local, &ctx, 0, 0, 1.);
    assert_eq!(local.game.session(), &before);
    let mut finished = Desktop::new(Session::new(2), Some("position.json".into()));
    finished.analysis_paused = true;
    let mut next = finished.game.session().clone();
    for player in &mut next.position.players {
        player.sheet.cells.fill(Some(0));
    }
    finished.commit(next);
    assert_eq!(finished.game.session().position.phase, Phase::Finished);
    finished.review_results = true;
    render(&mut finished, &ctx, vec![]);
    render(&mut finished, &ctx, vec![]);
    wheel_at(&mut finished, &ctx, 1, 0, -1.);
    assert_eq!(
        finished.game.session().position.players[1].sheet.cells[0],
        None
    );
    assert_eq!(finished.game.session().position.active_player, 1);
    assert!(finished.analysis_eligible());
}

fn wait_report(app: &mut Desktop, condition: impl Fn(&Desktop) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll();
        if condition(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "missing background coaching update"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn coaching_starts_automatically_and_pause_resume_preserves_evidence() {
    let mut session = Session::load(std::path::Path::new("examples/trail.json")).unwrap();
    session.settings.method = Method::Rollout;
    session.settings.time_ms = 0;
    session.settings.simulations = 96;
    session.settings.policy = Policy::Greedy;
    session.desktop = Some(DesktopGame::new(2, DiceSource::Physical, 77));
    session.desktop.as_mut().unwrap().advisor.mode = AnalysisMode::Limited;
    let mut app = Desktop::new(session, None);
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let key = app.requested.expect("valid loaded roll starts coaching");
    wait_report(&mut app, |a| a.analysis_done);
    assert_eq!(app.report.as_ref().unwrap().samples, 96);
    app.analysis_controls(AnalysisMode::Continuous, false);
    assert_eq!(app.requested, Some(key));
    wait_report(&mut app, |a| {
        a.report.as_ref().is_some_and(|r| r.samples > 96)
    });
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Pause analysis"));
    wait_report(&mut app, |a| {
        a.report
            .as_ref()
            .is_some_and(|r| r.stop_reason.contains("paused"))
    });
    assert!(app.analysis_paused);
    let samples = app.report.as_ref().unwrap().samples;
    let elapsed = app.report.as_ref().unwrap().elapsed_ms;
    std::thread::sleep(Duration::from_millis(40));
    render(&mut app, &ctx, vec![]);
    assert_eq!(app.report.as_ref().unwrap().samples, samples);
    assert_eq!(app.report.as_ref().unwrap().elapsed_ms, elapsed);
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Resume analysis"));
    wait_report(&mut app, |a| {
        a.report.as_ref().is_some_and(|r| r.samples > samples)
    });
    assert_eq!(app.requested, Some(key));
    // Corrections immediately hide old advice and invalidate its worker key.
    app.clear_die(Instant::now());
    assert!(app.report.is_none());
    render(&mut app, &ctx, vec![]);
    assert!(app.requested.is_none());
    assert_ne!(app.analysis_key(), key);
}

#[test]
fn invalid_positions_and_finished_games_do_not_start_coaching() {
    let mut app = Desktop::new(Session::new(2), None);
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    assert!(app.requested.is_none());
    for face in [6; 5] {
        app.choose_face(face, Instant::now());
    }
    app.game.set_editing(true);
    let mut invalid = app.game.session().clone();
    invalid.position.players[1].sheet.cells[0] = Some(0);
    app.commit(invalid);
    render(&mut app, &ctx, vec![]);
    assert!(app.requested.is_none());
    let mut finished = Session::new(2);
    for player in &mut finished.position.players {
        player.sheet.cells = [Some(0); 13];
    }
    finished.position.phase = Phase::Finished;
    app.game.replace_session(finished).unwrap();
    render(&mut app, &ctx, vec![]);
    assert!(app.requested.is_none());
    assert!(app.report.is_none());
}

#[test]
fn headless_render_handles_sizes_phases_and_dialogs() {
    for size in [
        [360.0, 300.0],
        [600.0, 450.0],
        [800.0, 600.0],
        [1180.0, 900.0],
        [1600.0, 1100.0],
    ] {
        let ctx = egui::Context::default();
        let mut app = Desktop::new(Session::new(8), None);
        for modal in 0..4 {
            app.edit = None;
            app.pending = None;
            app.show_settings = false;
            match modal {
                1 => app.edit = Some((7, 12, "0".into())),
                2 => {
                    for face in [6; 5] {
                        app.choose_face(face, Instant::now());
                    }
                    app.show_settings = true;
                }
                3 => app.pending = Some(Pending::Close),
                _ => (),
            }
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size.into())),
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            assert!(!output.shapes.is_empty());
        }
    }
}
#[test]
fn startup_setup_names_dice_and_starting_player_use_pointer_controls() {
    let ctx = egui::Context::default();
    let mut app = startup(Session::new(2), None);
    assert!(!app.dirty());
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Local game"));
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "4"));
    assert_eq!(app.local_setup.names.len(), 4);
    assert_eq!(app.local_setup.names[3], "Player 4");
    let output = render(&mut app, &ctx, vec![]);
    let name_pos = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) if t.galley.text() == "Player 1" && t.pos.x > 50.0 => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap();
    click(&mut app, &ctx, name_pos);
    render(
        &mut app,
        &ctx,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text("Ada 123".into()),
        ],
    );
    assert_eq!(app.local_setup.names[0], "Ada 123");
    assert_eq!(app.game.dice().faces, [0; 5]);
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "1. Ada 123"));
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "4. Player 4"));
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Digital dice"));
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Start local game"));
    assert_eq!(app.screen, Screen::Play);
    assert!(app.pending.is_none());
    assert_eq!(app.game.session().position.active_player, 3);
    assert_eq!(app.game.session().position.turn_order, [3, 0, 1, 2]);
    assert_eq!(app.game.dice_source(), DiceSource::Digital);
    for _ in 0..3 {
        render(&mut app, &ctx, vec![]);
    }
    assert_eq!(app.game.session().position.phase, Phase::BeforeFirstRoll);
    assert!(app.requested.is_none());
}

#[test]
fn setup_navigation_preserves_current_draft_until_replacement_is_accepted() {
    let ctx = egui::Context::default();
    let mut app = Desktop::new(Session::new(2), None);
    app.choose_face(6, Instant::now());
    let original = app.game.session().clone();
    app.navigate(Screen::Home);
    assert!(app.pending.is_none());
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Return to session"));
    assert_eq!(app.game.dice().faces, [6, 0, 0, 0, 0]);
    app.navigate(Screen::LocalSetup);
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Start local game"));
    assert!(app.pending.is_some());
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Go back"));
    assert_eq!(app.game.session(), &original);
    assert_eq!(app.game.dice().faces[0], 6);
    app.request(Pending::Local(LocalGameConfig::default()));
    let output = render(&mut app, &ctx, vec![]);
    click(
        &mut app,
        &ctx,
        label_center(&output, "Discard and continue"),
    );
    assert_eq!(app.screen, Screen::Play);
    assert_eq!(app.game.dice().faces, [0; 5]);
    assert!(!app.game.can_undo());
}

#[test]
fn start_screen_editor_and_load_preserve_analysis_mode_and_errors() {
    let ctx = egui::Context::default();
    let mut app = startup(Session::new(1), None);
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Analyze a position"));
    let output = render(&mut app, &ctx, vec![]);
    click(
        &mut app,
        &ctx,
        label_center(&output, "Open position editor"),
    );
    assert!(app.game.editing());
    assert!(app.game.session().desktop.is_none());
    assert_eq!(app.game.session().position.players.len(), 1);
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut session = LocalGameConfig::default().session(42).unwrap();
    session.position.players[0].name = "Loaded player".into();
    session.save(file.path()).unwrap();
    app.navigate(Screen::Home);
    app.execute(Pending::Load(file.path().into()));
    assert!(!app.game.editing());
    assert_eq!(app.screen, Screen::Play);
    assert!(!app.dirty());
    app.navigate(Screen::Home);
    app.execute(Pending::Load(file.path().join("missing")));
    assert_eq!(app.screen, Screen::Home);
    assert_eq!(app.game.session(), &session);
}

#[test]
fn complete_physical_and_digital_games_through_gui_rolls_scores_and_results() {
    let size = [1180.0, 900.0];
    for source in [DiceSource::Physical, DiceSource::Digital] {
        let ctx = egui::Context::default();
        let mut app = startup(Session::new(2), None);
        app.local_setup = LocalGameConfig {
            names: vec!["Ada".into(), "Ben".into()],
            starting_player: 1,
            dice_source: source,
        };
        app.navigate(Screen::LocalSetup);
        let output = render_size(&mut app, &ctx, vec![], size);
        click_size(
            &mut app,
            &ctx,
            label_center(&output, "Start local game"),
            size,
        );
        // Avoid timing-dependent advice decoration while exercising legal GUI moves.
        app.analysis_controls(AnalysisMode::Continuous, true);
        let mut original_rows = None;
        for category in 0..13 {
            for turn in 0..2 {
                assert_eq!(app.game.session().position.active_player, (turn + 1) % 2);
                let output = render_size(&mut app, &ctx, vec![], size);
                match source {
                    DiceSource::Digital => {
                        click_size(&mut app, &ctx, label_center(&output, "Roll"), size);
                    }
                    DiceSource::Physical => {
                        for _ in 0..5 {
                            render_size(&mut app, &ctx, vec![], size);
                            click_size(&mut app, &ctx, picker_center(&ctx, 6), size);
                        }
                    }
                }
                let output = render_size(&mut app, &ctx, vec![], size);
                let actor = &app.game.session().position.players
                    [app.game.session().position.active_player]
                    .name;
                let x = label_center(&output, &format!("{actor} (turn)")).x;
                let sheet_y = label_center(&output, "Score sheets").y;
                let rows: Vec<_> = rules::NAMES
                    .iter()
                    .map(|name| {
                        output
                            .shapes
                            .iter()
                            .find_map(|s| match &s.shape {
                                egui::epaint::Shape::Text(t)
                                    if t.galley.text() == *name && t.pos.y > sheet_y =>
                                {
                                    Some(t.pos.y + t.galley.size().y * 0.5)
                                }
                                _ => None,
                            })
                            .unwrap()
                    })
                    .collect();
                if let Some(previous) = &original_rows {
                    assert_eq!(
                        &rows, previous,
                        "scoring must not move the remaining categories"
                    );
                } else {
                    original_rows = Some(rows.clone());
                }
                let y = rows[category];
                assert!(y < size[1]);
                click_size(&mut app, &ctx, egui::pos2(x, y), size);
                assert!(
                    app.game.session().position.players[(turn + 1) % 2]
                        .sheet
                        .cells[category]
                        .is_some(),
                    "score click {source:?} category {category}, turn {turn}, at {x},{y}: {:?}, {}",
                    app.game.session().position.phase,
                    app.message
                );
                assert!(app.pending.is_none());
            }
        }
        assert_eq!(app.game.session().position.phase, Phase::Finished);
        assert!(app.requested.is_none());
        let file = tempfile::NamedTempFile::new().unwrap();
        app.game.save(file.path()).unwrap();
        let mut loaded = startup(
            Session::load(file.path()).unwrap(),
            Some(file.path().into()),
        );
        let loaded_ctx = egui::Context::default();
        let loaded_output = render_size(&mut loaded, &loaded_ctx, vec![], size);
        label_center(&loaded_output, "Game finished");
        assert_eq!(loaded.game.results(), app.game.results());
        assert!(!loaded.game.can_undo());

        let output = render_size(&mut app, &ctx, vec![], size);
        label_center(&output, "Game finished");
        if source == DiceSource::Physical {
            label_center(&output, "Tied winners: Ada, Ben");
        }
        click_size(
            &mut app,
            &ctx,
            label_center(&output, "Review score sheets"),
            size,
        );
        assert!(app.review_results);
        let output = render_size(&mut app, &ctx, vec![], size);
        label_center(&output, "TOTAL");
        click_size(&mut app, &ctx, label_center(&output, "Undo"), size);
        assert!(matches!(
            app.game.session().position.phase,
            Phase::Decision { .. }
        ));
        let output = render_size(&mut app, &ctx, vec![], size);
        click_size(&mut app, &ctx, label_center(&output, "Redo"), size);
        assert_eq!(app.game.session().position.phase, Phase::Finished);
        assert!(!app.review_results);
        let output = render_size(&mut app, &ctx, vec![], size);
        click_size(&mut app, &ctx, label_center(&output, "New game"), size);
        assert_eq!(app.screen, Screen::Home);
    }
}

#[test]
fn setup_and_results_render_in_both_themes_at_small_and_large_sizes() {
    for size in [[360.0, 300.0], [800.0, 600.0], [1180.0, 900.0]] {
        for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
            let ctx = egui::Context::default();
            ctx.set_visuals(visuals);
            let mut app = startup(Session::new(8), None);
            for screen in [
                Screen::Home,
                Screen::LocalSetup,
                Screen::ComputerSetup,
                Screen::AnalysisSetup,
            ] {
                app.navigate(screen);
                let output = render_size(&mut app, &ctx, vec![], size);
                assert!(!output.shapes.is_empty());
                if size[1] >= 600.0 && screen == Screen::LocalSetup {
                    let button = label_center(&output, "Start local game");
                    assert!(button.y < size[1], "eight-player setup fits at 800×600");
                }
            }
            let mut session = LocalGameConfig {
                names: vec!["Seat".into(); 8],
                ..Default::default()
            }
            .session(1)
            .unwrap();
            for p in &mut session.position.players {
                p.sheet.cells = [Some(0); 13];
            }
            session.position.phase = Phase::Finished;
            app.game.replace_session(session).unwrap();
            app.navigate(Screen::Play);
            let output = render_size(&mut app, &ctx, vec![], size);
            label_center(&output, "Game finished");
        }
    }
}
fn computer_app(source: DiceSource, policy: ComputerPolicy, search: Option<Settings>) -> Desktop {
    let config = ComputerGameConfig {
        dice_source: source,
        computer: crate::game::ComputerConfig {
            policy,
            search,
            fast_play: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut app = Desktop::new(config.session(77).unwrap(), None);
    app.analysis_controls(AnalysisMode::Continuous, true);
    app
}
fn wait_computer_update(app: &mut Desktop) -> AnalysisUpdate {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(update) = app
            .worker
            .take_updates()
            .into_iter()
            .find(|u| u.key.token.purpose == JobPurpose::ComputerDecision && u.finished)
        {
            return update;
        }
        assert!(Instant::now() < deadline, "computer worker did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn human_scores_first_open(app: &mut Desktop) {
    match app.game.dice_source() {
        DiceSource::Physical => {
            for _ in 0..5 {
                app.choose_face(6, Instant::now());
            }
        }
        DiceSource::Digital => {
            app.game.roll_digital(Instant::now()).unwrap();
            app.refresh_revision();
        }
    }
    let action = app
        .game
        .session()
        .position
        .actions()
        .into_iter()
        .find(|a| matches!(a, Action::Write { .. } | Action::CrossOut { .. }))
        .unwrap();
    app.apply(action).unwrap();
}
#[test]
fn computer_setup_presets_custom_budgets_and_pause_controls() {
    let ctx = egui::Context::default();
    let mut app = startup(Session::new(2), None);
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Play computer"));
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Strong"));
    assert_eq!(
        app.computer_setup.computer.policy,
        ComputerPolicy::ContinuationV2
    );
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Custom search"));
    let output = render(&mut app, &ctx, vec![]);
    assert_eq!(
        app.computer_setup
            .computer
            .search
            .as_ref()
            .unwrap()
            .simulations,
        20000
    );
    assert!(
        app.computer_setup
            .computer
            .search
            .as_ref()
            .unwrap()
            .conservative_advice
    );
    assert!(label_center(&output, "Start computer game").y < 600.0);
    click(&mut app, &ctx, label_center(&output, "Start computer game"));
    assert_eq!(app.game.control(), PlayerControl::Human);
    assert_eq!(app.game.session().position.phase, Phase::BeforeFirstRoll);
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Pause computer"));
    assert!(app.game.session().desktop.as_ref().unwrap().computer.paused);
    let output = render(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, label_center(&output, "Fast play"));
    assert!(
        app.game
            .session()
            .desktop
            .as_ref()
            .unwrap()
            .computer
            .fast_play
    );
}
#[test]
fn computer_worker_finishes_all_presets_and_custom_games_with_both_dice_sources() {
    for source in [DiceSource::Physical, DiceSource::Digital] {
        for preset in 0..4 {
            let policy = [
                ComputerPolicy::Greedy,
                ComputerPolicy::LegacyOpportunity,
                ComputerPolicy::ContinuationV2,
                ComputerPolicy::ContinuationV2,
            ][preset];
            let search = (preset == 3).then(|| Settings {
                simulations: 16,
                time_ms: 0,
                max_depth: 1,
                max_nodes: 8,
                ..Default::default()
            });
            let mut app = computer_app(source, policy, search);
            let ctx = egui::Context::default();
            let deadline = Instant::now() + Duration::from_secs(20);
            while app.game.session().position.phase != Phase::Finished {
                assert!(
                    Instant::now() < deadline,
                    "opponent did not finish: {}",
                    app.computer_status
                );
                if app.game.control() == PlayerControl::Human {
                    assert_eq!(app.game.session().position.phase, Phase::BeforeFirstRoll);
                    human_scores_first_open(&mut app);
                } else {
                    app.tick_computer(&ctx, Instant::now());
                    app.poll();
                    std::thread::sleep(Duration::from_millis(1));
                }
                app.game.session().validate().unwrap();
            }
            assert!(app.game.results().is_some());
            assert!(app.computer_notice.is_empty());
            app.undo(false);
            assert_eq!(app.game.control(), PlayerControl::Human);
            assert!(matches!(
                app.game.session().position.phase,
                Phase::Decision { .. }
            ));
            app.undo(true);
            assert_eq!(app.game.session().position.phase, Phase::Finished);
        }
    }
}
#[test]
fn computer_pause_undo_load_and_navigation_reject_stale_results_without_fallback() {
    let ctx = egui::Context::default();
    for operation in 0..5 {
        let mut app = computer_app(
            DiceSource::Physical,
            ComputerPolicy::LegacyOpportunity,
            None,
        );
        human_scores_first_open(&mut app);
        app.tick_computer(&ctx, Instant::now()); // actual computer roll
        app.tick_computer(&ctx, Instant::now()); // worker job
        let update = wait_computer_update(&mut app);
        let file = tempfile::NamedTempFile::new().unwrap();
        app.game.save(file.path()).unwrap();
        match operation {
            0 => {
                app.game.set_computer_paused(true);
                app.refresh_revision();
            }
            1 => app.undo(false),
            2 => app.execute(Pending::Load(file.path().into())),
            3 => app.navigate(Screen::Home),
            _ => {
                app.show_settings = true;
                app.tick_computer(&ctx, Instant::now());
            }
        }
        let position = app.game.session().position.clone();
        app.worker.inject(update);
        app.poll();
        assert_eq!(app.game.session().position, position);
        assert!(app.game.pending_computer_action().is_none());
        assert!(app.computer_requested.is_none());
        assert!(
            app.computer_notice.is_empty(),
            "cancellation must not cause fallback"
        );
        if operation == 2 {
            app.tick_computer(&ctx, Instant::now());
            let update = wait_computer_update(&mut app);
            app.computer_update(update, Instant::now());
            assert!(
                app.game.pending_computer_action().is_some(),
                "loaded decision resumes without rerolling"
            );
        }
    }
}
#[test]
fn only_finished_computer_jobs_execute_and_genuine_failure_uses_worker_baseline() {
    let ctx = egui::Context::default();
    let mut app = computer_app(
        DiceSource::Physical,
        ComputerPolicy::ContinuationV2,
        Some(Settings::default()),
    );
    human_scores_first_open(&mut app);
    app.tick_computer(&ctx, Instant::now());
    app.request_computer(false).unwrap();
    let key = app.computer_requested.unwrap();
    let error = AnalysisUpdate {
        key,
        report: Err("test search failure".into()),
        finished: true,
        paused: false,
    };
    let mut coaching_error = error.clone();
    coaching_error.key.token.purpose = JobPurpose::Coaching;
    app.computer_update(coaching_error, Instant::now());
    assert_eq!(app.computer_requested, Some(key));
    assert!(app.computer_notice.is_empty());
    app.computer_update(error, Instant::now());
    assert!(app.computer_fallback);
    assert!(app.computer_notice.contains("test search failure"));
    let fallback = wait_computer_update(&mut app);
    assert_eq!(fallback.report.as_ref().unwrap().samples, 0);
    app.game.set_fast_play(false, Instant::now());
    let now = Instant::now();
    app.computer_update(fallback, now);
    let choice = app.game.pending_computer_action().unwrap();
    if let Action::Reroll { keep } = choice {
        assert_eq!(
            app.game.dice().keep.iter().filter(|&&k| k).count(),
            keep.len()
        );
    }
    assert!(
        app.game
            .automatic_job(now + Duration::from_millis(399))
            .is_none()
    );
    assert!(
        app.game
            .automatic_job(now + Duration::from_millis(400))
            .is_some()
    );
    app.game.set_computer_paused(true);
    app.refresh_revision();
    app.tick_computer(&ctx, now + Duration::from_secs(1));
    assert!(matches!(
        app.game.session().position.phase,
        Phase::Decision { .. }
    ));
}
#[test]
fn provisional_computer_reports_wait_and_exhausted_search_without_advice_falls_back() {
    let ctx = egui::Context::default();
    let mut app = computer_app(
        DiceSource::Digital,
        ComputerPolicy::ContinuationV2,
        Some(Settings::default()),
    );
    human_scores_first_open(&mut app);
    app.tick_computer(&ctx, Instant::now());
    app.request_computer(false).unwrap();
    let key = app.computer_requested.unwrap();
    let report = solver::AnalysisSession::new(
        app.game.session().position.clone(),
        app.game.computer_settings(),
    )
    .unwrap()
    .snapshot();
    app.computer_update(
        AnalysisUpdate {
            key,
            report: Ok(report.clone()),
            finished: false,
            paused: false,
        },
        Instant::now(),
    );
    assert!(app.game.pending_computer_action().is_none());
    assert_eq!(app.computer_requested, Some(key));
    let mut no_advice = report;
    no_advice.best_action = None;
    app.computer_update(
        AnalysisUpdate {
            key,
            report: Ok(no_advice),
            finished: true,
            paused: false,
        },
        Instant::now(),
    );
    assert!(app.computer_fallback);
    assert!(app.computer_notice.contains("budget ended without advice"));
    let fallback = wait_computer_update(&mut app);
    app.computer_update(fallback, Instant::now());
    assert!(app.game.pending_computer_action().is_some());
}

/// Records shared files and hands out one picked file, like the web shell's browser.
#[derive(Clone, Default)]
struct FakeBrowser {
    shared: Rc<RefCell<Vec<String>>>,
    picker_opened: Rc<Cell<usize>>,
    picked: Rc<RefCell<Option<std::result::Result<String, String>>>>,
}
impl Browser for FakeBrowser {
    fn share(&self, json: &str) -> Result<()> {
        self.shared.borrow_mut().push(json.into());
        Ok(())
    }
    fn open(&self) {
        self.picker_opened.set(self.picker_opened.get() + 1);
    }
    fn take_opened(&self) -> Option<std::result::Result<String, String>> {
        self.picked.borrow_mut().take()
    }
}
fn browser_app(session: Session) -> (Desktop, FakeBrowser, egui::Context) {
    let mut app = startup(session, Some("position.json".into()));
    let browser = FakeBrowser::default();
    app.set_browser(Box::new(browser.clone()));
    (app, browser, egui::Context::default())
}
fn open_file_menu(app: &mut Desktop, ctx: &egui::Context) -> egui::FullOutput {
    let output = render(app, ctx, vec![]);
    click(app, ctx, label_center(&output, "File"));
    render(app, ctx, vec![])
}

#[test]
fn browser_shares_desktop_saves_without_paths_or_unsaved_prompts() {
    let mut session = LocalGameConfig::default().session(7).unwrap();
    session.position.players[0].sheet.cells[5] = Some(24);
    session.position.active_player = 1;
    let (mut app, browser, ctx) = browser_app(session.clone());
    let output = open_file_menu(&mut app, &ctx);
    let shown = texts(&output);
    for hidden in ["Session path", "Save", "Load session", "Unsaved"] {
        assert!(!shown.iter().any(|t| t == hidden), "{hidden} is shown");
    }
    click(&mut app, &ctx, label_center(&output, "Share position"));
    let shared = browser.shared.borrow().clone();
    assert_eq!(shared.len(), 1);
    assert_eq!(
        shared[0],
        crate::persistence::to_json(&crate::persistence::desktop_value(&session)).unwrap()
    );
    assert_eq!(app.autosave(), Some(snapshot(&session)));
    // A phone export opens in the desktop app.
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), &shared[0]).unwrap();
    let mut desktop = startup(Session::new(2), None);
    desktop.execute(Pending::Load(file.path().into()));
    assert_eq!(desktop.screen, Screen::Play);
    assert_eq!(desktop.game.session().position, session.position);
    let output = open_file_menu(&mut app, &ctx);
    click(&mut app, &ctx, label_center(&output, "Open position"));
    assert_eq!(browser.picker_opened.get(), 1);
}

#[test]
fn browser_opens_files_and_confirms_before_replacing_entries() {
    let mut current = Session::new(2);
    current.position.players[0].sheet.cells[0] = Some(3);
    current.position.active_player = 1;
    let (mut app, browser, ctx) = browser_app(current.clone());
    let mut opened = Session::new(3);
    opened.position.players[2].name = "Opened".into();
    *browser.picked.borrow_mut() = Some(Ok(crate::persistence::to_json(
        &crate::persistence::desktop_value(&opened),
    )
    .unwrap()));
    render(&mut app, &ctx, vec![]);
    let output = render(&mut app, &ctx, vec![]);
    assert!(texts(&output).contains(&"Discard the current position?".to_string()));
    assert_eq!(app.game.session().position, current.position);
    click(&mut app, &ctx, label_center(&output, "Discard"));
    assert_eq!(app.game.session().position.players[2].name, "Opened");
    assert_eq!(app.screen, Screen::Play);
    // Without entries, opening replaces the position directly.
    *browser.picked.borrow_mut() = Some(Ok(crate::persistence::to_json(
        &crate::persistence::desktop_value(&Session::new(2)),
    )
    .unwrap()));
    render(&mut app, &ctx, vec![]);
    assert_eq!(app.game.session().position.players.len(), 2);
    assert!(app.pending.is_none());
}

#[test]
fn browser_rejects_unreadable_and_invalid_files_without_changes() {
    let mut current = Session::new(2);
    current.position.players[1].name = "Kept".into();
    let (mut app, browser, ctx) = browser_app(current.clone());
    let mut invalid = Session::new(2);
    invalid.settings.perspective = 7;
    for picked in [
        Err("The file is larger than 1 MiB.".to_string()),
        Ok("not json".to_string()),
        Ok(serde_json::to_string(&invalid).unwrap()),
    ] {
        *browser.picked.borrow_mut() = Some(picked);
        render(&mut app, &ctx, vec![]);
        assert!(
            app.message
                .starts_with("This file is not a saved position.")
        );
        assert!(app.pending.is_none());
        assert_eq!(app.game.session().position, current.position);
    }
}
