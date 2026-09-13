use crate::{
    Error, Result,
    dice::Dice,
    persistence::Session,
    policy::Policy,
    rules::{self, Action},
    solver::{self, Cancel, Method, Objective, Report},
    state::Phase,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
    time::{Duration, Instant},
};

const HELP: &str = "QUICK START\nD: pick your five dice, then Enter to confirm. A: analyze. X: apply advice.\nKept dice are locked when entering a reroll. Esc discards picker changes.\nM: action menu (no commands to memorize). W: score selected row for active player.\n\nNAVIGATION\nArrows or h/j/k/l: select cell. g/G: first/last category. Enter: edit sheet.\nD picker: h/l selects die, j/k changes face; 1-6 sets face and advances.\nU: undo. R or Ctrl-R: redo. S: save. B: computation budget. ?: help.\nHelp: j/k or Ctrl-D/U scroll. Esc closes panels. Q: quit (save first).\n\nADVANCED COMMANDS\n: opens commands (player/category numbers start at 1)\nroll 6 6 6 6 2       enter actual dice, including kept dice\ndice 1 6 6 6 6 2     edit position: 1 reroll left, then five dice\nwrite 13             score current dice in category 13 (Chance)\nrecord 13 26         enter an observed score without dice; advance turn\napply 2              apply second-ranked advice\nturn 2 | perspective 1 | order 2 1 | name 2 Anna | extra 2 1\nbefore | finished | next | hypothetical on | new 4\nobjective shared|strict|score | model opportunity|legacy|greedy [player]\nmethod auto|exact|rollout|mcts | time 5000 | samples 20000 | seed 42\nsave position.json | load position.json | cache solitaire-cache.json\nhelp | rules | undo | redo | quit\n\nStandard rules: 63 upper points earn 35 bonus. An additional Kniffel requires\n50 already in Kniffel; it earns 50 extra plus maximum points in any open row.\nA positive matching entry cannot be voluntarily replaced by zero.\nSheets are checked locally; hypothetical mode allows edited turn counts.";

const ACTIONS: &[&str] = &[
    "Pick / correct actual dice",
    "Analyze this position",
    "Apply recommended move",
    "Score selected category",
    "Computation budget",
    "Save session",
    "Undo",
    "Redo",
    "Help",
];

struct DicePicker {
    faces: [u8; 5], // zero means not yet selected, never silently assume a roll
    locked: usize,
    selected: usize,
}

pub struct App {
    pub session: Session,
    undo: Vec<Session>,
    redo: Vec<Session>,
    pub row: usize,
    pub column: usize,
    pub input: Option<String>,
    editing_cell: bool,
    pub message: String,
    pub report: Option<Report>,
    pub help: bool,
    pub budget_open: bool,
    budget_field: usize,
    dice_picker: Option<DicePicker>,
    action_menu: Option<usize>,
    score_confirmation: Option<usize>,
    pub quit: bool,
    scroll: u16,
    revision: u64,
    cancel: Cancel,
    running: Option<Instant>,
    tx: SyncSender<(u64, Result<Report>)>,
    rx: Receiver<(u64, Result<Report>)>,
    save_path: PathBuf,
}
impl App {
    pub fn new(session: Session, path: Option<PathBuf>) -> Self {
        let (tx, rx) = mpsc::sync_channel(8);
        Self {
            session,
            undo: vec![],
            redo: vec![],
            row: 0,
            column: 0,
            input: None,
            editing_cell: false,
            message: "Start: D picks dice | M opens actions | ? help".into(),
            report: None,
            help: false,
            budget_open: false,
            budget_field: 0,
            dice_picker: None,
            action_menu: None,
            score_confirmation: None,
            quit: false,
            scroll: 0,
            revision: 0,
            cancel: Cancel::default(),
            running: None,
            tx,
            rx,
            save_path: path.unwrap_or_else(|| "position.json".into()),
        }
    }
    fn changed(&mut self, previous: Session) {
        self.undo.push(previous);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.invalidate();
    }
    fn invalidate(&mut self) {
        self.dice_picker = None;
        self.score_confirmation = None;
        self.revision += 1;
        self.cancel.cancel();
        self.running = None;
        self.report = None;
        self.scroll = 0;
        self.column = self.column.min(self.session.position.players.len() - 1);
    }
    fn open_dice_picker(&mut self) {
        let mut picker = DicePicker {
            faces: [0; 5],
            locked: 0,
            selected: 0,
        };
        match self.session.position.phase {
            Phase::Finished => {
                self.message = "Game finished. Edit the position before entering dice.".into();
                return;
            }
            Phase::Decision { dice, .. } => picker.faces.copy_from_slice(&dice.faces()),
            Phase::AwaitingRoll { kept, .. } => {
                let faces = kept.faces();
                picker.locked = faces.len();
                picker.faces[..faces.len()].copy_from_slice(&faces);
                picker.selected = picker.locked.min(4);
            }
            Phase::BeforeFirstRoll => (),
        }
        self.help = false;
        self.dice_picker = Some(picker);
        self.message = "Pick each rolled face; Enter confirms, Esc discards.".into();
    }

