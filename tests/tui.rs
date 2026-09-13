use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use knfsolv::{persistence::Session, state::Phase, tui::App};
use ratatui::{Terminal, backend::TestBackend};
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
#[test]
fn editor_actual_roll_workflow_and_undo() {
    let mut app = App::new(Session::new(2), None);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('0'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.session.position.players[0].sheet.cells[0], Some(0));
    app.command("undo").unwrap();
    assert_eq!(app.session.position.players[0].sheet.cells[0], None);
    app.command("redo").unwrap();
    assert_eq!(app.session.position.players[0].sheet.cells[0], Some(0));
    app.command("undo").unwrap();
    app.command("roll 6 6 6 6 6").unwrap();
    app.command("write 12").unwrap();
    assert_eq!(app.session.position.active_player, 1);
    assert!(matches!(app.session.position.phase, Phase::BeforeFirstRoll));
    app.session.validate().unwrap();
    let before = app.session.position.clone();
    assert!(app.command("dice 5 1 2 3 4 5").is_err());
    assert_eq!(app.session.position, before);
}
#[test]
fn renders_multiple_terminal_sizes_and_eight_players() {
    for (w, h) in [(40, 16), (80, 24), (120, 40)] {
        let mut app = App::new(Session::new(8), None);
        app.column = 7;
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Kniffel"));
        app.command("help").unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
    }
}

#[test]
fn budget_panel_changes_settings_and_persists_them() {
    let mut app = App::new(Session::new(2), None);
    key(&mut app, KeyCode::Char('b'));
    assert!(app.budget_open);
    key(&mut app, KeyCode::Enter);
    for c in "5000".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.session.settings.time_ms, 5000);
    for _ in 0..3 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Enter);
    for c in "3".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.session.settings.max_depth, 3);
    app.command("samples 100000").unwrap();
    app.command("nodes 200000").unwrap();
    assert!(app.command("nodes 0").is_err());
    assert!(app.command("depth -1").is_err());
    assert_eq!(app.session.settings.max_nodes, 200000);
    assert_eq!(app.session.settings.max_depth, 3);
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Computation budget"));
    assert!(text.contains("MCTS depth (your decisions): 3"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    app.session.save(&path).unwrap();
    let restored = Session::load(&path).unwrap().settings;
    assert_eq!(
        (
            restored.time_ms,
            restored.simulations,
            restored.max_nodes,
            restored.max_depth
        ),
        (5000, 100000, 200000, 3)
    );
    app.command("depth 0").unwrap();
    app.command("undo").unwrap();
    assert_eq!(app.session.settings.max_depth, 3);
    key(&mut app, KeyCode::Esc);
    assert!(!app.budget_open);
}

#[test]
fn dice_picker_requires_complete_roll_and_supports_vim_and_undo() {
    let mut app = App::new(Session::new(2), None);
    key(&mut app, KeyCode::Char('d'));
    key(&mut app, KeyCode::Enter);
    assert!(matches!(app.session.position.phase, Phase::BeforeFirstRoll));
    for c in "12345".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Char('k')); // last face 5 -> 6
    key(&mut app, KeyCode::Enter);
    match app.session.position.phase {
        Phase::Decision { dice, rerolls_left } => {
            assert_eq!(dice.faces(), vec![1, 2, 3, 4, 6]);
            assert_eq!(rerolls_left, 2);
        }
        _ => panic!("expected decision"),
    }
    let confirmed = app.session.position.clone();
    key(&mut app, KeyCode::Char('d'));
    key(&mut app, KeyCode::Char('6'));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.session.position, confirmed);
    key(&mut app, KeyCode::Char('u'));
    assert!(matches!(app.session.position.phase, Phase::BeforeFirstRoll));
    app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert_eq!(app.session.position, confirmed);
}

#[test]
fn reroll_picker_locks_kept_dice_and_preserves_roll_count() {
    use knfsolv::{dice::Dice, rules::Action};
    let mut session = Session::new(2);
    session
        .position
        .enter_roll(Dice::from_faces(&[2, 6, 6, 6, 6]).unwrap())
        .unwrap();
    session
        .position
        .apply(Action::Reroll {
            keep: Dice::from_faces(&[6, 6, 6, 6]).unwrap(),
        })
        .unwrap();
    let mut app = App::new(session, None);
    key(&mut app, KeyCode::Char('d'));
    for _ in 0..5 {
        key(&mut app, KeyCode::Char('h'));
    }
    key(&mut app, KeyCode::Char('3'));
    key(&mut app, KeyCode::Enter);
    match app.session.position.phase {
        Phase::Decision { dice, rerolls_left } => {
            assert_eq!(dice.faces(), vec![3, 6, 6, 6, 6]);
            assert_eq!(rerolls_left, 1);
        }
        _ => panic!("expected decision"),
    }
    key(&mut app, KeyCode::Char('d'));
    key(&mut app, KeyCode::Char('2'));
    key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.session.position.phase,
        Phase::Decision {
            rerolls_left: 1,
            ..
        }
    ));
    app.session.validate().unwrap();
}

#[test]
fn vim_navigation_menus_and_picker_render_at_small_sizes() {
    let mut app = App::new(Session::new(2), None);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('l'));
    assert_eq!((app.row, app.column), (1, 1));
    key(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Char('G'));
    assert_eq!((app.row, app.column), (12, 0));
    key(&mut app, KeyCode::Char('g'));
    key(&mut app, KeyCode::Char('?'));
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(app.row, 0); // help navigation must not edit sheet selection
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('b'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.input.as_deref(), Some("samples "));
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Esc);
    for (w, h) in [(40, 16), (80, 24), (120, 40)] {
        key(&mut app, KeyCode::Char('m'));
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Pick / correct actual dice"));
        key(&mut app, KeyCode::Enter); // first menu action opens picker
        terminal.draw(|f| app.draw(f)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Pick actual dice"));
        assert!(text.contains("[?]"));
        key(&mut app, KeyCode::Esc);
    }
}

#[test]
fn scoring_shortcut_previews_and_requires_confirmation() {
    let mut app = App::new(Session::new(2), None);
    app.command("roll 6 6 6 6 6").unwrap();
    app.row = 11;
    key(&mut app, KeyCode::Char('w'));
    assert_eq!(app.session.position.players[0].sheet.cells[11], None);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Kniffel: 50 points"));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.session.position.players[0].sheet.cells[11], None);
    key(&mut app, KeyCode::Char('w'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.session.position.players[0].sheet.cells[11], Some(50));
    assert_eq!(app.session.position.active_player, 1);
    app.session.validate().unwrap();
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.session.position.players[0].sheet.cells[11], None);
}

#[test]
fn advice_mode_and_model_path_commands_are_undoable() {
    let mut app = App::new(Session::new(2), None);
    app.command("advice conservative").unwrap();
    assert!(app.session.settings.conservative_advice);
    app.command("baseline-model data/continuation-v2.json")
        .unwrap();
    assert_eq!(
        app.session.settings.baseline_model.as_deref(),
        Some(std::path::Path::new("data/continuation-v2.json"))
    );
    app.command("baseline-model bundled").unwrap();
    assert!(app.session.settings.baseline_model.is_none());
    app.command("undo").unwrap();
    assert!(app.session.settings.baseline_model.is_some());
    app.command("advice sampled").unwrap();
    assert!(!app.session.settings.conservative_advice);
    assert!(app.command("advice invalid").is_err());
}
