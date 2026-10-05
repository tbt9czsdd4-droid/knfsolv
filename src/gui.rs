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
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use web_time::Instant;

enum Pending {
    Local(LocalGameConfig),
    Computer(ComputerGameConfig),
    Analyze(usize),
    Load(PathBuf),
    Close,
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
    edit: Option<(usize, usize, String)>,
    score_wheel: ScoreWheel,
    score_wheel_limits: ScoreWheelLimits,
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
    repaint: Arc<Mutex<Option<egui::Context>>>,
    requested: Option<AnalysisKey>,
    analysis_mode: AnalysisMode,
    analysis_paused: bool,
    analysis_done: bool,
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
    response.on_hover_text(format!("Keep {face}"));
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
                format!("{:.1}%", 100. * m.outright_win),
                format!("{:.1}%", 100. * m.tied_first),
                format!(
                    "{:.1}%",
                    100. * (1. - m.outright_win - m.tied_first).clamp(0., 1.)
                ),
                format!("{:.1}", m.expected_score),
            ]
        })
        .unwrap_or_else(|| std::array::from_fn(|_| "Estimating…".into()))
}
fn move_statistics(ui: &mut egui::Ui, metrics: Option<Metrics>) {
    let values = statistic_values(metrics);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        for (label, value) in ["Win", "Tie", "Lose", "Expected final game score"]
            .into_iter()
            .zip(values)
        {
            ui.vertical(|ui| {
                if label != "Expected final game score" {
                    ui.set_width(76.0);
                }
                ui.small(label)
                    .on_hover_text(if label == "Expected final game score" {
                        "Expected total at the end of the game, including future turns and bonuses."
                    } else {
                        "Win: outright first. Tie: tied for first. Lose: all remaining outcomes."
                    });
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
            score_wheel: ScoreWheel::default(),
            score_wheel_limits: ScoreWheelLimits::default(),
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
            repaint,
            requested: None,
            analysis_mode: advisor.mode,
            analysis_paused: advisor.paused,
            analysis_done: false,
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
        app.local_setup.names = (0..players.clamp(2, 8))
            .map(|i| format!("Player {}", i + 1))
            .collect();
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
    fn install_preferences(&mut self, p: crate::preferences::Preferences) {
        self.local_setup = p.local.clone();
        self.computer_setup = p.computer.clone();
        self.custom_search = p.custom_search;
        self.custom_settings = p.custom_settings.clone();
        self.analysis_players = p.analysis_players;
        self.score_wheel_limits = p.score_wheel_limits;
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
    fn navigate(&mut self, screen: Screen) {
        if screen != Screen::Play && self.game.control() == PlayerControl::Computer {
            self.game.invalidate_jobs();
            self.refresh_revision();
        }
        self.screen = screen;
        self.show_settings = false;
        self.edit = None;
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
        self.game.play(action, Instant::now())?;
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
    fn request(&mut self, pending: Pending) {
        if self.dirty() {
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
                self.started_session(
                    "New local game. Choose which dice to keep; click a score to end each turn.",
                );
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
                self.started_session("New computer game. Human rolls wait for your input; computer dice are generated automatically.");
            }
            Pending::Analyze(n) => {
                if let Err(e) = self.game.replace_session(Session::new(n)) {
                    self.message = e.to_string();
                    return;
                }
                self.refresh_revision();
                if self.preferences.is_none() {
                    self.analysis_mode = AnalysisMode::Continuous;
                    self.analysis_paused = false;
                }
                self.started_session("Edit scores to forecast the next player. Dice are optional; enter five for move advice.");
            }
            Pending::Load(path) => match self.game.load(&path) {
                Ok(()) => {
                    self.refresh_revision();
                    self.path = path.display().to_string();
                    self.current_path = path;
                    self.saved = snapshot(self.game.session());
                    self.message = format!("Loaded {}", self.path);
                    self.computer_notice.clear();
                    self.session_active = true;
                    self.navigate(Screen::Play);
                    if self.preferences.is_none() && self.game.session().desktop.is_none() {
                        self.analysis_mode = AnalysisMode::Continuous;
                        self.analysis_paused = false;
                    }
                }
                Err(e) => self.message = e.to_string(),
            },
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
        let settings = if self.game.editing() {
            self.game.session().settings.clone()
        } else {
            self.game.coaching_settings()
        };
        let key = self.analysis_key();
        self.worker.submit(AnalysisRequest {
            key,
            position: self.game.session().position.clone(),
            settings,
            continuous: self.analysis_mode == AnalysisMode::Continuous,
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
    fn analysis_controls(&mut self, mode: AnalysisMode, paused: bool) {
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
                ui.heading(if self.game.position_editor() { "Dice (optional)" } else { "Dice" });
                if self.game.position_editor() && !self.game.session().position.hypothetical {
                    match self.game.session().position.infer_next_player() {
                        Ok(Some(player)) => { ui.strong(format!("Next: {}", self.game.session().position.players[player].name)); }
                        Ok(None) => { ui.strong("Game finished"); }
                        Err(_) => { ui.strong("Next player unresolved"); }
                    }
                } else {
                    ui.strong(format!("Turn: {}", self.game.session().position.players[self.game.session().position.active_player].name));
                }
                if let Phase::Decision { rerolls_left, .. } = phase { ui.label(format!("{rerolls_left} {} left", if rerolls_left == 1 { "reroll" } else { "rerolls" })); }
            });
            ui.small(match phase {
                Phase::BeforeFirstRoll if self.game.position_editor() => "Forecast averages over future rolls. Optionally enter five faces for move advice.",
                _ if self.game.control() == PlayerControl::Computer && !self.game.editing() => "The computer rolls and plays automatically.",
                Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } if !physical => "Ready when you are. Roll to continue.",
                Phase::BeforeFirstRoll => "Enter five faces with the picker or keys 1–6.",
                Phase::AwaitingRoll { .. } => "Enter new faces only. Retained dice are locked.",
                Phase::Decision { .. } if physical => "Click dice to keep. Use the picker or 1–6 to correct the selected die.",
                Phase::Decision { .. } => "Click dice to keep, then reroll or choose a score.",
                Phase::Finished => "Game finished.",
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
                            let status = if locked { "Locked" } else if keep { "Keep" } else if face == 0 { if physical { "Enter" } else { "Ready" } } else if matches!(phase, Phase::Decision { rerolls_left: 1..=2, .. }) { "Reroll" } else { "Rolled" };
                            let response = ui.add_enabled_ui(!locked && phase != Phase::Finished && (self.game.editing() || self.game.control() == PlayerControl::Human), |ui| {
                                die_button(ui, face, tile, keep || locked, self.game.dice().selected == Some(i),
                                    format!("Die {}: {}; {status}{}", i + 1, if face == 0 { "empty".into() } else { face.to_string() }, if hint { "; Advice" } else { "" }))
                            }).inner;
                            if response.on_hover_text("Click to select for correction and toggle Keep. Advice is the engine recommendation only.").clicked() {
                                self.game.select_die(i, true);
                            }
                            ui.small(status);
                            ui.small(if hint { "★ Advice" } else { " " });
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
                                if response.on_hover_text(format!("Enter {face} · key {face}")).clicked() { self.choose_face(face, Instant::now()); }
                            }
                            if ui.add_sized([44., 32.], egui::Button::new("Clear")).on_hover_text("Backspace: clear the last editable die").clicked() { self.clear_die(Instant::now()); }
                        });
                    });
                }
                let can_roll = self.game.human_can_play();
                match phase {
                    Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } if self.game.dice_source() == DiceSource::Digital => {
                        if primary_button(ui, "Roll", can_roll).clicked() {
                            let result = self.game.roll_digital(Instant::now()); self.outcome(result); self.refresh_revision();
                        }
                    }
                    Phase::Decision { rerolls_left, .. } => {
                        let count = self.game.dice().keep.iter().filter(|&&k| !k).count();
                        if primary_button(ui, &format!("Reroll {count} {}", if count == 1 { "die" } else { "dice" }), can_roll && rerolls_left > 0 && count > 0)
                            .on_hover_text(if physical { "Retain selected dice, then enter the new physical faces." } else { "Generate the unkept dice now." }).clicked() {
                            let result = self.game.reroll_selected(Instant::now()); self.outcome(result); self.refresh_revision();
                        }
                    }
                    _ => { ui.allocate_space(egui::vec2(160., 44.)); }
                }
            });
            if self.game.position_editor() && phase != Phase::Finished
                && (phase != Phase::BeforeFirstRoll || self.game.dice().faces != [0; 5])
                && ui.button("Analyze before first roll").clicked() {
                let result = self.game.analyze_before_roll();
                self.outcome(result);
                self.refresh_revision();
            }
            if self.dice_draft() { ui.small(if self.game.dice().faces.contains(&0) { "Complete the roll to continue." } else { "Updating roll…" }); }
        });
    }
    fn sheet_ui(&mut self, ui: &mut egui::Ui) {
        surface(ui).show(ui, |ui| self.sheet_contents(ui));
    }
    fn sheet_contents(&mut self, ui: &mut egui::Ui) {
        ui.set_width(ui.available_width());
        ui.heading("Score sheets");
        ui.small(if self.game.position_editor() {
            "Hover and scroll to change a score; click to type. Empty = open; 0 = crossed out. Filled entries determine the next player."
        } else if self.game.editing() {
            "Edit mode: click any entry. Empty = open; 0 = crossed out. Turns are paused."
        } else {
            "Click an available score to end the turn, including 0. Undo restores it. ★ = advice."
        });
        let mut action = None;
        let mut wheel_cell = None;
        let ready = self.game.human_can_play();
        let suggested = self.report.as_ref().and_then(|r| r.best_action);
        egui::ScrollArea::horizontal().id_salt("sheets").show(ui, |ui| {
            egui::Grid::new("sheet").striped(true).min_col_width(85.0).min_row_height(26.0).show(ui, |ui| {
                ui.strong("Category");
                for (i, p) in self.game.session().position.players.iter().enumerate() {
                    let active = i == self.game.session().position.active_player && self.game.session().position.phase != Phase::Finished
                        && (!self.game.position_editor() || self.game.session().position.hypothetical || self.game.session().position.infer_next_player().is_ok());
                    ui.label(egui::RichText::new(format!("{}{}", p.name, if active { " (turn)" } else { "" })).strong()
                        .background_color(if active { ui.visuals().selection.bg_fill } else { egui::Color32::TRANSPARENT }));
                }
                ui.end_row();
                for category in 0..13 {
                    ui.label(rules::NAMES[category]);
                    for (player, p) in self.game.session().position.players.iter().enumerate() {
                        let value = p.sheet.cells[category];
                        if self.game.editing() {
                            let response = ui.push_id(("score_cell", player, category), |ui| {
                                ui.add_sized([76., 26.], egui::Button::new(value.map(|v| v.to_string()).unwrap_or_else(|| "Open".into())))
                            }).inner;
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("score_cell_rect", player, category)), response.rect));
                            if self.game.position_editor() && response.enabled() && response.hovered() {
                                wheel_cell = Some((player, category));
                            }
                            let response = if self.game.position_editor() {
                                response.on_hover_text("Scroll up/down through legal scores. Open = unfilled; 0 = crossed out. Click to enter manually. Wheel limits: Settings → Game.")
                            } else { response };
                            if response.clicked() {
                                self.edit = Some((player, category, value.map(|v| v.to_string()).unwrap_or_default()));
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
                                .on_hover_text(if rules::extra_eligible(&p.sheet, dice) { "Score and advance; also awards 50 extra Kniffel points." } else { "Score and advance to the next player." }).clicked()
                            {
                                action = Some(if points == 0 { Action::CrossOut { category } } else { Action::Write { category } });
                            }
                        } else {
                            ui.label("Open");
                        }
                    }
                    ui.end_row();
                }
                for row in 0..4 {
                    ui.strong(["Upper subtotal", "Upper bonus", "Extra points", "TOTAL"][row]);
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
            if ui.small_button("Engine settings").clicked() { self.show_settings = true; self.engine_tab = true; }
            let label = match self.report.as_ref().and_then(|r| r.progress) {
                None => "Preparing…".to_owned(),
                Some(p) => match p.phase {
                    crate::solver::SearchPhase::MctsTraining => format!("Tree depth {}", p.max_decision_depth),
                    crate::solver::SearchPhase::MctsValidation => format!("Tree depth {} · validating", p.max_decision_depth),
                    crate::solver::SearchPhase::Exact => format!("Exact depth {}", p.max_decision_depth),
                    crate::solver::SearchPhase::Rollout => "Full-game rollouts".to_owned(),
                },
            };
            ui.small(label).on_hover_text("Deepest explored branch, not a depth completed uniformly across every move. Counts the analyzed player's decisions (root = 1), excluding dice outcomes, opponents and baseline rollout continuation.");
        });
        if !self.computer_notice.is_empty() {
            ui.label(&self.computer_notice);
        }
        if self.game.control() == PlayerControl::Computer && !self.game.editing() {
            ui.heading("Computer turn");
            let config = &self.game.session().desktop.as_ref().unwrap().computer;
            ui.label(if config.search.is_some() {
                "Custom search · strength uncalibrated"
            } else {
                config.policy.label()
            });
            if config.paused {
                ui.label("Computer paused.");
            } else if let Some(action) = self.game.pending_computer_action() {
                ui.strong(format!("Choice: {action}"));
            } else if self.computer_requested.is_some() {
                ui.spinner();
            }
            if !self.computer_status.is_empty() {
                ui.label(&self.computer_status);
            }
            ui.small("Coaching resumes for the next human turn.");
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
                    "Analysis paused · inconsistent sheets".into()
                } else if self.before_roll_forecast() {
                    format!(
                        "Before-roll forecast · {}",
                        self.game.session().position.players
                            [self.game.session().settings.perspective]
                            .name
                    )
                } else {
                    "Advice".into()
                },
            );
            ui.selectable_value(&mut mode, AnalysisMode::Continuous, "Continuous");
            ui.selectable_value(&mut mode, AnalysisMode::Limited, "Limited");
            if ui
                .button(if paused {
                    "Resume analysis"
                } else {
                    "Pause analysis"
                })
                .clicked()
            {
                paused = !paused;
            }
            if self.running.is_some() && !paused {
                ui.spinner();
            }
            if self.analysis_done && ui.button("Restart analysis").clicked() {
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
                    "Game finished."
                } else {
                    if self.game.position_editor() { "Analysis waits: correct the position, or complete five dice or choose Analyze before first roll." } else { "Coaching waits for a complete, valid human roll." }
                },
            );
            return;
        }
        if let Some(report) = &self.report {
            let progress = if self.analysis_paused {
                "Paused"
            } else if self.running.is_some() {
                "Refining"
            } else {
                "Stopped"
            };
            ui.small(format!(
                "{progress} · {}{} · Confidence: {}",
                if report.recommendation_basis.starts_with("baseline_") {
                    "Provisional baseline · "
                } else {
                    ""
                },
                report.status.replace('_', " "),
                report.confidence.replace('_', " ")
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
                ui.small("Averaged over possible future rolls and modeled play.");
                ui.small(format!(
                    "{} completed games · {:.2}s computing · {}",
                    report.samples,
                    report.elapsed_ms as f64 / 1000.,
                    report.stop_reason
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
                ui.label("No recommendation yet.");
                if report.forecast.is_some() {
                    ui.strong("Game forecast");
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
                        "Show fewer"
                    } else {
                        "Show all moves"
                    })
                    .clicked()
            {
                self.show_all_moves = !self.show_all_moves;
            }
            egui::CollapsingHeader::new("Analysis details")
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
                "Paused · waiting for an estimate."
            } else {
                "Preparing provisional baseline advice…"
            });
        }
    }
    fn move_title(&self, action: Action) -> String {
        match action {
            Action::CrossOut { category } => {
                format!("Score {} · 0 points now", rules::NAMES[category])
            }
            Action::Write { category } => {
                if let Phase::Decision { dice, .. } = self.game.session().position.phase {
                    let sheet = &self.game.session().position.players
                        [self.game.session().position.active_player]
                        .sheet;
                    format!(
                        "Score {} · {} points now{}",
                        rules::NAMES[category],
                        rules::score(sheet, dice, category),
                        if rules::extra_eligible(sheet, dice) {
                            " (+50 extra Kniffel)"
                        } else {
                            ""
                        }
                    )
                } else {
                    action.to_string()
                }
            }
            Action::Reroll { keep } => format!(
                "Reroll {} {}",
                5 - keep.len(),
                if keep.len() == 4 { "die" } else { "dice" }
            ),
            Action::RollAll => "Roll all five dice".into(),
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
                                "Recommended"
                            } else {
                                "Alternative"
                            });
                            ui.label(self.move_title(action));
                        });
                        if let Action::Reroll { keep } = action {
                            ui.horizontal_wrapped(|ui| {
                                ui.small(if keep.is_empty() { "Keep none" } else { "Keep" });
                                for face in keep.faces() {
                                    small_die(ui, face);
                                }
                            });
                        }
                        move_statistics(ui, estimate.and_then(|a| a.metrics));
                        ui.push_id(("play_move", action), |ui| {
                            let response = ui.add_enabled(
                                self.game.human_can_play()
                                    && self.game.session().position.actions().contains(&action),
                                egui::Button::new(if recommended {
                                    "Play suggestion"
                                } else {
                                    "Play this move"
                                })
                                .min_size(egui::vec2(120., 32.)),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    response.enabled(),
                                    format!(
                                        "{}: {}",
                                        if recommended {
                                            "Play suggestion"
                                        } else {
                                            "Play this move"
                                        },
                                        self.move_title(action)
                                    ),
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
        if let Some((player, category, mut text)) = self.edit.clone() {
            egui::Window::new("Edit sheet entry")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "{} · {}",
                        self.game.session().position.players[player].name,
                        rules::NAMES[category]
                    ));
                    ui.label(if self.game.position_editor() {
                        "Empty = open. 0 = crossed out. Filled entries determine the next player."
                    } else {
                        "Empty = open. 0 = crossed out. Editing does not advance the turn."
                    });
                    ui.text_edit_singleline(&mut text);
                    self.edit = Some((player, category, text.clone()));
                    ui.horizontal(|ui| {
                        if ui.button("Apply edit").clicked() {
                            let result = (|| {
                                let value = if text.trim().is_empty() {
                                    None
                                } else {
                                    Some(text.trim().parse::<u16>().map_err(|_| {
                                        Error::Invalid(
                                            "Enter a nonnegative score or leave empty".into(),
                                        )
                                    })?)
                                };
                                let mut next = self.game.session().clone();
                                next.position.players[player].sheet.cells[category] = value;
                                next.position.players[player].sheet.validate()?;
                                self.commit(next);
                                self.edit = None;
                                Ok(())
                            })();
                            self.outcome(result);
                        }
                        if ui.button("Cancel").clicked() {
                            self.edit = None;
                        }
                    });
                });
        }
        if self.pending.is_some() {
            egui::Window::new("Unsaved changes")
                .collapsible(false)
                .resizable(false)
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
            ui.label("Standard rules · two rerolls per turn · 13 scoring categories");
            ui.add_space(12.0);
            if self.screen == Screen::Home {
                ui.heading("Choose how to play");
                if ui.add_sized([220.0, 40.0], egui::Button::new("Local game")).clicked() {
                    self.navigate(Screen::LocalSetup);
                }
                ui.small("2–8 people sharing a score sheet, with physical or digital dice.");
                if ui.add_sized([220.0, 40.0], egui::Button::new("Play computer")).clicked() {
                    self.navigate(Screen::ComputerSetup);
                }
                if ui.add_sized([220.0, 40.0], egui::Button::new("Analyze a position")).clicked() {
                    self.navigate(Screen::AnalysisSetup);
                }
                ui.separator();
                ui.label("Session path");
                ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(360.0));
                if ui.button("Load session").clicked() {
                    self.request(Pending::Load(PathBuf::from(&self.path)));
                }
            } else if self.screen == Screen::LocalSetup {
                ui.heading("Local game");
                let mut count = self.local_setup.names.len();
                ui.horizontal_wrapped(|ui| {
                    ui.label("Players");
                    for n in 2..=8 {
                        ui.selectable_value(&mut count, n, n.to_string());
                    }
                });
                for i in self.local_setup.names.len()..count {
                    self.local_setup.names.push(format!("Player {}", i + 1));
                }
                self.local_setup.names.truncate(count);
                for (i, name) in self.local_setup.names.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("Player {}", i + 1));
                        ui.add(egui::TextEdit::singleline(name).id_salt(("setup_name", i))
                            .hint_text(format!("Player {}", i + 1)).desired_width(220.0).char_limit(60));
                    });
                }
                self.local_setup.starting_player = self.local_setup.starting_player.min(count - 1);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Starting player");
                    egui::ComboBox::from_id_salt("setup_starting_player")
                        .selected_text(format!("{}. {}", self.local_setup.starting_player + 1,
                            self.local_setup.names[self.local_setup.starting_player]))
                        .show_ui(ui, |ui| {
                            for (i, name) in self.local_setup.names.iter().enumerate() {
                                ui.selectable_value(&mut self.local_setup.starting_player, i, format!("{}. {name}", i + 1));
                            }
                        });
                });
                ui.small("Play follows the listed order, wrapping around from the starting player.");
                ui.horizontal_wrapped(|ui| {
                    ui.label("Dice");
                    ui.selectable_value(&mut self.local_setup.dice_source, DiceSource::Physical, "Physical dice");
                    ui.selectable_value(&mut self.local_setup.dice_source, DiceSource::Digital, "Digital dice");
                });
                ui.label(match self.local_setup.dice_source {
                    DiceSource::Physical => "Roll your own dice, then enter their faces with the palette or keys 1–6.",
                    DiceSource::Digital => "Click Roll when you are ready. The app generates the dice.",
                });
                let validation = self.local_setup.session(0);
                if let Err(e) = &validation {
                    ui.colored_label(ui.visuals().error_fg_color, e.to_string());
                }
                if ui.add_enabled(validation.is_ok(), egui::Button::new("Start local game").min_size(egui::vec2(180.0, 36.0))).clicked() {
                    self.request(Pending::Local(self.local_setup.clone()));
                }
            } else if self.screen == Screen::ComputerSetup {
                self.computer_setup_ui(ui);
            } else {
                ui.heading("Analyze a position");
                ui.label("Start with empty sheets, then edit any scores, enter dice, and set the active player.");
                ui.horizontal_wrapped(|ui| {
                    ui.label("Players");
                    for n in 1..=8 {
                        ui.selectable_value(&mut self.analysis_players, n, n.to_string());
                    }
                });
                if ui.button("Open position editor").clicked() {
                    self.request(Pending::Analyze(self.analysis_players));
                }
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if self.screen != Screen::Home && ui.button("Back").clicked() {
                    self.navigate(Screen::Home);
                }
                if self.session_active && ui.button("Return to session").clicked() {
                    self.navigate(Screen::Play);
                }
            });
            ui.label(&self.message);
            egui::widgets::global_theme_preference_buttons(ui);
        });
    }
    fn computer_setup_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Play computer");
        ui.horizontal(|ui| {
            ui.label("Your name");
            ui.add(
                egui::TextEdit::singleline(&mut self.computer_setup.human_name)
                    .desired_width(180.0)
                    .char_limit(60),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Computer name");
            ui.add(
                egui::TextEdit::singleline(&mut self.computer_setup.computer_name)
                    .desired_width(180.0)
                    .char_limit(60),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Starts");
            ui.selectable_value(&mut self.computer_setup.human_starts, true, "You first");
            ui.selectable_value(
                &mut self.computer_setup.human_starts,
                false,
                "Computer first",
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Your dice");
            ui.selectable_value(
                &mut self.computer_setup.dice_source,
                DiceSource::Physical,
                "Physical dice",
            );
            ui.selectable_value(
                &mut self.computer_setup.dice_source,
                DiceSource::Digital,
                "Digital dice",
            );
        });
        ui.small(
            "Computer rolls are always generated by the app. Your digital rolls wait for Roll.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Difficulty");
            for policy in [
                ComputerPolicy::Greedy,
                ComputerPolicy::LegacyOpportunity,
                ComputerPolicy::ContinuationV2,
            ] {
                if ui
                    .selectable_label(
                        !self.custom_search && self.computer_setup.computer.policy == policy,
                        policy.label(),
                    )
                    .clicked()
                {
                    self.custom_search = false;
                    self.computer_setup.computer.policy = policy;
                }
            }
            if ui
                .selectable_label(self.custom_search, "Custom search")
                .clicked()
            {
                self.custom_search = true;
            }
        });
        if self.custom_search {
            ui.label("Custom search · strength uncalibrated. More thinking does not guarantee stronger play.");
            ui.small("Continuation-v2 baseline with bounded MCTS and independent validation. Coaching approximates it with the baseline.");
            egui::Grid::new("computer_budgets").show(ui, |ui| {
                ui.label("Time (ms; 0 = unlimited)");
                ui.add(egui::DragValue::new(&mut self.custom_settings.time_ms).range(0..=120_000));
                ui.end_row();
                ui.label("Simulations");
                ui.add(
                    egui::DragValue::new(&mut self.custom_settings.simulations)
                        .range(1..=1_000_000_000),
                );
                ui.end_row();
                ui.label("Depth (0 = unlimited)");
                ui.add(egui::DragValue::new(&mut self.custom_settings.max_depth).range(0..=100));
                ui.end_row();
                ui.label("Nodes");
                ui.add(
                    egui::DragValue::new(&mut self.custom_settings.max_nodes).range(1..=1_000_000),
                );
                ui.end_row();
            });
            ui.checkbox(
                &mut self.custom_settings.conservative_advice,
                "Conservative advice",
            );
            self.computer_setup.computer.policy = ComputerPolicy::ContinuationV2;
            self.computer_setup.computer.search = Some(self.custom_settings.clone());
        } else {
            ui.label(self.computer_setup.computer.policy.model().name());
            ui.small("Presets use different decision policies, evaluated in held-out games.");
            self.computer_setup.computer.search = None;
        }
        ui.checkbox(&mut self.computer_setup.computer.fast_play, "Fast play");
        let validation = self.computer_setup.session(0);
        if let Err(error) = &validation {
            ui.label(error.to_string());
        }
        if ui
            .add_enabled(
                validation.is_ok(),
                egui::Button::new("Start computer game").min_size(egui::vec2(180.0, 36.0)),
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
            ui.heading("Game finished");
            let winners: Vec<_> = results.iter().filter(|r| r.winner).map(|r| r.name.as_str()).collect();
            ui.heading(format!("{}: {}", if winners.len() == 1 { "Winner" } else { "Tied winners" }, winners.join(", ")));
            ui.horizontal_wrapped(|ui| {
                if ui.button("New game").clicked() {
                    self.navigate(Screen::Home);
                }
                if ui.button("Review score sheets").clicked() {
                    self.review_results = true;
                }
            });
            ui.label("Totals include the upper bonus and extra Kniffel points. Undo restores the last scoring decision.");
            egui::ScrollArea::horizontal().id_salt("result_columns").show(ui, |ui| {
                egui::Grid::new("final_totals").striped(true).show(ui, |ui| {
                    for label in ["Player", "Upper subtotal", "Upper bonus", "Extra points", "TOTAL", "Result"] {
                        ui.strong(label);
                    }
                    ui.end_row();
                    for result in results {
                        ui.label(result.name);
                        ui.label(result.upper.to_string());
                        ui.label(result.bonus.to_string());
                        ui.label(result.extra_points.to_string());
                        ui.strong(result.total.to_string());
                        ui.label(if result.winner { "Winner" } else { "" });
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
                        "Edit position"
                    } else if self.game.dice_source() == DiceSource::Digital {
                        "Digital play"
                    } else {
                        "Physical play"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut editing, false, "Play");
                        ui.selectable_value(&mut editing, true, "Edit position");
                    });
                self.game.set_editing(editing);
                self.refresh_revision();
                ui.separator();
                if ui
                    .add_enabled(self.game.can_undo(), egui::Button::new("Undo"))
                    .on_hover_text("Ctrl/Cmd-Z · first resets unfinished dice corrections")
                    .clicked()
                {
                    self.undo(false);
                }
                if ui
                    .add_enabled(self.game.can_redo(), egui::Button::new("Redo"))
                    .clicked()
                {
                    self.undo(true);
                }
                ui.separator();
                ui.menu_button("File", |ui| {
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
                    ui.separator();
                    if ui.button("New game").clicked() {
                        self.navigate(Screen::Home);
                        ui.close();
                    }
                });
                ui.toggle_value(&mut self.show_settings, "Settings");
                if self.game.has_computer() {
                    ui.separator();
                    let config = &self.game.session().desktop.as_ref().unwrap().computer;
                    let paused = config.paused;
                    let mut fast = config.fast_play;
                    if ui
                        .button(if paused {
                            "Resume computer"
                        } else {
                            "Pause computer"
                        })
                        .clicked()
                    {
                        self.game.set_computer_paused(!paused);
                        self.refresh_revision();
                    }
                    if ui.checkbox(&mut fast, "Fast play").changed() {
                        self.game.set_fast_play(fast, Instant::now());
                    }
                }
                if self.dirty() {
                    ui.small("Unsaved");
                }
            });
        });
    }
    fn contents(&mut self, ui: &mut egui::Ui) {
        *self.repaint.lock().unwrap() = Some(ui.ctx().clone());
        self.poll();
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
        let modal = self.pending.is_some() || self.edit.is_some() || self.show_settings;
        ui.add_enabled_ui(!modal, |ui| {
            if self.screen != Screen::Play {
                self.setup_ui(ui);
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
                    format!("Position needs attention: {e}"),
                );
            }
            ui.separator();
            if self.game.session().position.phase == Phase::Finished
                && (!self.game.editing() || self.game.position_editor())
            {
                if !self.review_results {
                    self.results_ui(ui);
                } else {
                    if ui.button("Show results").clicked() {
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
                    egui::widgets::global_theme_preference_buttons(ui);
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
mod tests {
    use super::*;
    use crate::{
        coaching::AnalysisUpdate,
        dice::Dice,
        solver::{self, Cancel},
    };
    use std::time::Duration;

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
        let report =
            solver::analyze(&session.position, &session.settings, Cancel::default()).unwrap();
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
                    !texts(&advice_render(&mut app, &ctx, vec![]))
                        .contains(&"Play this move".into())
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
                        die_button(ui, face, 48., false, false, format!("Enter face {face}"))
                            .clicked()
                    );
                },
            );
            assert!(
                format!("{:?}", output.platform_output.events)
                    .contains(&format!("Enter face {face}"))
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
            assert!(labels.iter().any(|t| t
                == &format!("Simultaneous 95% sampling interval · {expected}: [0.2000, 0.8000]")));
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

    fn render(
        app: &mut Desktop,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
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
        let report =
            solver::analyze(&session.position, &session.settings, Cancel::default()).unwrap();
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
        let report =
            solver::analyze(&session.position, &session.settings, Cancel::default()).unwrap();
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
    fn computer_app(
        source: DiceSource,
        policy: ComputerPolicy,
        search: Option<Settings>,
    ) -> Desktop {
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
}
