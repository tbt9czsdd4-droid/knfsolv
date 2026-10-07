//! Native desktop presentation. Domain decisions stay in the shared engine.
use crate::{
    Error, Result,
    coaching::{AnalysisKey, AnalysisRequest, AnalysisUpdate, Computation, Wake},
    game::{
        AnalysisMode, AutomaticStep, ComputerGameConfig, ComputerPolicy, DesktopGame, DiceSource,
        GameController, JobPurpose, LocalGameConfig, PlayerControl,
    },
    persistence::Session,
    policy::Policy,
    preferences::ScoreWheelLimits,
    rules::{self, Action},
    solver::{Alternative, BaselineComparison, Method, Metrics, Objective, Report, Settings},
    state::Phase,
};
use eframe::egui;
use i18n::{t, tf};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use web_time::Instant;

pub mod i18n;
mod phone;
mod picker;
mod players;
mod start;

enum Pending {
    Local(LocalGameConfig),
    Computer(ComputerGameConfig),
    Analyze(usize),
    /// The analyzer with these players, in playing order, and empty sheets.
    AnalyzeNamed(Vec<String>),
    /// A new position with the current players' names and order, and empty sheets.
    Fresh,
    Load(PathBuf),
    /// A position file the browser opened, already parsed and validated.
    Open(Box<Session>),
    Close,
}

/// Browser integration. There the current position is saved automatically, so positions are
/// only shared and opened as files, never saved to a path.
pub trait Browser {
    /// Offers the position file to the user (share sheet or download). Called while the tap
    /// or click is being handled, which browsers require.
    fn share(&self, json: &str) -> Result<()>;
    /// Shows the file picker; the chosen file arrives later through `take_opened`.
    fn open(&self);
    /// The text of a picked file, or why it could not be read.
    fn take_opened(&self) -> Option<std::result::Result<String, String>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Home,
    LocalSetup,
    ComputerSetup,
    AnalysisSetup,
    Play,
}

pub struct Desktop {
    game: GameController,
    observed_revision: u64,
    analysis_serial: u64,
    saved: String,
    path: String,
    current_path: PathBuf,
    message: String,
    /// The sheet cell (player, category) whose score picker is open.
    edit: Option<(usize, usize)>,
    /// The open player sheet in the position editor, with its unapplied changes.
    player_sheet: Option<players::PlayerDraft>,
    /// Names typed on the phone's „Wer spielt mit?“ screen.
    setup_names: Vec<String>,
    score_wheel: ScoreWheel,
    score_wheel_limits: ScoreWheelLimits,
    text_size: crate::preferences::TextSize,
    /// The zoom set from `text_size`; other zoom changes (Ctrl +/−) stay until it changes.
    applied_zoom: Option<f32>,
    show_settings: bool,
    engine_tab: bool,
    preferences: Option<crate::preferences::Preferences>,
    preference_store: Option<crate::preferences::Store>,
    pending: Option<Pending>,
    allow_close: bool,
    screen: Screen,
    session_active: bool,
    local_setup: LocalGameConfig,
    computer_setup: ComputerGameConfig,
    custom_search: bool,
    custom_settings: Settings,
    computer_requested: Option<AnalysisKey>,
    computer_fallback: bool,
    computer_status: String,
    computer_notice: String,
    analysis_players: usize,
    review_results: bool,
    report: Option<Report>,
    show_all_moves: bool,
    worker: Box<dyn Computation>,
    browser: Option<Box<dyn Browser>>,
    repaint: Arc<Mutex<Option<egui::Context>>>,
    requested: Option<AnalysisKey>,
    analysis_mode: AnalysisMode,
    analysis_paused: bool,
    analysis_done: bool,
    /// „Länger rechnen“: a finished limited analysis continues for the current position.
    extended: bool,
    running: Option<Instant>,
}

#[derive(Default)]
struct ScoreWheel {
    cell: Option<(usize, usize)>,
    remainder: f32,
    last_event: f64,
    suppress_tail: bool,
}
impl ScoreWheel {
    fn begin_frame(&mut self, ctx: &egui::Context) {
        ctx.input_mut(|input| {
            let fresh = input.events.iter().any(|e| {
                matches!(e,
                egui::Event::MouseWheel { delta, .. } if delta.y != 0.)
            });
            if fresh {
                self.suppress_tail = false;
            } else if self.suppress_tail {
                input.smooth_scroll_delta.y = 0.;
            }
        });
    }
    fn steps(&mut self, ctx: &egui::Context, cell: Option<(usize, usize)>) -> Vec<i32> {
        let now = ctx.input(|i| i.time);
        if self.cell != cell || now - self.last_event > 0.4 {
            self.remainder = 0.;
        }
        self.cell = cell;
        if cell.is_none() {
            return vec![];
        }
        let mut steps = vec![];
        ctx.input_mut(|input| {
            for event in &mut input.events {
                if let egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } = event
                {
                    if !modifiers.is_none() || delta.y == 0. {
                        continue;
                    }
                    let y = delta.y;
                    self.last_event = now;
                    self.suppress_tail = true;
                    if self.remainder.signum() != y.signum() {
                        self.remainder = 0.;
                    }
                    match unit {
                        egui::MouseWheelUnit::Point => {
                            self.remainder += y;
                            while self.remainder.abs() >= 40. {
                                let direction = self.remainder.signum();
                                steps.push(direction as i32);
                                self.remainder -= direction * 40.;
                            }
                        }
                        _ => {
                            self.remainder = 0.;
                            steps.push(y.signum() as i32);
                        }
                    }
                    delta.y = 0.;
                    input.smooth_scroll_delta.y = 0.;
                }
            }
        });
        steps
    }
}

fn wheel_score(
    category: usize,
    value: Option<u16>,
    direction: i32,
    limits: ScoreWheelLimits,
) -> Option<u16> {
    let scores = rules::category_scores(category);
    let index = value.map_or(0, |v| {
        scores
            .iter()
            .position(|&s| s == v)
            .expect("validated score")
            + 1
    });
    let next = index as i32 + direction;
    let next = match limits {
        ScoreWheelLimits::StopAtEnds => next.clamp(0, scores.len() as i32),
        ScoreWheelLimits::WrapAround => next.rem_euclid(scores.len() as i32 + 1),
    } as usize;
    next.checked_sub(1).map(|i| scores[i])
}

fn surface(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(8)
        .inner_margin(10)
}
fn pip_positions(face: u8) -> &'static [(f32, f32)] {
    match face {
        1 => &[(0., 0.)],
        2 => &[(-1., -1.), (1., 1.)],
        3 => &[(-1., -1.), (0., 0.), (1., 1.)],
        4 => &[(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)],
        5 => &[(-1., -1.), (1., -1.), (0., 0.), (-1., 1.), (1., 1.)],
        6 => &[
            (-1., -1.),
            (1., -1.),
            (-1., 0.),
            (1., 0.),
            (-1., 1.),
            (1., 1.),
        ],
        _ => &[],
    }
}
fn paint_die(ui: &egui::Ui, rect: egui::Rect, face: u8, color: egui::Color32) {
    if face == 0 {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "—",
            egui::FontId::proportional(rect.width() * 0.4),
            color,
        );
    } else {
        for &(x, y) in pip_positions(face) {
            ui.painter().circle_filled(
                rect.center() + egui::vec2(x, y) * rect.width() * 0.25,
                rect.width() * 0.075,
                color,
            );
        }
    }
}
fn die_button(
    ui: &mut egui::Ui,
    face: u8,
    size: f32,
    selected: bool,
    correction: bool,
    label: String,
) -> egui::Response {
    let mut button = egui::Button::new("")
        .selected(selected)
        .corner_radius(7)
        .fill(if selected {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().extreme_bg_color
        })
        .stroke(if selected {
            ui.visuals().selection.stroke
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        });
    if correction {
        button = button.stroke(egui::Stroke::new(2.0_f32, ui.visuals().text_color()));
    }
    let response = ui.add_sized([size, size], button);
    if response.has_focus() || response.hovered() {
        ui.painter().rect_stroke(
            response.rect.shrink(2.),
            5.,
            egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    paint_die(
        ui,
        response.rect,
        face,
        ui.style().interact(&response).fg_stroke.color,
    );
    response
}
fn small_die(ui: &mut egui::Ui, face: u8) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(24., 24.), egui::Sense::hover());
    ui.painter().rect(
        rect,
        4.,
        ui.visuals().extreme_bg_color,
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    paint_die(ui, rect, face, ui.visuals().text_color());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            ui.is_enabled(),
            format!("Retained die {face}"),
        )
    });
    response.on_hover_text(tf("Keep {face}", &[("face", &face)]));
}
/// "Reroll 3 dice", with the singular for one die.
fn reroll_label(count: usize) -> String {
    if count == 1 {
        t("Reroll 1 die").into()
    } else {
        tf("Reroll {count} dice", &[("count", &count)])
    }
}
fn primary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(label).strong())
            .fill(ui.visuals().selection.bg_fill)
            .stroke(ui.visuals().selection.stroke)
            .corner_radius(7)
            .min_size(egui::vec2(160., 44.)),
    )
}

/// Margins that make a single-line text field 44 points tall for touch.
fn touch_field_margin(ui: &egui::Ui) -> egui::Vec2 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let row = ui.fonts_mut(|f| f.row_height(&font));
    egui::vec2(8.0, ((44.0 - row) / 2.0).max(4.0))
}
/// The difficulty name of a computer preset.
fn policy_label(policy: ComputerPolicy) -> &'static str {
    match policy {
        ComputerPolicy::Greedy => t("Easy"),
        ComputerPolicy::LegacyOpportunity => t("Standard"),
        ComputerPolicy::ContinuationV2 => t("Strong"),
    }
}
/// Light, dark, or system appearance; egui's own buttons are English only.
fn theme_buttons(ui: &mut egui::Ui) {
    let mut theme = ui.ctx().options(|o| o.theme_preference);
    let before = theme;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut theme, egui::ThemePreference::System, t("💻 System"));
        ui.selectable_value(&mut theme, egui::ThemePreference::Light, t("☀ Light"));
        ui.selectable_value(&mut theme, egui::ThemePreference::Dark, t("🌙 Dark"));
    });
    if theme != before {
        ui.ctx().set_theme(theme);
    }
}
/// A report status in words; English shows the engine's identifier with spaces.
fn status_label(status: &str) -> String {
    match status {
        "incomplete" => t("incomplete").into(),
        "estimated" => t("estimated").into(),
        "exact_under_model" => t("exact under model").into(),
        "policy_decision" => t("policy decision").into(),
        other => other.replace('_', " "),
    }
}
/// A report confidence in words, like [`status_label`].
fn confidence_label(confidence: &str) -> String {
    match confidence {
        "insufficient" => t("insufficient").into(),
        "low" => t("low").into(),
        "separated_under_model" => t("separated under model").into(),
        "exact_under_model" => t("exact under model").into(),
        "not_applicable" => t("not applicable").into(),
        other => other.replace('_', " "),
    }
}
fn objective_label(objective: Objective) -> &'static str {
    match objective {
        Objective::SharedFirst => "Shared-first utility (ties split credit)",
        Objective::StrictFirst => "Outright-first probability",
        Objective::ExpectedScore => "Expected final game score (points)",
    }
}
fn comparison_ui(ui: &mut egui::Ui, label: &str, objective: &str, c: &BaselineComparison) {
    ui.label(format!(
        "{label} · {objective}: {:+.4}; paired 95% interval [{:+.4}, {:+.4}], {} pairs",
        c.mean_gain, c.interval[0], c.interval[1], c.samples
    ));
}
fn statistic_values(metrics: Option<Metrics>) -> [String; 4] {
    metrics
        .map(|m| {
            [
                i18n::percent(m.outright_win, 1),
                i18n::percent(m.tied_first, 1),
                i18n::percent((1. - m.outright_win - m.tied_first).clamp(0., 1.), 1),
                i18n::decimal(m.expected_score, 1),
            ]
        })
        .unwrap_or_else(|| std::array::from_fn(|_| t("Estimating…").into()))
}
fn move_statistics(ui: &mut egui::Ui, metrics: Option<Metrics>) {
    let values = statistic_values(metrics);
    let outcomes = t("Win: outright first. Tie: tied for first. Lose: all remaining outcomes.");
    let labels = [
        (t("Win"), outcomes),
        (t("Tie"), outcomes),
        (t("Lose"), outcomes),
        (
            t("Expected final game score"),
            t("Expected total at the end of the game, including future turns and bonuses."),
        ),
    ];
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        for (i, ((label, help), value)) in labels.into_iter().zip(values).enumerate() {
            ui.vertical(|ui| {
                if i < 3 {
                    ui.set_width(76.0);
                }
                ui.small(label).on_hover_text(help);
                ui.label(value);
            });
        }
    });
}