    fn perform_action(&mut self, index: usize) {
        let command = match index {
            0 => {
                self.open_dice_picker();
                return;
            }
            1 => "analyze".to_string(),
            2 => "apply".to_string(),
            3 => {
                if self.column != self.session.position.active_player {
                    self.message = "Select the active player's column to score a turn.".into();
                    return;
                }
                let mut preview = self.session.position.clone();
                let action = preview.actions().into_iter().find(|a| matches!(a,
                    Action::Write { category } | Action::CrossOut { category } if *category == self.row));
                if let Some(action) = action {
                    if let Err(e) = preview.apply(action) {
                        self.message = e.to_string();
                    } else {
                        self.score_confirmation = Some(self.row);
                    }
                } else {
                    self.message = "Pick dice and select an open category before scoring.".into();
                }
                return;
            }
            4 => "budget".into(),
            5 => "save".into(),
            6 => "undo".into(),
            7 => "redo".into(),
            _ => "help".into(),
        };
        if let Err(e) = self.command(&command) {
            self.message = e.to_string();
        } else if index == 2 && matches!(self.session.position.phase, Phase::AwaitingRoll { .. }) {
            self.open_dice_picker();
        }
    }

    fn picker_key(&mut self, code: KeyCode) {
        let picker = self.dice_picker.as_mut().unwrap();
        match code {
            KeyCode::Esc => {
                self.dice_picker = None;
                self.message = "Dice entry cancelled; position unchanged.".into();
            }
            KeyCode::Left | KeyCode::Char('h') => {
                picker.selected = picker.selected.saturating_sub(1).max(picker.locked.min(4))
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                picker.selected = (picker.selected + 1).min(4)
            }
            KeyCode::BackTab => {
                picker.selected = picker.selected.saturating_sub(1).max(picker.locked.min(4))
            }
            KeyCode::Char(c @ '1'..='6') if picker.selected >= picker.locked => {
                picker.faces[picker.selected] = c as u8 - b'0';
                picker.selected = (picker.selected + 1).min(4);
            }
            KeyCode::Up | KeyCode::Char('k') if picker.selected >= picker.locked => {
                let face = &mut picker.faces[picker.selected];
                *face = *face % 6 + 1;
            }
            KeyCode::Down | KeyCode::Char('j') if picker.selected >= picker.locked => {
                let face = &mut picker.faces[picker.selected];
                *face = if *face <= 1 { 6 } else { *face - 1 };
            }
            KeyCode::Enter => {
                if picker.faces.contains(&0) {
                    picker.selected = picker.faces.iter().position(|&v| v == 0).unwrap();
                    self.message = "Choose all five faces before confirming.".into();
                    return;
                }
                let faces = picker
                    .faces
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                let command = match self.session.position.phase {
                    Phase::Decision { rerolls_left, .. } => format!("dice {rerolls_left} {faces}"),
                    _ => format!("roll {faces}"),
                };
                if let Err(e) = self.command(&command) {
                    self.message = e.to_string();
                } else {
                    self.dice_picker = None;
                    self.message =
                        "Dice confirmed. A: analyze | W: score selected category | U: undo".into();
                }
            }
            _ => (),
        }
    }
    pub fn poll(&mut self) {
        while let Ok((rev, result)) = self.rx.try_recv() {
            if rev != self.revision {
                continue;
            }
            self.running = None;
            match result {
                Ok(report) => {
                    self.message = report.stop_reason.clone();
                    self.report = Some(report);
                }
                Err(e) => self.message = e.to_string(),
            }
        }
    }
    fn analyze(&mut self) -> Result<()> {
        self.session.validate()?;
        self.cancel.cancel();
        self.cancel = Cancel::default();
        self.revision += 1;
        self.report = None;
        let session = self.session.clone();
        let cancel = self.cancel.clone();
        let tx = self.tx.clone();
        let revision = self.revision;
        self.running = Some(Instant::now());
        std::thread::spawn(move || {
            let r = solver::analyze(&session.position, &session.settings, cancel);
            let _ = tx.try_send((revision, r));
        });
        Ok(())
    }
    pub fn command(&mut self, command: &str) -> Result<()> {
        let words: Vec<_> = command.split_whitespace().collect();
        let arg = |i: usize| {
            words
                .get(i)
                .copied()
                .ok_or_else(|| Error::Invalid("Missing command argument (press ? for help)".into()))
        };
        let number = |i: usize| -> Result<usize> {
            arg(i)?
                .parse()
                .map_err(|_| Error::Invalid("Expected a nonnegative integer".into()))
        };
        let index = |i: usize, max: usize| -> Result<usize> {
            let n = number(i)?;
            if n == 0 || n > max {
                Err(Error::Invalid(format!("Expected a number from 1 to {max}")))
            } else {
                Ok(n - 1)
            }
        };
        let tail = || {
            command
                .split_once(char::is_whitespace)
                .map(|(_, s)| s.trim())
                .unwrap_or("")
        };
        let Some(&verb) = words.first() else {
            return Ok(());
        };
        match verb {
            "quit" | "q" => {
                self.quit = true;
                return Ok(());
            }
            "help" | "rules" => {
                self.help = !self.help;
                self.scroll = 0;
                return Ok(());
            }
            "analyze" => return self.analyze(),
            "budget" => {
                self.budget_open = !self.budget_open;
                return Ok(());
            }
            "save" => {
                let path = if words.len() > 1 {
                    PathBuf::from(tail())
                } else {
                    self.save_path.clone()
                };
                self.session.save(&path)?;
                self.save_path = path;
                self.message = format!("Saved {}", self.save_path.display());
                return Ok(());
            }
            "undo" => {
                if let Some(old) = self.undo.pop() {
                    self.redo.push(self.session.clone());
                    self.session = old;
                    self.invalidate();
                }
                return Ok(());
            }
            "redo" => {
                if let Some(next) = self.redo.pop() {
                    self.undo.push(self.session.clone());
                    self.session = next;
                    self.invalidate();
                }
                return Ok(());
            }
            _ => (),
        }
        // Work on a copy so failed commands do not partially mutate a position.
        let previous = self.session.clone();
        let mut next = previous.clone();
        let p = &mut next.position;
        match verb {
            "new" => {
                let n = number(1)?;
                if !(1..=8).contains(&n) {
                    return Err(Error::Invalid("Need 1–8 players".into()));
                }
                next = Session::new(n);
            }
            "load" => next = Session::load(&PathBuf::from(tail()))?,
            "turn" => p.active_player = index(1, p.players.len())?,
            "perspective" => next.settings.perspective = index(1, p.players.len())?,
            "order" => {
                let order: Vec<usize> = (1..words.len())
                    .map(|i| index(i, p.players.len()))
                    .collect::<Result<_>>()?;
                let mut sorted = order.clone();
                sorted.sort_unstable();
                if sorted != (0..p.players.len()).collect::<Vec<_>>() {
                    return Err(Error::Invalid(
                        "Order must list every player exactly once".into(),
                    ));
                }
                p.turn_order = order;
            }
            "name" => {
                let i = index(1, p.players.len())?;
                arg(2)?;
                p.players[i].name = words[2..].join(" ");
            }
            "extra" => {
                let i = index(1, p.players.len())?;
                p.players[i].sheet.extra_kniffels = u8::try_from(number(2)?)
                    .map_err(|_| Error::Invalid("Extra count is too large".into()))?;
                p.players[i].sheet.validate()?;
            }
            "before" => p.phase = Phase::BeforeFirstRoll,
            "finished" => {
                if p.open() != 0 {
                    return Err(Error::Invalid("Some categories remain open".into()));
                }
                p.phase = Phase::Finished;
            }
            "next" => p.advance(),
            "hypothetical" => {
                p.hypothetical = match arg(1)? {
                    "on" => true,
                    "off" => false,
                    _ => return Err(Error::Invalid("Use hypothetical on or off".into())),
                }
            }
            "roll" | "dice" => {
                let start = if verb == "dice" { 2 } else { 1 };
                let faces: Vec<u8> = words[start.min(words.len())..]
                    .iter()
                    .map(|v| {
                        v.parse()
                            .map_err(|_| Error::Invalid("Dice must be 1–6".into()))
                    })
                    .collect::<Result<_>>()?;
                let dice = Dice::from_faces(&faces)?;
                if dice.len() != 5 {
                    return Err(Error::Invalid("Enter exactly five dice".into()));
                }
                if verb == "roll" {
                    p.enter_roll(dice)?;
                } else {
                    let r = number(1)?;
                    if r > 2 {
                        return Err(Error::Invalid("Rerolls left must be 0, 1, or 2".into()));
                    }
                    p.phase = Phase::Decision {
                        dice,
                        rerolls_left: r as u8,
                    };
                }
            }
            "write" => {
                let category = index(1, 13)?;
                let Phase::Decision { dice, .. } = p.phase else {
                    return Err(Error::Invalid("Enter the current dice first".into()));
                };
                let a = if rules::score(&p.players[p.active_player].sheet, dice, category) == 0 {
                    Action::CrossOut { category }
                } else {
                    Action::Write { category }
                };
                p.apply(a)?;
            }
            "record" => {
                let c = index(1, 13)?;
                if p.players[p.active_player].sheet.cells[c].is_some() {
                    return Err(Error::Invalid(
                        "Category is already filled; edit it in the grid for a correction".into(),
                    ));
                }
                let points = u16::try_from(number(2)?)
                    .map_err(|_| Error::Invalid("Score too large".into()))?;
                p.players[p.active_player].sheet.cells[c] = Some(points);
                p.players[p.active_player].sheet.validate()?;
                p.advance();
            }
            "apply" => {
                let report = self
                    .report
                    .as_ref()
                    .ok_or_else(|| Error::Invalid("Analyze this position first".into()))?;
                let action = if words.len() == 1 {
                    report.best_action
                } else {
                    report
                        .alternatives
                        .get(index(1, report.alternatives.len())?)
                        .filter(|a| a.metrics.is_some())
                        .map(|a| a.action)
                };
                p.apply(action.ok_or_else(|| {
                    Error::Invalid("No assessed recommendation to apply".into())
                })?)?;
            }
            "objective" => {
                next.settings.objective = match arg(1)? {
                    "shared" => Objective::SharedFirst,
                    "strict" => Objective::StrictFirst,
                    "score" => Objective::ExpectedScore,
                    _ => return Err(Error::Invalid("Use shared, strict, or score".into())),
                }
            }
            "model" => {
                let model = match arg(1)? {
                    "opportunity" => Policy::Opportunity,
                    "greedy" => Policy::Greedy,
                    "legacy" | "legacy_opportunity" => Policy::LegacyOpportunity,
                    _ => return Err(Error::Invalid("Use opportunity, legacy, or greedy".into())),
                };
                if words.len() > 2 {
                    let i = index(2, p.players.len())?;
                    if next.settings.opponent_policies.is_empty() {
                        next.settings.opponent_policies =
                            vec![next.settings.policy; p.players.len()];
                    }
                    next.settings.opponent_policies[i] = model;
                } else {
                    next.settings.policy = model;
                    next.settings.opponent_policies.clear();
                }
            }
            "method" => {
                next.settings.method = match arg(1)? {
                    "auto" => Method::Auto,
                    "exact" => Method::Exact,
                    "rollout" => Method::Rollout,
                    "mcts" => Method::Mcts,
                    _ => return Err(Error::Invalid("Use auto, exact, rollout, or mcts".into())),
                }
            }
            "time" => next.settings.time_ms = number(1)? as u64,
            "depth" => next.settings.max_depth = number(1)?,
            "nodes" => {
                let n = number(1)?;
                if n == 0 {
                    return Err(Error::Invalid("Node budget must be positive".into()));
                }
                next.settings.max_nodes = n;
            }
            "samples" => {
                let n = number(1)?;
                if n == 0 {
                    return Err(Error::Invalid("Samples must be positive".into()));
                }
                next.settings.simulations = n as u64;
            }
            "seed" => next.settings.seed = number(1)? as u64,
            "cache" => next.settings.cache = Some(tail().into()),
            "baseline-model" => {
                next.settings.baseline_model = if tail() == "bundled" {
                    None
                } else {
                    Some(tail().into())
                }
            }
            "advice" => {
                next.settings.conservative_advice = match arg(1)? {
                    "sampled" => false,
                    "conservative" => true,
                    _ => {
                        return Err(Error::Invalid(
                            "Use advice sampled or advice conservative".into(),
                        ));
                    }
                }
            }
            _ => return Err(Error::Invalid("Unknown command; press ? for help".into())),
        }
        self.session = next;
        self.changed(previous);
        self.message = "Position updated. A analyzes; ? shows help.".into();
        Ok(())
    }
    pub fn key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(event::KeyModifiers::CONTROL) {
            self.cancel.cancel();
            self.quit = true;
            return;
        }
        if let Some(input) = &mut self.input {
            match key.code {
                KeyCode::Esc => {
                    self.input = None;
                    self.editing_cell = false;
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => input.push(c),
                KeyCode::Enter => {
                    let text = self.input.take().unwrap();
                    let result = if self.editing_cell {
                        self.editing_cell = false;
                        let value = if text.trim() == "-" || text.trim().is_empty() {
                            Ok(None)
                        } else {
                            text.trim().parse::<u16>().map(Some).map_err(|_| {
                                Error::Invalid("Use - for open or a nonnegative score".into())
                            })
                        };
                        value.and_then(|v| {
                            let previous = self.session.clone();
                            let mut sheet = self.session.position.players[self.column].sheet;
                            sheet.cells[self.row] = v;
                            sheet.validate()?;
                            self.session.position.players[self.column].sheet = sheet;
                            self.changed(previous);
                            Ok(())
                        })
                    } else {
                        self.command(&text)
                    };
                    if let Err(e) = result {
                        self.message = e.to_string();
                    }
                }
                _ => (),
            }
            return;
        }
        if self.dice_picker.is_some() {
            self.picker_key(key.code);
            return;
        }
        if let Some(category) = self.score_confirmation {
            match key.code {
                KeyCode::Enter => {
                    self.score_confirmation = None;
                    if let Err(e) = self.command(&format!("write {}", category + 1)) {
                        self.message = e.to_string();
                    }
                }
                KeyCode::Esc => self.score_confirmation = None,
                _ => (),
            }
            return;
        }
        if let Some(selected) = self.action_menu {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.action_menu = Some((selected + ACTIONS.len() - 1) % ACTIONS.len())
                }
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                    self.action_menu = Some((selected + 1) % ACTIONS.len())
                }
                KeyCode::Enter => {
                    self.action_menu = None;
                    self.perform_action(selected);
                }
                KeyCode::Esc | KeyCode::Char('m') => self.action_menu = None,
                _ => (),
            }
            return;
        }
        if self.help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => self.help = false,
                KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char('d') => {
                    self.scroll = self.scroll.saturating_add(5)
                }
                KeyCode::PageUp | KeyCode::Char('u') => self.scroll = self.scroll.saturating_sub(5),
                KeyCode::Char('g') => self.scroll = 0,
                _ => (),
            }
            return;
        }
        if self.budget_open {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.budget_field = (self.budget_field + 3) % 4,
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                    self.budget_field = (self.budget_field + 1) % 4
                }
                KeyCode::Enter => {
                    self.input = Some(format!(
                        "{} ",
                        ["time", "samples", "nodes", "depth"][self.budget_field]
                    ))
                }
                KeyCode::Esc | KeyCode::Char('b' | 'B') => self.budget_open = false,
                KeyCode::Char('q' | 'Q') => self.quit = true,
                _ => (),
            }
            return;
        }
        if key.modifiers.contains(event::KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('r') => {
                    self.perform_action(7);
                    return;
                }
                KeyCode::Char('d') => {
                    self.scroll = self.scroll.saturating_add(5);
                    return;
                }
                KeyCode::Char('u') => {
                    self.scroll = self.scroll.saturating_sub(5);
                    return;
                }
                _ => (),
            }
        }
        let command = match key.code {
            KeyCode::Char('m' | 'M') => {
                self.action_menu = Some(0);
                None
            }
            KeyCode::Char('w' | 'W') => {
                self.perform_action(3);
                None
            }
            KeyCode::Char('g') => {
                self.row = 0;
                None
            }
            KeyCode::Char('G') => {
                self.row = 12;
                None
            }
            KeyCode::Char('b' | 'B') => Some("budget"),
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(5);
                None
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(5);
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.row = (self.row + 12) % 13;
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.row = (self.row + 1) % 13;
                None
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.column = self.column.saturating_sub(1);
                None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.column = (self.column + 1).min(self.session.position.players.len() - 1);
                None
            }
            KeyCode::Enter => {
                self.input = Some(String::new());
                self.editing_cell = true;
                None
            }
            KeyCode::Char(':') => {
                self.input = Some(String::new());
                None
            }
            KeyCode::Char('d' | 'D') => {
                self.open_dice_picker();
                None
            }
            KeyCode::Char('a' | 'A') => Some("analyze"),
            KeyCode::Char('x' | 'X') => {
                self.perform_action(2);
                None
            }
            KeyCode::Char('u' | 'U') => Some("undo"),
            KeyCode::Char('r' | 'R') => Some("redo"),
            KeyCode::Char('s' | 'S') => Some("save"),
            KeyCode::Char('?') => Some("help"),
            KeyCode::Char('q' | 'Q') => Some("quit"),
            KeyCode::Esc => {
                self.cancel.cancel();
                self.running = None;
                self.revision += 1;
                self.help = false;
                self.message = "Analysis cancelled".into();
                None
            }
            _ => None,
        };
        if let Some(c) = command
            && let Err(e) = self.command(c)
        {
            self.message = e.to_string();
        }
    }
    pub fn draw(&self, f: &mut Frame) {
        let p = &self.session.position;
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(3),
            ])
            .split(f.area());
        let phase = match p.phase {
            Phase::BeforeFirstRoll => "Enter first roll (D)".into(),
            Phase::Decision { dice, rerolls_left } => {
                format!("Dice: {dice} | rerolls left: {rerolls_left}")
            }
            Phase::AwaitingRoll {
                kept,
                rerolls_after,
            } => format!("Keep {kept}; enter new full roll (D) | then {rerolls_after} rerolls"),
            Phase::Finished => "Game finished".into(),
        };
        let header = format!(
            "Kniffel | active: {} | advice for: {} | {:?}{}\n{phase}",
            p.players[p.active_player].name,
            p.players[self.session.settings.perspective].name,
            self.session.settings.objective,
            if p.hypothetical {
                " | hypothetical"
            } else {
                ""
            }
        );
        f.render_widget(
            Paragraph::new(header).block(Block::default().borders(Borders::BOTTOM)),
            areas[0],
        );
        if let Some(category) = self.score_confirmation {
            let points = match p.phase {
                Phase::Decision { dice, .. } => {
                    rules::score(&p.players[p.active_player].sheet, dice, category)
                }
                _ => 0,
            };
            let extra = matches!(p.phase, Phase::Decision { dice, .. } if rules::extra_eligible(&p.players[p.active_player].sheet, dice));
            f.render_widget(Paragraph::new(format!(
                "Confirm score for {}\n{}: {} points{}\nThis ends the turn.\nEnter: score and advance | Esc: go back\nU can undo after confirmation.",
                p.players[p.active_player].name, rules::NAMES[category], points,
                if extra { " (+50 extra Kniffel)" } else { "" }
            )).wrap(Wrap { trim: false }), areas[1]);
        } else if let Some(picker) = &self.dice_picker {
            let dice = picker
                .faces
                .iter()
                .enumerate()
                .map(|(i, face)| {
                    let value = if *face == 0 {
                        "?".into()
                    } else {
                        face.to_string()
                    };
                    Span::styled(
                        format!(
                            "{}[{}]{} ",
                            if i == picker.selected { ">" } else { " " },
                            value,
                            if i < picker.locked { "K" } else { " " }
                        ),
                        if i == picker.selected {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    )
                })
                .collect::<Vec<_>>();
            f.render_widget(
                Paragraph::new(vec![
                    Line::from("Pick actual dice (K = kept, locked)"),
                    Line::from(dice),
                    Line::from("h/l or Left/Right: choose die"),
                    Line::from("j/k or Down/Up: change face"),
                    Line::from("1-6: set face and move to next die"),
                    Line::from("Enter: confirm all | Esc: discard"),
                    Line::from("Only enter dice you actually rolled."),
                ])
                .wrap(Wrap { trim: false }),
                areas[1],
            );
        } else if let Some(selected) = self.action_menu {
            let mut lines = vec![Line::from(
                "Actions | j/k or arrows, Enter selects, Esc closes",
            )];
            for (i, label) in ACTIONS.iter().enumerate() {
                lines.push(Line::from(format!(
                    "{} {}",
                    if selected == i { ">" } else { " " },
                    label
                )));
            }
            // Keep the selected action visible even on a short terminal.
            let offset = (selected + 2).saturating_sub(areas[1].height as usize) as u16;
            f.render_widget(Paragraph::new(lines).scroll((offset, 0)), areas[1]);
        } else if self.budget_open {
            let settings = &self.session.settings;
            let unlimited = |n: u64| {
                if n == 0 {
                    "unlimited".to_string()
                } else {
                    n.to_string()
                }
            };
            let labels = [
                format!("Time limit (ms): {}", unlimited(settings.time_ms)),
                format!("Total simulations: {}", settings.simulations),
                format!("Maximum search nodes: {}", settings.max_nodes),
                format!(
                    "MCTS depth (your decisions): {}",
                    unlimited(settings.max_depth as u64)
                ),
            ];
            let mut lines = vec![
                Line::from("Computation budget"),
                Line::from("j/k or Up/Down: select | Enter: edit | B/Esc: close"),
                Line::from(""),
            ];
            for (i, label) in labels.into_iter().enumerate() {
                let selected = i == self.budget_field;
                lines.push(Line::from(Span::styled(
                    format!("{} {label}", if selected { ">" } else { " " }),
                    if selected {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                )));
            }
            lines.extend([
                Line::from(""),
                Line::from("Time and depth: 0 = unlimited. Samples and nodes must be positive."),
                Line::from("Time/samples stop analysis; nodes/depth limit tree expansion."),
                Line::from("Depth 1 searches only your next decision; later play uses the baseline."),
                Line::from("Dice and opponents do not count toward depth. Simulations still finish the game."),
                Line::from("Depth applies to MCTS. Exact search uses time/nodes; rollout uses time/samples."),
                Line::from("Changes are saved with the session and invalidate old advice."),
            ]);
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), areas[1]);
        } else if self.help {
            f.render_widget(
                Paragraph::new(format!(
                    "B: computation budget/depth | :nodes 50000 | :depth 3 (0 = unlimited)\n:advice conservative (default) | :advice sampled\n:baseline-model path.json | :baseline-model bundled\n{HELP}"
                ))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0)),
                areas[1],
            );
        } else {
            let middle = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(areas[1].height.saturating_sub(5).min(19)),
                    Constraint::Min(5),
                ])
                .split(areas[1]);
            let visible = ((middle[0].width.saturating_sub(20) / 14) as usize)
                .max(1)
                .min(p.players.len());
            let first = (self.column + 1).saturating_sub(visible);
            let end = (first + visible).min(p.players.len());
            let mut header = vec![Cell::from("Category")];
            for i in first..end {
                header.push(Cell::from(format!(
                    "{} {}{}",
                    i + 1,
                    p.players[i].name,
                    if i == p.active_player { " *" } else { "" }
                )));
            }
            let mut rows = Vec::new();
            let row_capacity = middle[0].height.saturating_sub(2) as usize;
            let first_row = (self.row + 1).saturating_sub(row_capacity.max(1));
            for row in first_row..(first_row + row_capacity).min(17) {
                let label = if row < 13 {
                    rules::NAMES[row]
                } else {
                    ["Upper subtotal", "Upper bonus", "Extra Kniffels", "TOTAL"][row - 13]
                };
                let mut cells = vec![Cell::from(label)];
                for i in first..end {
                    let sheet = &p.players[i].sheet;
                    let text = match row {
                        0..=12 => sheet.cells[row]
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "-".into()),
                        13 => sheet.upper().to_string(),
                        14 => sheet.bonus().to_string(),
                        15 => format!(
                            "{} (+{})",
                            sheet.extra_kniffels,
                            50 * sheet.extra_kniffels as u16
                        ),
                        _ => sheet.total().to_string(),
                    };
                    let style = if row == self.row && i == self.column {
                        Style::default()
                            .bg(Color::Blue)
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let text = if row == self.row && i == self.column {
                        format!("> {text}")
                    } else {
                        text
                    };
                    cells.push(Cell::from(text).style(style));
                }
                rows.push(Row::new(cells));
            }
            let mut widths = vec![Constraint::Length(19)];
            widths.extend(std::iter::repeat_n(Constraint::Min(12), end - first));
            f.render_widget(
                Table::new(rows, widths)
                    .header(Row::new(header).style(Style::default().fg(Color::Cyan)))
                    .block(Block::default().borders(Borders::BOTTOM)),
                middle[0],
            );
            let advice = if let Some(t) = self.running {
                format!(
                    "Analyzing… {:.1}s (Esc cancels)\n{}; {} ms budget",
                    t.elapsed().as_secs_f64(),
                    self.session.settings.policy.name(),
                    self.session.settings.time_ms
                )
            } else if let Some(report) = &self.report {
                report.text()
            } else {
                "D: pick dice -> A: analyze -> X: apply advice\nM: action menu | W: score selected row\nhjkl/arrows: navigate | Enter: edit sheet | B: budget | ?: help".into()
            };
            f.render_widget(
                Paragraph::new(advice)
                    .wrap(Wrap { trim: false })
                    .scroll((self.scroll, 0)),
                middle[1],
            );
        }
        let status = if let Some(input) = &self.input {
            format!(
                "{}{}",
                if self.editing_cell {
                    "Score (- = open): "
                } else {
                    ":"
                },
                input
            )
        } else {
            self.message.clone()
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(status, Style::default().fg(Color::Yellow))),
                Line::from(format!(
                    "{} | total {} (upper {}, bonus {}, extras {}) | M: actions | ?: help",
                    p.players[self.column].name,
                    p.players[self.column].sheet.total(),
                    p.players[self.column].sheet.upper(),
                    p.players[self.column].sheet.bonus(),
                    p.players[self.column].sheet.extra_kniffels
                )),
            ])
            .block(Block::default().borders(Borders::TOP)),
            areas[2],
        );
    }
}
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}
pub fn run(session: Session, path: Option<PathBuf>) -> Result<()> {
    session.validate()?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(Error::Invalid(
            "The TUI needs an interactive terminal. Use `knfsolv analyze <file>` for scripts."
                .into(),
        ));
    }
    enable_raw_mode()?;
    let _guard = Guard;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut app = App::new(session, path);
    while !app.quit {
        app.poll();
        terminal.draw(|f| app.draw(f))?;
        if event::poll(Duration::from_millis(33))?
            && let Event::Key(k) = event::read()?
        {
            app.key(k);
        }
    }
    app.cancel.cancel();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_worker_results_are_discarded_after_an_edit() {
        let mut session = Session::new(1);
        session.position.players[0].sheet.cells = [Some(0); 13];
        session.position.phase = Phase::Finished;
        let report =
            solver::analyze(&session.position, &session.settings, Cancel::default()).unwrap();
        let mut app = App::new(session, None);
        app.tx.send((app.revision, Ok(report))).unwrap();
        app.command("new 2").unwrap();
        app.poll();
        assert!(app.report.is_none());
    }
}