fn snapshot(session: &Session) -> String {
    serde_json::to_string(&crate::persistence::desktop_value(session))
        .expect("session has serializable integer and enum fields")
}

impl Desktop {
    /// Analyses run on a native worker thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(session: Session, path: Option<PathBuf>) -> Self {
        Self::with_computation(session, path, |wake| {
            Box::new(crate::coaching::ComputationWorker::new(wake))
        })
    }
    /// `computation` receives the wake-up that repaints the interface when reports arrive.
    pub fn with_computation(
        mut session: Session,
        path: Option<PathBuf>,
        computation: impl FnOnce(Wake) -> Box<dyn Computation>,
    ) -> Self {
        if path.is_none() && session.desktop.is_none() {
            session.desktop = Some(DesktopGame::new(
                session.position.players.len(),
                DiceSource::Physical,
                rand::random(),
            ));
        }
        let repaint: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
        let wake = repaint.clone();
        let worker = computation(Box::new(move || {
            if let Some(ctx) = &*wake.lock().unwrap() {
                ctx.request_repaint();
            }
        }));
        let advisor = session
            .desktop
            .as_ref()
            .map(|d| d.advisor.clone())
            .unwrap_or_default();
        let saved = if path.is_some() {
            snapshot(&session)
        } else {
            String::new()
        };
        let current_path = path.clone().unwrap_or_else(|| "position.json".into());
        let game = GameController::new(session).expect("validated desktop session");
        Self {
            game,
            observed_revision: 0,
            analysis_serial: 0,
            saved,
            path: current_path.display().to_string(),
            current_path,
            message: String::new(),
            edit: None,
            player_sheet: None,
            setup_names: Vec::new(),
            score_wheel: ScoreWheel::default(),
            score_wheel_limits: ScoreWheelLimits::default(),
            text_size: Default::default(),
            applied_zoom: None,
            show_settings: false,
            engine_tab: true,
            preferences: None,
            preference_store: None,
            pending: None,
            allow_close: false,
            screen: Screen::Play,
            session_active: true,
            local_setup: LocalGameConfig::default(),
            computer_setup: ComputerGameConfig::default(),
            custom_search: false,
            custom_settings: Settings {
                method: Method::Mcts,
                ..Default::default()
            },
            computer_requested: None,
            computer_fallback: false,
            computer_status: String::new(),
            computer_notice: String::new(),
            analysis_players: 2,
            review_results: false,
            report: None,
            show_all_moves: false,
            worker,
            browser: None,
            repaint,
            requested: None,
            analysis_mode: advisor.mode,
            analysis_paused: advisor.paused,
            analysis_done: false,
            extended: false,
            running: None,
        }
    }
    /// Startup without a loaded file offers setup; explicit loads open immediately.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn startup(session: Session, path: Option<PathBuf>) -> Self {
        let preferences = crate::persistence::FileStorage(crate::preferences::path());
        Self::startup_with(session, path, preferences, |wake| {
            Box::new(crate::coaching::ComputationWorker::new(wake))
        })
    }
    /// Startup with the platform's preference storage and analysis computation.
    pub fn startup_with(
        session: Session,
        path: Option<PathBuf>,
        preferences: impl crate::persistence::Storage + 'static,
        computation: impl FnOnce(Wake) -> Box<dyn Computation>,
    ) -> Self {
        let show_home = path.is_none();
        let players = session.position.players.len();
        let mut app = Self::with_computation(session, path, computation);
        app.analysis_players = players;
        app.local_setup.names = (0..players.clamp(2, 8)).map(start::player_number).collect();
        if show_home {
            app.screen = Screen::Home;
            app.session_active = false;
            app.message.clear();
        }
        let defaults = crate::preferences::Preferences {
            analysis_players: players,
            local: app.local_setup.clone(),
            ..Default::default()
        };
        let (store, preferences) =
            crate::preferences::Store::load_with_defaults(preferences, defaults);
        app.install_preferences(preferences);
        app.preference_store = Some(store);
        app
    }
    /// Switches to browser behavior: automatic saving, share and open instead of paths.
    pub fn set_browser(&mut self, browser: Box<dyn Browser>) {
        self.browser = Some(browser);
    }
    /// The position to keep across restarts, in the format of desktop saves; `None` before a
    /// session starts. Partial dice entry and undo history are not part of it.
    pub fn autosave(&self) -> Option<String> {
        self.session_active.then(|| snapshot(self.game.session()))
    }
    fn install_preferences(&mut self, p: crate::preferences::Preferences) {
        self.local_setup = p.local.clone();
        self.computer_setup = p.computer.clone();
        self.localize_default_names();
        self.custom_search = p.custom_search;
        self.custom_settings = p.custom_settings.clone();
        self.analysis_players = p.analysis_players;
        self.score_wheel_limits = p.score_wheel_limits;
        self.text_size = p.text_size;
        self.analysis_mode = p.analysis_mode;
        self.analysis_paused = p.analysis_paused;
        self.game.set_global_advice(&p.engine);
        self.game
            .set_advisor_controls(p.analysis_mode, p.analysis_paused);
        self.preferences = Some(p);
    }
    fn engine_changed(&mut self, engine: Settings) {
        if let Some(p) = &mut self.preferences {
            p.engine = engine.clone();
            p.engine.perspective = 0;
        }
        self.game.set_global_advice(&engine);
        self.worker.clear();
        self.analysis_serial += 1;
        self.requested = None;
        self.computer_requested = None;
        self.report = None;
        self.analysis_done = false;
        self.running = None;
    }
    fn persist_preferences(&mut self, ctx: &egui::Context) {
        if let Some(p) = &mut self.preferences {
            p.theme = match ctx.options(|o| o.theme_preference) {
                egui::ThemePreference::Dark => crate::preferences::Theme::Dark,
                egui::ThemePreference::Light => crate::preferences::Theme::Light,
                egui::ThemePreference::System => crate::preferences::Theme::System,
            };
            p.local = self.local_setup.clone();
            p.computer = self.computer_setup.clone();
            p.custom_search = self.custom_search;
            p.custom_settings = self.custom_settings.clone();
            p.analysis_players = self.analysis_players;
            p.score_wheel_limits = self.score_wheel_limits;
            p.text_size = self.text_size;
            p.analysis_mode = self.analysis_mode;
            p.analysis_paused = self.analysis_paused;
            if let Some(store) = &mut self.preference_store {
                let now = Instant::now();
                store.observe(p, now);
                store.tick(now);
                if let Some(due) = store.due() {
                    ctx.request_repaint_after(due.saturating_duration_since(now));
                }
            }
        }
    }
    /// Default names saved in English ("Player 2", "You") follow the interface language.
    fn localize_default_names(&mut self) {
        for (i, name) in self.local_setup.names.iter_mut().enumerate() {
            if *name == format!("Player {}", i + 1) {
                *name = start::player_number(i);
            }
        }
        if self.computer_setup.human_name == "You" {
            self.computer_setup.human_name = t("You").into();
        }
    }
    fn navigate(&mut self, screen: Screen) {
        if screen != Screen::Play && self.game.control() == PlayerControl::Computer {
            self.game.invalidate_jobs();
            self.refresh_revision();
        }
        // The phone setups start from the names saved last time unless names were typed already.
        if self.setup_names.iter().all(|name| name.trim().is_empty()) {
            match screen {
                Screen::LocalSetup => {
                    self.setup_names = (self.local_setup.names.iter().enumerate())
                        .map(|(i, name)| {
                            if *name == start::player_number(i) {
                                String::new()
                            } else {
                                name.clone()
                            }
                        })
                        .collect();
                }
                Screen::ComputerSetup if self.computer_setup.human_name != t("You") => {
                    self.setup_names = vec![self.computer_setup.human_name.clone()];
                }
                _ => {}
            }
        }
        self.screen = screen;
        self.show_settings = false;
        self.edit = None;
        self.player_sheet = None;
        if let Some(key) = self.requested {
            let paused = self.analysis_paused || screen != Screen::Play;
            self.worker
                .controls(key, self.analysis_mode == AnalysisMode::Continuous, paused);
            self.running = if paused || self.analysis_done {
                None
            } else {
                Some(Instant::now())
            };
        }
    }
    /// Whether replacing the session would lose scores entered on a sheet.
    fn has_entries(&self) -> bool {
        self.session_active
            && self
                .game
                .session()
                .position
                .players
                .iter()
                .any(|p| p.sheet.open() < rules::NAMES.len())
    }
    fn dirty(&self) -> bool {
        self.session_active
            && (snapshot(self.game.session()) != self.saved || self.game.has_draft())
    }
    fn dice_draft(&self) -> bool {
        self.game.decision_pending()
    }
    fn refresh_revision(&mut self) {
        if self.observed_revision != self.game.revision() {
            self.worker.clear();
            self.extended = false;
            self.requested = None;
            self.computer_requested = None;
            self.computer_fallback = false;
            self.computer_status.clear();
            self.analysis_done = false;
            if self.preferences.is_none()
                && let Some(desktop) = &self.game.session().desktop
            {
                self.analysis_mode = desktop.advisor.mode;
                self.analysis_paused = desktop.advisor.paused;
            }
            self.running = None;
            self.report = None;
            self.edit = None;
            self.player_sheet = None;
            self.review_results = false;
            if let Some(p) = &self.preferences {
                self.game.set_global_advice(&p.engine);
                self.game
                    .set_advisor_controls(self.analysis_mode, self.analysis_paused);
            }
            self.observed_revision = self.game.revision();
        }
    }
    fn commit(&mut self, next: Session) {
        let result = self.game.edit_session(next);
        self.outcome(result);
        self.refresh_revision();
    }
    fn undo(&mut self, redo: bool) {
        if redo {
            self.game.redo();
        } else {
            self.game.undo();
        }
        self.game
            .set_advisor_controls(self.analysis_mode, self.analysis_paused);
        self.refresh_revision();
    }
    fn apply(&mut self, action: Action) -> Result<()> {
        if self.game.position_editor() {
            self.game.apply_advice(action, Instant::now())?;
        } else {
            self.game.play(action, Instant::now())?;
        }
        self.refresh_revision();
        self.message = "Move applied.".into();
        Ok(())
    }
    fn choose_face(&mut self, face: u8, now: Instant) {
        let result = self.game.choose_face(face, now);
        self.outcome(result);
        self.refresh_revision();
    }
    fn clear_die(&mut self, now: Instant) {
        let result = self.game.clear_die(now);
        self.outcome(result);
        self.refresh_revision();
    }
    fn flush_correction(&mut self, now: Instant) {
        let result = self.game.flush_correction(now);
        self.outcome(result);
        self.refresh_revision();
    }
    fn prepare_save(&mut self) -> Result<()> {
        let result = self.game.prepare_save();
        self.refresh_revision();
        result
    }
    fn save(&mut self) -> Result<()> {
        self.prepare_save()?;
        crate::persistence::save_desktop(self.game.session(), &PathBuf::from(&self.path))?;
        self.current_path = PathBuf::from(&self.path);
        self.saved = snapshot(self.game.session());
        self.message = format!("Saved {}", self.path);
        Ok(())
    }
    fn save_before_pending(&mut self) -> Result<()> {
        self.prepare_save()?;
        // Loading another path must never save the current game over that target.
        crate::persistence::save_desktop(self.game.session(), &self.current_path)?;
        self.saved = snapshot(self.game.session());
        Ok(())
    }
    /// Shares the committed position, completing a full dice entry first.
    fn share(&mut self) -> Result<()> {
        self.prepare_save()?;
        let json =
            crate::persistence::to_json(&crate::persistence::desktop_value(self.game.session()))?;
        if let Some(browser) = &self.browser {
            browser.share(&json)?;
        }
        Ok(())
    }
    fn poll_opened(&mut self) {
        let Some(opened) = self.browser.as_ref().and_then(|b| b.take_opened()) else {
            return;
        };
        match opened
            .map_err(Error::Invalid)
            .and_then(|json| Session::from_json(&json))
        {
            Ok(session) => self.request(Pending::Open(Box::new(session))),
            Err(e) => {
                self.message = tf(
                    "This file is not a saved position. {error}",
                    &[("error", &e)],
                )
            }
        }
    }
    fn request(&mut self, pending: Pending) {
        // Browsers save automatically; they only confirm before entered scores are replaced.
        let confirm = if self.browser.is_some() {
            !matches!(pending, Pending::Close) && self.has_entries()
        } else {
            self.dirty()
        };
        if confirm {
            self.pending = Some(pending);
        } else {
            self.execute(pending);
        }
    }
    fn execute(&mut self, pending: Pending) {
        match pending {
            Pending::Close => self.allow_close = true,
            Pending::Local(config) => {
                if let Err(e) = self.game.start_local(&config, rand::random()) {
                    self.message = e.to_string();
                    return;
                }
                self.refresh_revision();
                self.started_session(t(
                    "New local game. Choose which dice to keep; click a score to end each turn.",
                ));
            }
            Pending::Computer(config) => {
                let result = config
                    .session(rand::random())
                    .and_then(|session| self.game.replace_session(session));
                if let Err(e) = result {
                    self.message = e.to_string();
                    return;
                }
                self.refresh_revision();
                self.started_session(t("New computer game. Human rolls wait for your input; computer dice are generated automatically."));
            }
            Pending::Fresh => {
                let current = &self.game.session().position;
                let mut session = Session::new(current.players.len());
                for (player, old) in session.position.players.iter_mut().zip(&current.players) {
                    player.name = old.name.clone();
                }
                session.position.turn_order = current.turn_order.clone();
                session.position.active_player = current.turn_order[0];
                session.settings.perspective = current.turn_order[0];
                if let Err(e) = self.game.replace_session(session) {
                    self.message = e.to_string();
                    return;
                }
                self.refresh_revision();
                self.started_session("");
            }
            Pending::Analyze(n) => self.start_editor(Session::new(n)),
            Pending::AnalyzeNamed(names) => {
                let mut session = Session::new(names.len());
                for (player, name) in session.position.players.iter_mut().zip(names) {
                    player.name = name;
                }
                self.start_editor(session);
            }
            Pending::Load(path) => match self.game.load(&path) {
                Ok(()) => {
                    self.path = path.display().to_string();
                    self.current_path = path;
                    self.opened(format!("Loaded {}", self.path));
                }
                Err(e) => self.message = e.to_string(),
            },
            Pending::Open(session) => match self.game.replace_session(*session) {
                Ok(()) => self.opened(t("Opened the position.").into()),
                Err(e) => self.message = e.to_string(),
            },
        }
    }
    fn start_editor(&mut self, session: Session) {
        if let Err(e) = self.game.replace_session(session) {
            self.message = e.to_string();
            return;
        }
        self.refresh_revision();
        if self.preferences.is_none() {
            self.analysis_mode = AnalysisMode::Continuous;
            self.analysis_paused = false;
        }
        self.started_session(t("Edit scores to forecast the next player. Dice are optional; enter five for move advice."));
    }
    fn opened(&mut self, message: String) {
        self.refresh_revision();
        self.saved = snapshot(self.game.session());
        self.message = message;
        self.computer_notice.clear();
        self.session_active = true;
        self.navigate(Screen::Play);
        if self.preferences.is_none() && self.game.session().desktop.is_none() {
            self.analysis_mode = AnalysisMode::Continuous;
            self.analysis_paused = false;
        }
    }
    fn started_session(&mut self, message: &str) {
        self.computer_notice.clear();
        self.session_active = true;
        self.saved.clear();
        self.path = "position.json".into();
        self.current_path = PathBuf::from(&self.path);
        self.message = message.into();
        self.navigate(Screen::Play);
    }
    fn before_roll_forecast(&self) -> bool {
        self.game.position_editor() && self.game.session().position.phase == Phase::BeforeFirstRoll
    }
    fn analysis_eligible(&self) -> bool {
        self.screen == Screen::Play
            && !self.dice_draft()
            && ((self.before_roll_forecast() && self.game.dice().faces == [0; 5])
                || (!self.game.dice().faces.contains(&0)
                    && matches!(self.game.session().position.phase, Phase::Decision { .. })))
            && (self.game.control() == PlayerControl::Human || self.game.editing())
            && self.game.session().validate().is_ok()
    }
    fn analyze(&mut self, _ctx: &egui::Context) -> Result<()> {
        if !self.analysis_eligible() {
            return Err(Error::Invalid(
                "Correct the position and complete or discard partial dice entry before analysis."
                    .into(),
            ));
        }
        self.analysis_serial += 1;
        self.report = None;
        self.analysis_done = false;
        self.running = Some(Instant::now());
        let mut settings = if self.game.editing() {
            self.game.session().settings.clone()
        } else {
            self.game.coaching_settings()
        };
        // Alone, every move "wins"; advice then maximizes the expected final score.
        if self.game.session().position.players.len() == 1 {
            settings.objective = Objective::ExpectedScore;
        }
        let key = self.analysis_key();
        self.worker.submit(AnalysisRequest {
            key,
            position: self.game.session().position.clone(),
            settings,
            continuous: self.analysis_mode == AnalysisMode::Continuous || self.extended,
            paused: self.analysis_paused,
            baseline_only: false,
        })?;
        self.requested = Some(key);
        Ok(())
    }
    fn analysis_key(&self) -> AnalysisKey {
        AnalysisKey {
            token: self.game.token(JobPurpose::Coaching),
            serial: self.analysis_serial,
        }
    }
    /// Continues the finished limited search, keeping its evidence, until the position changes.
    fn compute_longer(&mut self) {
        self.extended = true;
        self.analysis_done = false;
        if let Some(key) = self.requested {
            self.worker.controls(key, true, self.analysis_paused);
            self.running = (!self.analysis_paused).then(Instant::now);
        }
    }
    fn analysis_controls(&mut self, mode: AnalysisMode, paused: bool) {
        self.extended = false;
        self.analysis_mode = mode;
        self.analysis_paused = paused;
        self.game.set_advisor_controls(mode, paused);
        if let Some(key) = self.requested {
            self.worker
                .controls(key, mode == AnalysisMode::Continuous, paused);
            self.running = if paused { None } else { Some(Instant::now()) };
        }
    }
    fn ensure_analysis(&mut self, ctx: &egui::Context) {
        if self.analysis_eligible()
            && !self.analysis_paused
            && self.requested.is_none()
            && self.report.is_none()
        {
            let result = self.analyze(ctx);
            self.outcome(result);
        }
    }
    fn poll(&mut self) {
        self.refresh_revision();
        for update in self.worker.take_updates() {
            if update.key.token.purpose == JobPurpose::ComputerDecision {
                self.computer_update(update, Instant::now());
                continue;
            }
            if update.key != self.analysis_key() || update.key.token.purpose != JobPurpose::Coaching
            {
                continue;
            }
            self.analysis_done = update.finished;
            self.running = if update.finished || self.analysis_paused {
                None
            } else {
                self.running.or(Some(Instant::now()))
            };
            match update.report {
                Ok(report) => {
                    self.report = Some(report);
                }
                Err(error) => {
                    self.message = error;
                }
            }
        }
    }
    fn computer_enabled(&self) -> bool {
        self.screen == Screen::Play
            && !self.show_settings
            && self.pending.is_none()
            && self.edit.is_none()
            && !self.game.editing()
            && self.game.control() == PlayerControl::Computer
            && self
                .game
                .session()
                .desktop
                .as_ref()
                .is_some_and(|d| !d.computer.paused)
            && self.game.session().position.phase != Phase::Finished
    }
    fn request_computer(&mut self, fallback: bool) -> Result<()> {
        self.analysis_serial += 1;
        let key = AnalysisKey {
            token: self.game.token(JobPurpose::ComputerDecision),
            serial: self.analysis_serial,
        };
        let baseline_only = fallback
            || self
                .game
                .session()
                .desktop
                .as_ref()
                .is_none_or(|d| d.computer.search.is_none());
        let mut settings = self.game.computer_settings();
        if baseline_only {
            settings = Settings {
                perspective: settings.perspective,
                policy: settings.policy,
                opponent_policies: settings.opponent_policies,
                method: Method::Rollout,
                ..Default::default()
            };
        }
        self.worker.submit(AnalysisRequest {
            key,
            position: self.game.session().position.clone(),
            settings,
            continuous: false,
            paused: false,
            baseline_only,
        })?;
        self.computer_requested = Some(key);
        self.computer_fallback = fallback;
        self.computer_status = if fallback {
            "Choosing the declared baseline after a search failure…"
        } else {
            "Thinking…"
        }
        .into();
        Ok(())
    }
    fn computer_update(&mut self, update: AnalysisUpdate, now: Instant) {
        if self.computer_requested != Some(update.key)
            || !self.game.accepts(update.key.token)
            || update.key.token.purpose != JobPurpose::ComputerDecision
            || !self.computer_enabled()
            || update.paused
        {
            return;
        }
        if !update.finished {
            if let Ok(report) = update.report {
                self.computer_status = format!("Thinking… {} completed games", report.samples);
            }
            return;
        }
        self.computer_requested = None;
        let decision = update.report.and_then(|report| {
            report
                .best_action
                .map(|action| (action, report.selection_reason))
                .ok_or_else(|| "The search budget ended without advice.".into())
        });
        let reason = match decision {
            Ok((action, explanation)) => {
                match self
                    .game
                    .accept_computer_decision(update.key.token, action, now)
                {
                    Ok(true) => {
                        self.computer_status = format!("{action}. {explanation}");
                        return;
                    }
                    Ok(false) => return, // Cancellation/revision changes never create fallback moves.
                    Err(e) => e.to_string(),
                }
            }
            Err(e) => e,
        };
        if self.computer_fallback {
            self.computer_notice =
                format!("Computer paused: its legal baseline could not be obtained. {reason}");
            self.game.set_computer_paused(true);
            self.refresh_revision();
        } else {
            self.computer_notice =
                format!("Computer search fallback: {reason} Using the declared legal baseline.");
            let result = self.request_computer(true);
            self.outcome(result);
        }
    }
    fn tick_computer(&mut self, ctx: &egui::Context, now: Instant) {
        if !self.computer_enabled() || egui::Popup::is_any_open(ctx) {
            if self.computer_requested.is_some() || self.game.pending_computer_action().is_some() {
                self.game.invalidate_jobs();
                self.refresh_revision();
            }
            return;
        }
        if let Some(job) = self.game.automatic_job(now) {
            match job.step {
                AutomaticStep::Decide if self.computer_requested.is_none() => {
                    let result = self.request_computer(false);
                    self.outcome(result);
                }
                AutomaticStep::Roll | AutomaticStep::Apply(_) => {
                    let actor = self.game.session().position.players[job.token.actor]
                        .name
                        .clone();
                    match self.game.execute_automatic(job, now) {
                        Ok(true) => {
                            self.refresh_revision();
                            self.message = match job.step {
                                AutomaticStep::Roll => format!(
                                    "{actor} rolled {}.",
                                    self.game
                                        .dice()
                                        .faces
                                        .iter()
                                        .map(u8::to_string)
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                ),
                                AutomaticStep::Apply(action) => format!("{actor}: {action}."),
                                _ => unreachable!(),
                            };
                        }
                        Ok(false) => (),
                        Err(e) => {
                            self.message = e.to_string();
                            self.game.set_computer_paused(true);
                            self.refresh_revision();
                        }
                    }
                }
                _ => (),
            }
        }
        ctx.request_repaint_after(Duration::from_millis(25));
    }
    fn outcome(&mut self, result: Result<()>) {
        if let Err(e) = result {
            self.message = e.to_string();
        }
    }
    fn suggested_keeps(&self) -> [bool; 5] {
        let mut marked = [false; 5];
        if let Some(Action::Reroll { keep }) = self.report.as_ref().and_then(|r| r.best_action) {
            let mut remaining = keep.0;
            for (i, &face) in self.game.dice().faces.iter().enumerate() {
                if face > 0 && remaining[face as usize - 1] > 0 {
                    marked[i] = true;
                    remaining[face as usize - 1] -= 1;
                }
            }
        }
        marked
    }
    fn dice_ui(&mut self, ui: &mut egui::Ui) {
        surface(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let phase = self.game.session().position.phase;
            let physical = self.game.physical_entry_allowed();
            ui.horizontal_wrapped(|ui| {
                ui.heading(if self.game.position_editor() { t("Dice (optional)") } else { t("Dice") });
                if self.game.position_editor() && !self.game.session().position.hypothetical {
                    match self.game.session().position.infer_next_player() {
                        Ok(Some(player)) => { ui.strong(tf("Next: {name}", &[("name", &self.game.session().position.players[player].name)])); }
                        Ok(None) => { ui.strong(t("Game finished")); }
                        Err(_) => { ui.strong(t("Next player unresolved")); }
                    }
                } else {
                    ui.strong(tf("Turn: {name}", &[("name", &self.game.session().position.players[self.game.session().position.active_player].name)]));
                }
                if let Phase::Decision { rerolls_left, .. } = phase { ui.label(if rerolls_left == 1 { t("1 reroll left").into() } else { tf("{count} rerolls left", &[("count", &rerolls_left)]) }); }
            });
            ui.small(match phase {
                Phase::BeforeFirstRoll if self.game.position_editor() => t("Forecast averages over future rolls. Optionally enter five faces for move advice."),
                _ if self.game.control() == PlayerControl::Computer && !self.game.editing() => t("The computer rolls and plays automatically."),
                Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } if !physical => t("Ready when you are. Roll to continue."),
                Phase::BeforeFirstRoll => t("Enter five faces with the picker or keys 1–6."),
                Phase::AwaitingRoll { .. } => t("Enter new faces only. Retained dice are locked."),
                Phase::Decision { .. } if physical => t("Click dice to keep. Use the picker or 1–6 to correct the selected die."),
                Phase::Decision { .. } => t("Click dice to keep, then reroll or choose a score."),
                Phase::Finished => t("Game finished."),
            });
            ui.add_space(6.);
            let suggested = self.suggested_keeps();
            let tile = ((ui.available_width() - 24.) / 5.).clamp(44., 64.);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                for (i, hint) in suggested.into_iter().enumerate() {
                    ui.push_id(("game_die", i), |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(tile);
                            let face = self.game.dice().faces[i];
                            let locked = self.game.dice().locked[i];
                            let keep = self.game.dice().keep[i];
                            let state = if locked { "Locked" } else if keep { "Keep" } else if face == 0 { if physical { "Enter" } else { "Ready" } } else if matches!(phase, Phase::Decision { rerolls_left: 1..=2, .. }) { "Reroll" } else { "Rolled" };
                            let status = match state {
                                "Locked" => t("Locked"),
                                "Keep" => t("Kept"),
                                "Enter" => t("Enter"),
                                "Ready" => t("Ready"),
                                "Reroll" => t("Reroll"),
                                _ => t("Rolled"),
                            };
                            let response = ui.add_enabled_ui(!locked && phase != Phase::Finished && (self.game.editing() || self.game.control() == PlayerControl::Human), |ui| {
                                die_button(ui, face, tile, keep || locked, self.game.dice().selected == Some(i),
                                    format!("Die {}: {}; {state}{}", i + 1, if face == 0 { "empty".into() } else { face.to_string() }, if hint { "; Advice" } else { "" }))
                            }).inner;
                            if response.on_hover_text(t("Click to select for correction and toggle Keep. Advice is the engine recommendation only.")).clicked() {
                                self.game.select_die(i, true);
                            }
                            ui.small(status);
                            ui.small(if hint { t("★ Advice") } else { " " });
                        });
                    });
                }
            });
            ui.add_space(6.);
            ui.horizontal_wrapped(|ui| {
                ui.set_min_height(44.);
                if self.game.dice_source() == DiceSource::Physical || self.game.editing() {
                    ui.add_enabled_ui(physical, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.;
                            for face in 1..=6 {
                                let response = ui.push_id(("face_picker", face), |ui| {
                                    die_button(ui, face, 32., false, false, format!("Enter face {face}"))
                                }).inner;
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("picker_rect", face)), response.rect));
                                if response.on_hover_text(tf("Enter {face} · key {face}", &[("face", &face)])).clicked() { self.choose_face(face, Instant::now()); }
                            }
                            if ui.add_sized([44., 32.], egui::Button::new(t("Clear"))).on_hover_text(t("Backspace: clear the last editable die")).clicked() { self.clear_die(Instant::now()); }
                        });
                    });
                }
                let can_roll = self.game.human_can_play();
                match phase {
                    Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } if self.game.dice_source() == DiceSource::Digital => {
                        if primary_button(ui, t("Roll"), can_roll).clicked() {
                            let result = self.game.roll_digital(Instant::now()); self.outcome(result); self.refresh_revision();
                        }
                    }
                    Phase::Decision { rerolls_left, .. } => {
                        let count = self.game.dice().keep.iter().filter(|&&k| !k).count();
                        if primary_button(ui, &reroll_label(count), can_roll && rerolls_left > 0 && count > 0)
                            .on_hover_text(if physical { t("Retain selected dice, then enter the new physical faces.") } else { t("Generate the unkept dice now.") }).clicked() {
                            let result = self.game.reroll_selected(Instant::now()); self.outcome(result); self.refresh_revision();
                        }
                    }
                    _ => { ui.allocate_space(egui::vec2(160., 44.)); }
                }
            });
            if self.game.position_editor() && phase != Phase::Finished
                && (phase != Phase::BeforeFirstRoll || self.game.dice().faces != [0; 5])
                && ui.button(t("Analyze before first roll")).clicked() {
                let result = self.game.analyze_before_roll();
                self.outcome(result);
                self.refresh_revision();
            }
            if self.dice_draft() { ui.small(if self.game.dice().faces.contains(&0) { t("Complete the roll to continue.") } else { t("Updating roll…") }); }
        });
    }
    fn sheet_ui(&mut self, ui: &mut egui::Ui) {
        surface(ui).show(ui, |ui| self.sheet_contents(ui));
    }
    fn sheet_contents(&mut self, ui: &mut egui::Ui) {
        ui.set_width(ui.available_width());
        ui.heading(t("Score sheets"));
        ui.small(if self.game.position_editor() {
            t("Click a score to pick its value, or hover and scroll. Open = unfilled; 0 = crossed out. Filled entries determine the next player.")
        } else if self.game.editing() {
            t("Edit mode: click any entry. Empty = open; 0 = crossed out. Turns are paused.")
        } else {
            t("Click an available score to end the turn, including 0. Undo restores it. ★ = advice.")
        });
        let mut action = None;
        let mut wheel_cell = None;
        let ready = self.game.human_can_play();
        let suggested = self.report.as_ref().and_then(|r| r.best_action);
        egui::ScrollArea::horizontal().id_salt("sheets").show(ui, |ui| {
            egui::Grid::new("sheet").striped(true).min_col_width(85.0).min_row_height(26.0).show(ui, |ui| {
                ui.strong(t("Category"));
                for (i, p) in self.game.session().position.players.iter().enumerate() {
                    let active = i == self.game.session().position.active_player && self.game.session().position.phase != Phase::Finished
                        && (!self.game.position_editor() || self.game.session().position.hypothetical || self.game.session().position.infer_next_player().is_ok());
                    ui.label(egui::RichText::new(format!("{}{}", p.name, if active { t(" (turn)") } else { "" })).strong()
                        .background_color(if active { ui.visuals().selection.bg_fill } else { egui::Color32::TRANSPARENT }));
                }
                ui.end_row();
                for category in 0..13 {
                    ui.label(i18n::category_name(category));
                    for (player, p) in self.game.session().position.players.iter().enumerate() {
                        let value = p.sheet.cells[category];
                        if self.game.editing() {
                            let response = ui.push_id(("score_cell", player, category), |ui| {
                                ui.add_sized([76., 26.], egui::Button::new(value.map(|v| v.to_string()).unwrap_or_else(|| t("Open").into())))
                            }).inner;
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("score_cell_rect", player, category)), response.rect));
                            if self.game.position_editor() && response.enabled() && response.hovered() {
                                wheel_cell = Some((player, category));
                            }
                            let response = if self.game.position_editor() {
                                response.on_hover_text(t("Scroll up/down through legal scores, or click to pick one. Open = unfilled; 0 = crossed out. Wheel limits: Settings → Game."))
                            } else { response };
                            if response.clicked() {
                                self.edit = Some((player, category));
                            }
                        } else if let Some(value) = value {
                            ui.label(value.to_string());
                        } else if player == self.game.session().position.active_player
                            && let Phase::Decision { dice, .. } = self.game.session().position.phase
                        {
                            let points = rules::score(&p.sheet, dice, category);
                            let hint = matches!(suggested, Some(Action::Write { category: c } | Action::CrossOut { category: c }) if c == category);
                            let computer_choice = matches!(self.game.pending_computer_action(), Some(Action::Write { category: c } | Action::CrossOut { category: c }) if c == category);
                            let label = if computer_choice { format!("{points} ← Computer") } else if ready { format!("{points}{}", if hint { " ★" } else { "" }) } else if self.game.control() == PlayerControl::Computer { points.to_string() } else { "…".into() };
                            if ui.add_enabled(ready, egui::Button::new(label).min_size(egui::vec2(76.0, 26.0)))
                                .on_hover_text(if rules::extra_eligible(&p.sheet, dice) { t("Score and advance; also awards 50 extra Kniffel points.") } else { t("Score and advance to the next player.") }).clicked()
                            {
                                action = Some(if points == 0 { Action::CrossOut { category } } else { Action::Write { category } });
                            }
                        } else {
                            ui.label(t("Open"));
                        }
                    }
                    ui.end_row();
                }
                for row in 0..4 {
                    ui.strong([t("Upper subtotal"), t("Upper bonus"), t("Extra points"), t("TOTAL")][row]);
                    for p in &self.game.session().position.players {
                        ui.strong(match row {
                            0 => p.sheet.upper(), 1 => p.sheet.bonus(),
                            2 => p.sheet.extra_kniffels as u16 * 50, _ => p.sheet.total(),
                        }.to_string());
                    }
                    ui.end_row();
                }
            });
            if self.edit.is_some() { wheel_cell = None; }
            let steps = self.score_wheel.steps(ui.ctx(), wheel_cell);
            if let Some((player, category)) = wheel_cell {
                for direction in steps {
                    let mut next = self.game.session().clone();
                    let value = next.position.players[player].sheet.cells[category];
                    let updated = wheel_score(category, value, direction, self.score_wheel_limits);
                    if updated != value {
                        next.position.players[player].sheet.cells[category] = updated;
                        let result = self.game.edit_session(next);
                        let failed = result.is_err();
                        self.outcome(result);
                        self.refresh_revision();
                        if failed { break; }
                    }
                }
            }
        });
        if let Some(action) = action {
            let result = self.apply(action);
            self.outcome(result);
        }
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        {
            let mut next = self.game.session().clone();
            if !self.engine_tab {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Score wheel limits (global editor preference)");
                    egui::ComboBox::from_id_salt("score_wheel_limits")
                        .selected_text(match self.score_wheel_limits {
                            ScoreWheelLimits::StopAtEnds => "Stop at ends",
                            ScoreWheelLimits::WrapAround => "Wrap around",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.score_wheel_limits,
                                ScoreWheelLimits::StopAtEnds,
                                "Stop at ends",
                            );
                            ui.selectable_value(
                                &mut self.score_wheel_limits,
                                ScoreWheelLimits::WrapAround,
                                "Wrap around",
                            );
                        });
                });
                if let Some(desktop) = &mut next.desktop {
                    ui.horizontal(|ui| {
                        ui.label("Human dice");
                        ui.selectable_value(
                            &mut desktop.dice_source,
                            DiceSource::Physical,
                            "Physical",
                        );
                        ui.selectable_value(
                            &mut desktop.dice_source,
                            DiceSource::Digital,
                            "Digital",
                        );
                    });
                }
                ui.add_enabled_ui(self.game.editing(), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_enabled_ui(
                            !self.game.position_editor() || next.position.hypothetical,
                            |ui| {
                                ui.label("Active player");
                                egui::ComboBox::from_id_salt("active")
                                    .selected_text(
                                        &next.position.players[next.position.active_player].name,
                                    )
                                    .show_ui(ui, |ui| {
                                        for (i, p) in next.position.players.iter().enumerate() {
                                            ui.selectable_value(
                                                &mut next.position.active_player,
                                                i,
                                                &p.name,
                                            );
                                        }
                                    });
                                ui.label("Advice for");
                                egui::ComboBox::from_id_salt("perspective")
                                    .selected_text(
                                        &next.position.players[next.settings.perspective].name,
                                    )
                                    .show_ui(ui, |ui| {
                                        for (i, p) in next.position.players.iter().enumerate() {
                                            ui.selectable_value(
                                                &mut next.settings.perspective,
                                                i,
                                                &p.name,
                                            );
                                        }
                                    });
                            },
                        );
                        ui.checkbox(&mut next.position.hypothetical, "Hypothetical position");
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Phase");
                        if ui.button("Before first roll").clicked() {
                            next.position.phase = Phase::BeforeFirstRoll;
                        }
                        if ui.button("Finished").clicked() {
                            next.position.phase = Phase::Finished;
                        }
                        if let Phase::Decision { rerolls_left, .. } = &mut next.position.phase {
                            ui.label("Rerolls left");
                            ui.add(egui::DragValue::new(rerolls_left).range(0..=2));
                        }
                    });
                    for i in 0..next.position.players.len() {
                        ui.horizontal(|ui| {
                            ui.label(format!("Player {}", i + 1));
                            ui.add(
                                egui::TextEdit::singleline(&mut next.position.players[i].name)
                                    .desired_width(140.0),
                            );
                            ui.label("Extra Kniffels (count)");
                            ui.add(
                                egui::DragValue::new(
                                    &mut next.position.players[i].sheet.extra_kniffels,
                                )
                                .range(0..=255),
                            );
                        });
                    }
                    ui.label("Turn order (move players left):");
                    ui.horizontal_wrapped(|ui| {
                        for rank in 0..next.position.turn_order.len() {
                            let player = next.position.turn_order[rank];
                            if ui
                                .add_enabled(
                                    rank > 0,
                                    egui::Button::new(format!(
                                        "← {}",
                                        next.position.players[player].name
                                    )),
                                )
                                .clicked()
                            {
                                next.position.turn_order.swap(rank - 1, rank);
                            }
                        }
                    });
                });
                self.commit(next);
                return;
            }
            for i in 0..next.position.players.len() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Seat {} model default", i + 1));
                    let mut model = next.settings.model(i);
                    egui::ComboBox::from_id_salt(("model", i))
                        .selected_text(model.name())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut model, Policy::Opportunity, "Opportunity");
                            ui.selectable_value(&mut model, Policy::Greedy, "Greedy");
                            ui.selectable_value(
                                &mut model,
                                Policy::LegacyOpportunity,
                                "Legacy opportunity",
                            );
                        });
                    if model != next.settings.model(i) {
                        if next.settings.opponent_policies.is_empty() {
                            next.settings.opponent_policies =
                                vec![next.settings.policy; next.position.players.len()];
                        }
                        next.settings.opponent_policies[i] = model;
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Objective");
                egui::ComboBox::from_id_salt("objective")
                    .selected_text(format!("{:?}", next.settings.objective))
                    .show_ui(ui, |ui| {
                        for (o, label) in [
                            (Objective::SharedFirst, "Shared first (split ties)"),
                            (Objective::StrictFirst, "Outright first"),
                            (Objective::ExpectedScore, "Expected score"),
                        ] {
                            ui.selectable_value(&mut next.settings.objective, o, label);
                        }
                    });
                ui.label("Method");
                egui::ComboBox::from_id_salt("method")
                    .selected_text(format!("{:?}", next.settings.method))
                    .show_ui(ui, |ui| {
                        for m in [Method::Auto, Method::Exact, Method::Rollout, Method::Mcts] {
                            ui.selectable_value(&mut next.settings.method, m, format!("{m:?}"));
                        }
                    });
            });
            egui::Grid::new("budgets").show(ui, |ui| {
                ui.label("Time limit (ms; 0 unlimited)");
                ui.add(egui::DragValue::new(&mut next.settings.time_ms));
                ui.end_row();
                ui.label("Simulations");
                ui.add(egui::DragValue::new(&mut next.settings.simulations).range(1..=u64::MAX));
                ui.end_row();
                ui.label("Search nodes");
                ui.add(egui::DragValue::new(&mut next.settings.max_nodes).range(1..=usize::MAX));
                ui.end_row();
                ui.label("MCTS depth limit (0 unlimited)");
                ui.add(egui::DragValue::new(&mut next.settings.max_depth));
                ui.end_row();
                ui.label("Random seed");
                ui.add(egui::DragValue::new(&mut next.settings.seed));
                ui.end_row();
            });
            ui.checkbox(
                &mut next.settings.conservative_advice,
                "Conservative advice: retain baseline unless improvement is established",
            );
            ui.horizontal(|ui| {
                ui.label("Continuation model (empty = bundled)");
                let mut path = next
                    .settings
                    .baseline_model
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                if ui.text_edit_singleline(&mut path).changed() {
                    next.settings.baseline_model = if path.trim().is_empty() {
                        None
                    } else {
                        Some(path.into())
                    };
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Policy cache path (empty = default)");
                let mut path = next
                    .settings
                    .cache
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                if ui.text_edit_singleline(&mut path).changed() {
                    next.settings.cache = (!path.trim().is_empty()).then(|| path.into());
                }
            });
            ui.small("Depth counts your decisions, not dice or opponents. Simulations finish the whole game. Exact search uses time/nodes.");
            if next.settings != self.game.session().settings {
                self.engine_changed(next.settings);
            }
        }
    }
    fn advice_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.small_button(t("Engine settings")).clicked() { self.show_settings = true; self.engine_tab = true; }
            let label = match self.report.as_ref().and_then(|r| r.progress) {
                None => t("Preparing…").to_owned(),
                Some(p) => {
                    let depth = [("depth", &p.max_decision_depth as &dyn std::fmt::Display)];
                    match p.phase {
                        crate::solver::SearchPhase::MctsTraining => tf("Tree depth {depth}", &depth),
                        crate::solver::SearchPhase::MctsValidation => tf("Tree depth {depth} · validating", &depth),
                        crate::solver::SearchPhase::Exact => tf("Exact depth {depth}", &depth),
                        crate::solver::SearchPhase::Rollout => t("Full-game rollouts").to_owned(),
                    }
                }
            };
            ui.small(label).on_hover_text(t("Deepest explored branch, not a depth completed uniformly across every move. Counts the analyzed player's decisions (root = 1), excluding dice outcomes, opponents and baseline rollout continuation."));
        });
        if !self.computer_notice.is_empty() {
            ui.label(&self.computer_notice);
        }
        if self.game.control() == PlayerControl::Computer && !self.game.editing() {
            ui.heading(t("Computer turn"));
            let config = &self.game.session().desktop.as_ref().unwrap().computer;
            ui.label(if config.search.is_some() {
                t("Custom search · strength uncalibrated")
            } else {
                policy_label(config.policy)
            });
            if config.paused {
                ui.label(t("Computer paused."));
            } else if let Some(action) = self.game.pending_computer_action() {
                ui.strong(tf("Choice: {move}", &[("move", &self.move_title(action))]));
            } else if self.computer_requested.is_some() {
                ui.spinner();
            }
            if !self.computer_status.is_empty() {
                ui.label(&self.computer_status);
            }
            ui.small(t("Coaching resumes for the next human turn."));
            return;
        }
        let mut mode = self.analysis_mode;
        let mut paused = self.analysis_paused;
        ui.horizontal_wrapped(|ui| {
            ui.heading(
                if self.game.position_editor()
                    && !self.game.session().position.hypothetical
                    && self.game.session().position.infer_next_player().is_err()
                {
                    t("Analysis paused · inconsistent sheets").into()
                } else if self.before_roll_forecast() {
                    tf(
                        "Before-roll forecast · {name}",
                        &[(
                            "name",
                            &self.game.session().position.players
                                [self.game.session().settings.perspective]
                                .name,
                        )],
                    )
                } else {
                    t("Advice").into()
                },
            );
            ui.selectable_value(&mut mode, AnalysisMode::Continuous, t("Continuous"));
            ui.selectable_value(&mut mode, AnalysisMode::Limited, t("Limited"));
            if ui
                .button(if paused {
                    t("Resume analysis")
                } else {
                    t("Pause analysis")
                })
                .clicked()
            {
                paused = !paused;
            }
            if self.running.is_some() && !paused {
                ui.spinner();
            }
            if self.analysis_done
                && self.analysis_mode == AnalysisMode::Limited
                && !self
                    .report
                    .as_ref()
                    .is_some_and(|r| r.status == "exact_under_model")
                && ui.button(t("Compute longer")).clicked()
            {
                self.compute_longer();
            }
            if self.analysis_done && ui.button(t("Restart analysis")).clicked() {
                let result = self.analyze(ui.ctx());
                self.outcome(result);
            }
        });
        if mode != self.analysis_mode || paused != self.analysis_paused {
            self.analysis_controls(mode, paused);
        }
        if !self.analysis_eligible() {
            if self.game.position_editor()
                && !self.game.session().position.hypothetical
                && let Err(error) = self.game.session().position.infer_next_player()
            {
                ui.label(error.to_string());
                return;
            }
            ui.label(
                if matches!(self.game.session().position.phase, Phase::Finished) {
                    t("Game finished.")
                } else if self.game.position_editor() {
                    t("Analysis waits: correct the position, or complete five dice or choose Analyze before first roll.")
                } else {
                    t("Coaching waits for a complete, valid human roll.")
                },
            );
            return;
        }
        if let Some(report) = &self.report {
            let progress = if self.analysis_paused {
                t("Paused")
            } else if self.running.is_some() {
                t("Refining")
            } else {
                t("Stopped")
            };
            ui.small(format!(
                "{progress} · {}{} · {}",
                if report.recommendation_basis.starts_with("baseline_") {
                    t("Provisional baseline · ")
                } else {
                    ""
                },
                status_label(&report.status),
                tf(
                    "Confidence: {confidence}",
                    &[("confidence", &confidence_label(&report.confidence))]
                )
            ));
            if self.before_roll_forecast() {
                move_statistics(
                    ui,
                    report.forecast.or_else(|| {
                        report
                            .alternatives
                            .iter()
                            .find(|a| a.action == Action::RollAll)
                            .and_then(|a| a.metrics)
                    }),
                );
                ui.small(t("Averaged over possible future rolls and modeled play."));
                ui.small(tf(
                    "{count} games · {seconds} s computing",
                    &[
                        ("count", &i18n::count(report.samples)),
                        (
                            "seconds",
                            &i18n::decimal(report.elapsed_ms as f64 / 1000., 2),
                        ),
                    ],
                ));
                return;
            }
            let best = report.best_action;
            let mut play = None;
            if let Some(action) = best {
                if self.move_card(
                    ui,
                    action,
                    report.alternatives.iter().find(|a| a.action == action),
                    true,
                ) {
                    play = Some(action);
                }
            } else {
                ui.label(t("No recommendation yet."));
                if report.forecast.is_some() {
                    ui.strong(t("Game forecast"));
                    move_statistics(ui, report.forecast);
                }
            }
            let others: Vec<_> = report
                .alternatives
                .iter()
                .filter(|a| Some(a.action) != best)
                .collect();
            for alternative in
                others
                    .iter()
                    .take(if self.show_all_moves { others.len() } else { 3 })
            {
                if self.move_card(ui, alternative.action, Some(alternative), false) {
                    play = Some(alternative.action);
                }
            }
            if others.len() > 3
                && ui
                    .button(if self.show_all_moves {
                        t("Show fewer")
                    } else {
                        t("Show all moves")
                    })
                    .clicked()
            {
                self.show_all_moves = !self.show_all_moves;
            }
            // Engine terms stay English here, as on the phone's „Technische Details“.
            egui::CollapsingHeader::new(t("Analysis details"))
                .id_salt("analysis_details").default_open(false).show(ui, |ui| {
                    let s = &report.settings;
                    ui.label(format!("For {} · {}", self.game.session().position.players[s.perspective].name, report.method));
                    ui.label(format!("Selection: {}", report.recommendation_basis));
                    ui.label(&report.selection_reason);
                    if let Some(a) = report.baseline_action { ui.label(format!("Baseline: {a}")); }
                    if let Some(a) = report.sampled_best_action { ui.label(format!("Highest sampled estimate: {a}")); }
                    let objective = objective_label(s.objective);
                    ui.label(format!("Optimization objective: {objective}"));
                    ui.label(format!("Model: {} · per-player policies: {:?}", s.policy.name(), s.opponent_policies));
                    if self.game.has_computer() && !self.game.editing() {
                        ui.label(if self.game.session().desktop.as_ref().unwrap().computer.search.is_some() {
                            "Computer model: continuation-v2 baseline, an approximation to its custom search."
                        } else { "Computer model: its declared preset. Bundled continuation model is used during this game." });
                    }
                    ui.label(&report.note);
                    if let Some(c) = &report.baseline_comparison {
                        comparison_ui(ui, "Sampled leader minus baseline", objective, c);
                    }
                    ui.label(format!("{} completed games · {:.2}s computing · {} nodes",
                        report.samples, report.elapsed_ms as f64 / 1000., report.nodes));
                    ui.label(format!("Budgets: {} ms, {} games · depth {}, {} nodes · {:?}",
                        s.time_ms, s.simulations, s.max_depth, s.max_nodes, self.analysis_mode));
                    ui.label(format!("Stop reason: {}", report.stop_reason));
                    for a in &report.alternatives {
                        ui.separator();
                        ui.strong(self.move_title(a.action));
                        ui.label(format!("{} samples", a.samples));
                        if let Some(m) = a.metrics { ui.label(format!("{objective}: {:.4}", m.utility)); }
                        if let Some([low, high]) = a.interval {
                            ui.label(format!("Simultaneous 95% sampling interval · {objective}: [{low:.4}, {high:.4}]"));
                        }
                        if let Some(c) = &a.baseline_comparison { comparison_ui(ui, "Gain vs baseline", objective, c); }
                    }
                });
            if let Some(action) = play {
                let result = self.apply(action);
                self.outcome(result);
            }
        } else {
            ui.label(if self.analysis_paused {
                t("Paused · waiting for an estimate.")
            } else {
                t("Preparing provisional baseline advice…")
            });
        }
    }
    fn move_title(&self, action: Action) -> String {
        match action {
            Action::CrossOut { category } => tf(
                "Score {category} · {points} points now",
                &[("category", &i18n::category_name(category)), ("points", &0)],
            ),
            Action::Write { category } => {
                if let Phase::Decision { dice, .. } = self.game.session().position.phase {
                    let sheet = &self.game.session().position.players
                        [self.game.session().position.active_player]
                        .sheet;
                    let title = tf(
                        "Score {category} · {points} points now",
                        &[
                            ("category", &i18n::category_name(category)),
                            ("points", &rules::score(sheet, dice, category)),
                        ],
                    );
                    if rules::extra_eligible(sheet, dice) {
                        title + t(" (+50 extra Kniffel)")
                    } else {
                        title
                    }
                } else {
                    action.to_string()
                }
            }
            Action::Reroll { keep } => reroll_label(5 - keep.len()),
            Action::RollAll => t("Roll all five dice").into(),
        }
    }
    fn move_card(
        &self,
        ui: &mut egui::Ui,
        action: Action,
        estimate: Option<&Alternative>,
        recommended: bool,
    ) -> bool {
        let stroke = if recommended {
            egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color)
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };
        let response = ui.scope_builder(
            egui::UiBuilder::new().id(egui::Id::new(("move_card", action))),
            |ui| {
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .stroke(stroke)
                    .corner_radius(6)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(if recommended {
                                t("Recommended")
                            } else {
                                t("Alternative")
                            });
                            ui.label(self.move_title(action));
                        });
                        if let Action::Reroll { keep } = action {
                            ui.horizontal_wrapped(|ui| {
                                ui.small(if keep.is_empty() {
                                    t("Keep none")
                                } else {
                                    t("Keep")
                                });
                                for face in keep.faces() {
                                    small_die(ui, face);
                                }
                            });
                        }
                        move_statistics(ui, estimate.and_then(|a| a.metrics));
                        ui.push_id(("play_move", action), |ui| {
                            // In the position editor, advice is applied rather than played.
                            let (allowed, label) = if self.game.position_editor() {
                                let label = if recommended {
                                    t("Apply suggestion")
                                } else {
                                    t("Apply this move")
                                };
                                (self.game.can_apply_advice(), label)
                            } else {
                                let label = if recommended {
                                    t("Play suggestion")
                                } else {
                                    t("Play this move")
                                };
                                (self.game.human_can_play(), label)
                            };
                            let response = ui.add_enabled(
                                allowed && self.game.session().position.actions().contains(&action),
                                egui::Button::new(label).min_size(egui::vec2(120., 32.)),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    response.enabled(),
                                    format!("{label}: {}", self.move_title(action)),
                                )
                            });
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(
                                    egui::Id::new(("move_response", action)),
                                    (response.id, response.rect),
                                )
                            });
                            response.clicked()
                        })
                        .inner
                    })
            },
        );
        ui.add_space(6.0);
        response.inner.inner
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        self.picker_ui(ctx);
        self.player_sheet_ui(ctx);
        // Centered, narrower than the smallest phone, with 44-point buttons.
        let width = (ctx.content_rect().width() - 48.0).clamp(200.0, 320.0);
        if self.pending.is_some() && self.browser.is_some() {
            egui::Window::new(t("Replace position"))
                .id(egui::Id::new("replace_position"))
                .collapsible(false)
                .resizable(false)
                .min_width(width)
                .max_width(width)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(t("Discard the current position?"));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
                        let button =
                            |label| egui::Button::new(label).min_size(egui::vec2(half, 44.0));
                        if ui.add(button(t("Discard"))).clicked() {
                            let p = self.pending.take().unwrap();
                            self.execute(p);
                        }
                        if ui.add(button(t("Go back"))).clicked() {
                            self.pending = None;
                        }
                    });
                });
        } else if self.pending.is_some() {
            egui::Window::new("Unsaved changes")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Save the current session before continuing?");
                    ui.label(format!(
                        "Save current game to: {}",
                        self.current_path.display()
                    ));
                    if ui.button("Save and continue").clicked() {
                        match self.save_before_pending() {
                            Ok(()) => {
                                let p = self.pending.take().unwrap();
                                self.execute(p);
                            }
                            Err(e) => self.message = e.to_string(),
                        }
                    }
                    if ui.button("Discard and continue").clicked() {
                        let p = self.pending.take().unwrap();
                        self.execute(p);
                    }
                    if ui.button("Go back").clicked() {
                        self.pending = None;
                    }
                });
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.score_wheel.begin_frame(ui.ctx());
        if self.edit.is_some()
            || self.show_settings
            || self.pending.is_some()
            || !self.game.position_editor()
        {
            self.score_wheel.cell = None;
            self.score_wheel.remainder = 0.;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(ui.visuals().panel_fill.to_opaque()))
            .show_inside(ui, |ui| self.contents(ui));
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused()
            || self.screen != Screen::Play
            || self.show_settings
            || self.pending.is_some()
            || self.edit.is_some()
        {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if modifiers.command && key == egui::Key::Z {
                self.undo(modifiers.shift);
                continue;
            }
            if modifiers.command || modifiers.alt || modifiers.ctrl {
                continue;
            }
            let face = match key {
                egui::Key::Num1 => Some(1),
                egui::Key::Num2 => Some(2),
                egui::Key::Num3 => Some(3),
                egui::Key::Num4 => Some(4),
                egui::Key::Num5 => Some(5),
                egui::Key::Num6 => Some(6),
                _ => None,
            };
            if let Some(face) = face {
                if self.game.physical_entry_allowed() {
                    self.choose_face(face, Instant::now());
                }
            } else if key == egui::Key::Backspace && self.game.physical_entry_allowed() {
                self.clear_die(Instant::now());
            } else if matches!(
                key,
                egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::H | egui::Key::L
            ) && ctx.memory(|m| m.focused().is_none())
            {
                let step = if matches!(key, egui::Key::ArrowLeft | egui::Key::H) {
                    4
                } else {
                    1
                };
                let mut slot = self
                    .game
                    .dice()
                    .selected
                    .unwrap_or(if step == 1 { 4 } else { 0 });
                for _ in 0..5 {
                    slot = (slot + step) % 5;
                    if !self.game.dice().locked[slot] {
                        self.game.select_die(slot, false);
                        break;
                    }
                }
            } else if key == egui::Key::Space
                && ctx.memory(|m| m.focused().is_none())
                && let Some(slot) = self.game.dice().selected
                && !self.dice_draft()
                && matches!(
                    self.game.session().position.phase,
                    Phase::Decision {
                        rerolls_left: 1..=2,
                        ..
                    }
                )
            {
                self.game.toggle_keep(slot);
            }
        }
    }
    fn setup_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().id_salt("setup").show(ui, |ui| {
            ui.heading("Kniffel");
            ui.label(t("Standard rules · two rerolls per turn · 13 scoring categories"));
            ui.add_space(12.0);
            if self.screen == Screen::Home {
                ui.heading(t("Choose how to play"));
                if ui.add_sized([220.0, 40.0], egui::Button::new(t("Local game"))).clicked() {
                    self.navigate(Screen::LocalSetup);
                }
                ui.small(t("2–8 people sharing a score sheet, with physical or digital dice."));
                if ui.add_sized([220.0, 40.0], egui::Button::new(t("Play computer"))).clicked() {
                    self.navigate(Screen::ComputerSetup);
                }
                if ui.add_sized([220.0, 40.0], egui::Button::new(t("Analyze a position"))).clicked() {
                    self.navigate(Screen::AnalysisSetup);
                }
                ui.separator();
                if let Some(browser) = &self.browser {
                    if ui.button(t("Open position")).clicked() {
                        browser.open();
                    }
                } else {
                    ui.label("Session path");
                    ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(360.0));
                    if ui.button("Load session").clicked() {
                        self.request(Pending::Load(PathBuf::from(&self.path)));
                    }
                }
            } else if self.screen == Screen::LocalSetup {
                ui.heading(t("Local game"));
                let mut count = self.local_setup.names.len();
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("Players"));
                    for n in 2..=8 {
                        ui.selectable_value(&mut count, n, n.to_string());
                    }
                });
                for i in self.local_setup.names.len()..count {
                    self.local_setup.names.push(start::player_number(i));
                }
                self.local_setup.names.truncate(count);
                for (i, name) in self.local_setup.names.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(start::player_number(i));
                        ui.add(egui::TextEdit::singleline(name).id_salt(("setup_name", i))
                            .hint_text(start::player_number(i)).desired_width(220.0).char_limit(60));
                    });
                }
                self.local_setup.starting_player = self.local_setup.starting_player.min(count - 1);
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("Starting player"));
                    egui::ComboBox::from_id_salt("setup_starting_player")
                        .selected_text(format!("{}. {}", self.local_setup.starting_player + 1,
                            self.local_setup.names[self.local_setup.starting_player]))
                        .show_ui(ui, |ui| {
                            for (i, name) in self.local_setup.names.iter().enumerate() {
                                ui.selectable_value(&mut self.local_setup.starting_player, i, format!("{}. {name}", i + 1));
                            }
                        });
                });
                ui.small(t("Play follows the listed order, wrapping around from the starting player."));
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("Dice"));
                    ui.selectable_value(&mut self.local_setup.dice_source, DiceSource::Physical, t("Physical dice"));
                    ui.selectable_value(&mut self.local_setup.dice_source, DiceSource::Digital, t("Digital dice"));
                });
                ui.label(match self.local_setup.dice_source {
                    DiceSource::Physical => t("Roll your own dice, then enter their faces with the palette or keys 1–6."),
                    DiceSource::Digital => t("Click Roll when you are ready. The app generates the dice."),
                });
                let validation = self.local_setup.session(0);
                if let Err(e) = &validation {
                    ui.colored_label(ui.visuals().error_fg_color, e.to_string());
                }
                if ui.add_enabled(validation.is_ok(), egui::Button::new(t("Start local game")).min_size(egui::vec2(180.0, 36.0))).clicked() {
                    self.request(Pending::Local(self.local_setup.clone()));
                }
            } else if self.screen == Screen::ComputerSetup {
                self.computer_setup_ui(ui);
            } else {
                ui.heading(t("Analyze a position"));
                ui.label(t("Start with empty sheets, then edit any scores, enter dice, and set the active player."));
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("Players"));
                    for n in 1..=8 {
                        ui.selectable_value(&mut self.analysis_players, n, n.to_string());
                    }
                });
                if ui.button(t("Open position editor")).clicked() {
                    self.request(Pending::Analyze(self.analysis_players));
                }
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if self.screen != Screen::Home && ui.button(t("Back")).clicked() {
                    self.navigate(Screen::Home);
                }
                if self.session_active && ui.button(t("Return to session")).clicked() {
                    self.navigate(Screen::Play);
                }
            });
            ui.label(&self.message);
            theme_buttons(ui);
        });
    }
    fn computer_setup_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(t("Play computer"));
        ui.horizontal(|ui| {
            ui.label(t("Your name"));
            ui.add(
                egui::TextEdit::singleline(&mut self.computer_setup.human_name)
                    .desired_width(180.0)
                    .char_limit(60),
            );
        });
        ui.horizontal(|ui| {
            ui.label(t("Computer name"));
            ui.add(
                egui::TextEdit::singleline(&mut self.computer_setup.computer_name)
                    .desired_width(180.0)
                    .char_limit(60),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Starts"));
            ui.selectable_value(&mut self.computer_setup.human_starts, true, t("You first"));
            ui.selectable_value(
                &mut self.computer_setup.human_starts,
                false,
                t("Computer first"),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Your dice"));
            ui.selectable_value(
                &mut self.computer_setup.dice_source,
                DiceSource::Physical,
                t("Physical dice"),
            );
            ui.selectable_value(
                &mut self.computer_setup.dice_source,
                DiceSource::Digital,
                t("Digital dice"),
            );
        });
        ui.small(t(
            "Computer rolls are always generated by the app. Your digital rolls wait for Roll.",
        ));
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Difficulty"));
            for policy in [
                ComputerPolicy::Greedy,
                ComputerPolicy::LegacyOpportunity,
                ComputerPolicy::ContinuationV2,
            ] {
                if ui
                    .selectable_label(
                        !self.custom_search && self.computer_setup.computer.policy == policy,
                        policy_label(policy),
                    )
                    .clicked()
                {
                    self.custom_search = false;
                    self.computer_setup.computer.policy = policy;
                }
            }
            if ui
                .selectable_label(self.custom_search, t("Custom search"))
                .clicked()
            {
                self.custom_search = true;
            }
        });
        if self.custom_search {
            ui.label(t("Custom search · strength uncalibrated. More thinking does not guarantee stronger play."));
            ui.small(t("Continuation-v2 baseline with bounded MCTS and independent validation. Coaching approximates it with the baseline."));
            egui::Grid::new("computer_budgets").show(ui, |ui| {
                ui.label(t("Time (ms; 0 = unlimited)"));
                ui.add(egui::DragValue::new(&mut self.custom_settings.time_ms).range(0..=120_000));
                ui.end_row();
                ui.label(t("Simulations"));
                ui.add(
                    egui::DragValue::new(&mut self.custom_settings.simulations)
                        .range(1..=1_000_000_000),
                );
                ui.end_row();
                ui.label(t("Depth (0 = unlimited)"));
                ui.add(egui::DragValue::new(&mut self.custom_settings.max_depth).range(0..=100));
                ui.end_row();
                ui.label(t("Nodes"));
                ui.add(
                    egui::DragValue::new(&mut self.custom_settings.max_nodes).range(1..=1_000_000),
                );
                ui.end_row();
            });
            ui.checkbox(
                &mut self.custom_settings.conservative_advice,
                t("Conservative advice"),
            );
            self.computer_setup.computer.policy = ComputerPolicy::ContinuationV2;
            self.computer_setup.computer.search = Some(self.custom_settings.clone());
        } else {
            ui.label(tf(
                "Model: {name}",
                &[("name", &self.computer_setup.computer.policy.model().name())],
            ));
            ui.small(t(
                "Presets use different decision policies, evaluated in held-out games.",
            ));
            self.computer_setup.computer.search = None;
        }
        ui.checkbox(&mut self.computer_setup.computer.fast_play, t("Fast play"));
        let validation = self.computer_setup.session(0);
        if let Err(error) = &validation {
            ui.label(error.to_string());
        }
        if ui
            .add_enabled(
                validation.is_ok(),
                egui::Button::new(t("Start computer game")).min_size(egui::vec2(180.0, 36.0)),
            )
            .clicked()
        {
            self.request(Pending::Computer(self.computer_setup.clone()));
        }
    }
    fn results_ui(&mut self, ui: &mut egui::Ui) {
        let Some(results) = self.game.results() else {
            return;
        };
        egui::ScrollArea::vertical().id_salt("results").show(ui, |ui| {
            ui.heading(t("Game finished"));
            let winners: Vec<_> = results.iter().filter(|r| r.winner).map(|r| r.name.as_str()).collect();
            let names = winners.join(", ");
            ui.heading(if winners.len() == 1 {
                tf("Winner: {names}", &[("names", &names)])
            } else {
                tf("Tied winners: {names}", &[("names", &names)])
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("New game")).clicked() {
                    self.navigate(Screen::Home);
                }
                if ui.button(t("Review score sheets")).clicked() {
                    self.review_results = true;
                }
            });
            ui.label(t("Totals include the upper bonus and extra Kniffel points. Undo restores the last scoring decision."));
            egui::ScrollArea::horizontal().id_salt("result_columns").show(ui, |ui| {
                egui::Grid::new("final_totals").striped(true).show(ui, |ui| {
                    for label in [t("Player"), t("Upper subtotal"), t("Upper bonus"), t("Extra points"), t("TOTAL"), t("Result")] {
                        ui.strong(label);
                    }
                    ui.end_row();
                    for result in results {
                        ui.label(result.name);
                        ui.label(result.upper.to_string());
                        ui.label(result.bonus.to_string());
                        ui.label(result.extra_points.to_string());
                        ui.strong(result.total.to_string());
                        ui.label(if result.winner { t("Winner") } else { "" });
                        ui.end_row();
                    }
                });
            });
        });
    }
    fn toolbar_ui(&mut self, ui: &mut egui::Ui) {
        surface(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().interact_size.y = 28.;
            ui.horizontal_wrapped(|ui| {
                ui.heading("Kniffel");
                let mut editing = self.game.editing();
                egui::ComboBox::from_id_salt("play_mode")
                    .selected_text(if self.game.editing() {
                        t("Edit position")
                    } else if self.game.dice_source() == DiceSource::Digital {
                        t("Digital play")
                    } else {
                        t("Physical play")
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut editing, false, t("Play"));
                        ui.selectable_value(&mut editing, true, t("Edit position"));
                    });
                self.game.set_editing(editing);
                self.refresh_revision();
                ui.separator();
                if ui
                    .add_enabled(self.game.can_undo(), egui::Button::new(t("Undo")))
                    .on_hover_text(t("Ctrl/Cmd-Z · first resets unfinished dice corrections"))
                    .clicked()
                {
                    self.undo(false);
                }
                if ui
                    .add_enabled(self.game.can_redo(), egui::Button::new(t("Redo")))
                    .clicked()
                {
                    self.undo(true);
                }
                ui.separator();
                ui.menu_button(t("File"), |ui| {
                    if self.browser.is_some() {
                        if ui.button(t("Share position")).clicked() {
                            let result = self.share();
                            self.outcome(result);
                            ui.close();
                        }
                        if ui.button(t("Open position")).clicked() {
                            if let Some(browser) = &self.browser {
                                browser.open();
                            }
                            ui.close();
                        }
                    } else {
                        ui.label("Session path");
                        ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(250.0));
                        if ui.button("Save").clicked() {
                            let result = self.save();
                            self.outcome(result);
                            ui.close();
                        }
                        if ui.button("Load session").clicked() {
                            self.request(Pending::Load(PathBuf::from(&self.path)));
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button(t("New game")).clicked() {
                        self.navigate(Screen::Home);
                        ui.close();
                    }
                });
                ui.toggle_value(&mut self.show_settings, t("Settings"));
                if self.game.has_computer() {
                    ui.separator();
                    let config = &self.game.session().desktop.as_ref().unwrap().computer;
                    let paused = config.paused;
                    let mut fast = config.fast_play;
                    if ui
                        .button(if paused {
                            t("Resume computer")
                        } else {
                            t("Pause computer")
                        })
                        .clicked()
                    {
                        self.game.set_computer_paused(!paused);
                        self.refresh_revision();
                    }
                    if ui.checkbox(&mut fast, t("Fast play")).changed() {
                        self.game.set_fast_play(fast, Instant::now());
                    }
                }
                if self.browser.is_none() && self.dirty() {
                    ui.small("Unsaved");
                }
            });
        });
    }
    fn contents(&mut self, ui: &mut egui::Ui) {
        *self.repaint.lock().unwrap() = Some(ui.ctx().clone());
        let zoom = self.text_size.zoom();
        if self.applied_zoom != Some(zoom) {
            // Takes effect from the next frame on.
            ui.ctx().set_zoom_factor(zoom);
            self.applied_zoom = Some(zoom);
        }
        self.poll();
        self.poll_opened();
        let ctx = ui.ctx().clone();
        self.keyboard(&ctx);
        self.flush_correction(Instant::now());
        if let Some(due) = self.game.dice().correction_due
            && !self.game.dice().faces.contains(&0)
        {
            ctx.request_repaint_after(due.saturating_duration_since(Instant::now()));
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request(Pending::Close);
        }
        if self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let modal = self.pending.is_some()
            || self.edit.is_some()
            || self.player_sheet.is_some()
            || self.show_settings;
        ui.add_enabled_ui(!modal, |ui| {
            if self.screen != Screen::Play {
                let phone = ui.available_width() < phone::PHONE_WIDTH;
                match self.screen {
                    Screen::Home if phone => self.phone_start_ui(ui),
                    Screen::AnalysisSetup if phone => self.phone_players_ui(ui),
                    Screen::LocalSetup if phone => self.phone_local_ui(ui),
                    Screen::ComputerSetup if phone => self.phone_computer_ui(ui),
                    _ => self.setup_ui(ui),
                }
                return;
            }
            if ui.available_width() < phone::PHONE_WIDTH {
                self.phone_ui(ui);
                return;
            }
            self.toolbar_ui(ui);
            ui.small(&self.message);
            let validation =
                if self.game.position_editor() && !self.game.session().position.hypothetical {
                    self.game
                        .session()
                        .position
                        .infer_next_player()
                        .and_then(|_| self.game.session().validate())
                } else {
                    self.game.session().validate()
                };
            if let Err(e) = validation {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    tf("Position needs attention: {error}", &[("error", &e)]),
                );
            }
            ui.separator();
            if self.game.session().position.phase == Phase::Finished
                && (!self.game.editing() || self.game.position_editor())
            {
                if !self.review_results {
                    self.results_ui(ui);
                } else {
                    if ui.button(t("Show results")).clicked() {
                        self.review_results = false;
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("review_sheets")
                        .show(ui, |ui| self.sheet_ui(ui));
                }
                return;
            }
            if ui.available_width() >= 950.0 {
                let height = ui.available_height();
                let left_width = ui.available_width() * 0.61;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_width, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            self.dice_ui(ui);
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .id_salt("play_sheets")
                                .show(ui, |ui| self.sheet_ui(ui));
                        },
                    );
                    ui.separator();
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("advice")
                            .show(ui, |ui| self.advice_ui(ui));
                    });
                });
            } else if ui.available_width() >= 600.0 && ui.available_height() >= 450.0 {
                self.dice_ui(ui);
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("compact_advice")
                    .max_height(180.0)
                    .show(ui, |ui| self.advice_ui(ui));
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("compact_sheets")
                    .show(ui, |ui| self.sheet_ui(ui));
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("compact_play")
                    .show(ui, |ui| {
                        self.dice_ui(ui);
                        ui.separator();
                        self.advice_ui(ui);
                        ui.separator();
                        self.sheet_ui(ui);
                    });
            }
        });
        if self.show_settings {
            let mut open = true;
            egui::Window::new("Settings")
                .open(&mut open)
                .resizable(true)
                .default_width(540.0)
                .vscroll(true)
                .show(&ctx, |ui| {
                    theme_buttons(ui);
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.engine_tab, true, "Engine");
                        ui.selectable_value(&mut self.engine_tab, false, "Game");
                    });
                    if !self.engine_tab {
                        let mut editing = self.game.editing();
                        if ui
                            .checkbox(
                                &mut editing,
                                "Edit position (enable historical and player editing)",
                            )
                            .changed()
                        {
                            self.game.set_editing(editing);
                            self.refresh_revision();
                        }
                    }
                    self.settings_ui(ui);
                });
            self.show_settings = open;
        }
        if let Some(store) = &mut self.preference_store
            && let Some(error) = store.error.clone()
        {
            egui::Window::new("Preferences")
                .collapsible(false)
                .show(&ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(error);
                        if store.save_failed && ui.button("Retry").clicked() {
                            store.retry();
                        }
                    });
                });
        }
        self.persist_preferences(&ctx);
        self.dialogs(&ctx);
        self.ensure_analysis(&ctx);
        self.tick_computer(&ctx, Instant::now());
    }
}
impl eframe::App for Desktop {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_opaque().to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn run(session: Session, path: Option<PathBuf>) -> Result<()> {
    session.validate()?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_transparent(false)
            .with_inner_size([1180.0, 900.0])
            .with_min_inner_size([360.0, 300.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Kniffel",
        options,
        Box::new(move |cc| {
            let app = Desktop::startup(session, path);
            if let Some(p) = &app.preferences {
                cc.egui_ctx.set_theme(match p.theme {
                    crate::preferences::Theme::System => egui::ThemePreference::System,
                    crate::preferences::Theme::Dark => egui::ThemePreference::Dark,
                    crate::preferences::Theme::Light => egui::ThemePreference::Light,
                });
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| Error::Invalid(format!("Cannot start graphical interface: {e}")))
}

#[cfg(test)]
mod tests;
