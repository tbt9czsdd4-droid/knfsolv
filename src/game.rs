//! Desktop game state and commands, independent of any rendering or computation worker.
use crate::{
    Error, Result,
    dice::Dice,
    persistence::Session,
    policy::Policy,
    rules::Action,
    solver::{Method, Settings},
    state::Phase,
};
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerControl {
    #[default]
    Human,
    Computer,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiceSource {
    #[default]
    Physical,
    Digital,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisMode {
    #[default]
    Continuous,
    Limited,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdvisorConfig {
    pub mode: AnalysisMode,
    pub paused: bool,
}
/// Persist policy identities independently of presentation/difficulty labels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerPolicy {
    Greedy,
    LegacyOpportunity,
    #[default]
    ContinuationV2,
}
impl ComputerPolicy {
    pub fn label(self) -> &'static str {
        // Held-out swapped-seat acceptance is recorded in docs/verification.md.
        match self {
            Self::Greedy => "Easy",
            Self::LegacyOpportunity => "Standard",
            Self::ContinuationV2 => "Strong",
        }
    }
    pub fn model(self) -> Policy {
        match self {
            Self::Greedy => Policy::Greedy,
            Self::LegacyOpportunity => Policy::LegacyOpportunity,
            Self::ContinuationV2 => Policy::Opportunity,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ComputerConfig {
    pub policy: ComputerPolicy,
    /// None uses the declared policy directly. Some stores a custom search budget/model.
    pub search: Option<Settings>,
    pub fast_play: bool,
    pub paused: bool,
}
impl ComputerConfig {
    pub fn baseline(&self) -> Policy {
        if self.search.is_some() {
            Policy::Opportunity
        } else {
            self.policy.model()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerGameConfig {
    pub human_name: String,
    pub computer_name: String,
    pub human_starts: bool,
    pub dice_source: DiceSource,
    pub computer: ComputerConfig,
}
impl Default for ComputerGameConfig {
    fn default() -> Self {
        Self {
            human_name: "You".into(),
            computer_name: "Computer".into(),
            human_starts: true,
            dice_source: DiceSource::Physical,
            computer: ComputerConfig {
                policy: ComputerPolicy::LegacyOpportunity,
                ..Default::default()
            },
        }
    }
}
impl ComputerGameConfig {
    pub fn session(&self, seed: u64) -> Result<Session> {
        let mut session = LocalGameConfig {
            names: vec![self.human_name.clone(), self.computer_name.clone()],
            starting_player: usize::from(!self.human_starts),
            dice_source: self.dice_source,
        }
        .session(seed)?;
        let desktop = session.desktop.as_mut().unwrap();
        desktop.controllers[1] = PlayerControl::Computer;
        desktop.computer = self.computer.clone();
        // Saved positions also declare this opponent to bounded CLI/TUI analysis.
        session.settings.opponent_policies =
            vec![session.settings.policy, self.computer.baseline()];
        session.validate()?;
        Ok(session)
    }
}

/// ChaCha8 word stream, independent of the solver seed and simulation RNGs.
/// Version 1 maps u32 words to faces with rejection sampling, so saved games resume exactly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameRng {
    pub seed: u64,
    pub word_position: u64,
}
impl GameRng {
    fn fill(&mut self, faces: &mut [u8; 5]) -> Result<()> {
        let mut rng = ChaCha8Rng::seed_from_u64(self.seed);
        rng.set_word_pos(self.word_position as u128);
        for face in faces.iter_mut().filter(|f| **f == 0) {
            loop {
                let word = rng.next_u32();
                if word < u32::MAX - u32::MAX % 6 {
                    *face = (word % 6 + 1) as u8;
                    break;
                }
            }
        }
        self.word_position = rng
            .get_word_pos()
            .try_into()
            .map_err(|_| Error::Invalid("Gameplay RNG stream exhausted".into()))?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopGame {
    pub version: u32,
    pub controllers: Vec<PlayerControl>,
    #[serde(default)]
    pub dice_source: DiceSource,
    #[serde(default)]
    pub advisor: AdvisorConfig,
    #[serde(default)]
    pub computer: ComputerConfig,
    pub rng: GameRng,
}
impl DesktopGame {
    pub fn new(players: usize, dice_source: DiceSource, seed: u64) -> Self {
        Self {
            version: 1,
            controllers: vec![PlayerControl::Human; players],
            dice_source,
            advisor: AdvisorConfig::default(),
            computer: ComputerConfig::default(),
            rng: GameRng {
                seed,
                word_position: 0,
            },
        }
    }
    pub fn validate(&self, session: &Session) -> Result<()> {
        if self.version != 1 || self.controllers.len() != session.position.players.len() {
            return Err(Error::Invalid(
                "Desktop game needs version 1 and one controller per player".into(),
            ));
        }
        if let Some(settings) = &self.computer.search {
            settings.validate(&session.position)?;
        }
        Ok(())
    }
}

/// New local matches always start from empty sheets with two to eight humans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalGameConfig {
    pub names: Vec<String>,
    pub starting_player: usize,
    pub dice_source: DiceSource,
}
impl Default for LocalGameConfig {
    fn default() -> Self {
        Self {
            names: vec!["Player 1".into(), "Player 2".into()],
            starting_player: 0,
            dice_source: DiceSource::Physical,
        }
    }
}
impl LocalGameConfig {
    pub fn session(&self, seed: u64) -> Result<Session> {
        let n = self.names.len();
        if !(2..=8).contains(&n) || self.starting_player >= n {
            return Err(Error::Invalid(
                "Choose 2–8 players and a starting player.".into(),
            ));
        }
        if self.names.iter().any(|name| name.trim().is_empty()) {
            return Err(Error::Invalid("Enter a name for every player.".into()));
        }
        let mut session = Session::new(n);
        for (player, name) in session.position.players.iter_mut().zip(&self.names) {
            player.name = name.trim().into();
        }
        session
            .position
            .turn_order
            .rotate_left(self.starting_player);
        session.position.active_player = self.starting_player;
        session.settings.perspective = self.starting_player;
        session.desktop = Some(DesktopGame::new(n, self.dice_source, seed));
        session.validate()?;
        Ok(session)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerResult {
    pub player: usize,
    pub name: String,
    pub upper: u16,
    pub bonus: u16,
    pub extra_points: u16,
    pub total: u16,
    pub winner: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiceDraft {
    pub faces: [u8; 5],
    pub locked: [bool; 5],
    pub keep: [bool; 5],
    pub selected: Option<usize>,
    pub correction_due: Option<Instant>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameEventKind {
    Roll { faces: [u8; 5], generated: bool },
    Correction { faces: [u8; 5] },
    Action(Action),
    Edit,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameEvent {
    pub actor: usize,
    pub kind: GameEventKind,
}
#[derive(Clone)]
struct Frame {
    session: Session,
    dice: DiceDraft,
    events: Vec<GameEvent>,
}
#[derive(Clone)]
struct UndoEntry {
    before: Frame,
    human_boundary: bool,
}
struct RedoEntry {
    after: Frame,
    undone: Vec<UndoEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobPurpose {
    Coaching,
    ComputerRoll,
    ComputerDecision,
    ComputerAction,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JobToken {
    pub revision: u64,
    pub actor: usize,
    pub purpose: JobPurpose,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomaticStep {
    Roll,
    Decide,
    Apply(Action),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomaticJob {
    pub token: JobToken,
    pub step: AutomaticStep,
}
#[derive(Clone, Copy)]
struct ScheduledAction {
    action: Action,
    due: Instant,
    token: JobToken,
}

pub struct GameController {
    session: Session,
    dice: DiceDraft,
    committed_faces: [u8; 5],
    editing: bool,
    revision: u64,
    undo: Vec<UndoEntry>,
    redo: Vec<RedoEntry>,
    events: Vec<GameEvent>,
    scheduled: Option<ScheduledAction>,
    automatic_after: Option<Instant>,
}
impl GameController {
    pub fn new(mut session: Session) -> Result<Self> {
        session.validate()?;
        if session.desktop.is_none() && !session.position.hypothetical {
            session.settings.perspective = session.position.active_player;
        }
        let mut game = Self {
            editing: session.desktop.is_none(),
            session,
            dice: DiceDraft::default(),
            committed_faces: [0; 5],
            revision: 0,
            undo: vec![],
            redo: vec![],
            events: vec![],
            scheduled: None,
            automatic_after: None,
        };
        game.sync_dice();
        Ok(game)
    }
    pub fn local(players: usize, source: DiceSource, seed: u64) -> Result<Self> {
        let mut session = Session::new(players);
        session.desktop = Some(DesktopGame::new(players, source, seed));
        Self::new(session)
    }
    pub fn start_local(&mut self, config: &LocalGameConfig, seed: u64) -> Result<()> {
        self.replace_session(config.session(seed)?)
    }
    /// Final totals use the same authoritative sheet calculations as scoring.
    pub fn results(&self) -> Option<Vec<PlayerResult>> {
        if self.session.position.phase != Phase::Finished || self.session.validate().is_err() {
            return None;
        }
        let high = self
            .session
            .position
            .players
            .iter()
            .map(|p| p.sheet.total())
            .max()?;
        Some(
            self.session
                .position
                .players
                .iter()
                .enumerate()
                .map(|(player, p)| PlayerResult {
                    player,
                    name: p.name.clone(),
                    upper: p.sheet.upper(),
                    bonus: p.sheet.bonus(),
                    extra_points: p.sheet.extra_kniffels as u16 * 50,
                    total: p.sheet.total(),
                    winner: p.sheet.total() == high,
                })
                .collect(),
        )
    }
    pub fn session(&self) -> &Session {
        &self.session
    }
    pub fn has_computer(&self) -> bool {
        self.session
            .desktop
            .as_ref()
            .is_some_and(|d| d.controllers.contains(&PlayerControl::Computer))
    }
    /// Normal-play coaching models the actual declared computer baseline.
    /// A custom computer's search is approximated by continuation-v2.
    pub fn coaching_settings(&self) -> Settings {
        let mut settings = self.session.settings.clone();
        settings.perspective = self.session.position.active_player;
        if self.has_computer() {
            let desktop = self.session.desktop.as_ref().unwrap();
            settings.opponent_policies = desktop
                .controllers
                .iter()
                .enumerate()
                .map(|(i, control)| {
                    if *control == PlayerControl::Computer {
                        desktop.computer.baseline()
                    } else {
                        self.session.settings.model(i)
                    }
                })
                .collect();
            // Computer presets use the bundled model, independent of advisor model paths.
            settings.baseline_model = None;
            settings.cache = None;
        }
        settings
    }
    pub fn computer_settings(&self) -> Settings {
        let mut settings = self
            .session
            .desktop
            .as_ref()
            .and_then(|d| d.computer.search.clone())
            .unwrap_or_default();
        settings.perspective = self.session.position.active_player;
        settings.method = Method::Mcts;
        settings.policy = self
            .session
            .desktop
            .as_ref()
            .map_or(Policy::Opportunity, |d| d.computer.baseline());
        settings.opponent_policies = self.coaching_settings().opponent_policies;
        settings.cache = None;
        settings.baseline_model = None;
        settings
    }
    pub fn pending_computer_action(&self) -> Option<Action> {
        self.scheduled.map(|s| s.action)
    }
    pub fn dice(&self) -> &DiceDraft {
        &self.dice
    }
    pub fn history(&self) -> &[GameEvent] {
        &self.events
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn editing(&self) -> bool {
        self.editing
    }
    pub fn position_editor(&self) -> bool {
        self.editing && self.session.desktop.is_none()
    }

    /// Discard entered dice in one reversible editor change.
    pub fn analyze_before_roll(&mut self) -> Result<()> {
        if !self.position_editor() || self.session.position.open() == 0 {
            return Err(Error::Invalid(
                "Before-roll forecasting requires an unfinished position editor sheet".into(),
            ));
        }
        let mut next = self.session.clone();
        next.position.phase = Phase::BeforeFirstRoll;
        if next == self.session && self.has_draft() {
            let mut before = self.frame();
            before.dice = self.dice.clone();
            before.dice.correction_due = None;
            self.record(next, DiceDraft::default(), vec![], true);
            self.undo.last_mut().unwrap().before = before;
        } else {
            self.edit_session(next)?;
        }
        Ok(())
    }
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
    pub fn can_undo(&self) -> bool {
        self.has_draft() || self.undo_index().is_some()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn control(&self) -> PlayerControl {
        self.session
            .desktop
            .as_ref()
            .map_or(PlayerControl::Human, |d| {
                d.controllers[self.session.position.active_player]
            })
    }
    pub fn dice_source(&self) -> DiceSource {
        if self.control() == PlayerControl::Computer {
            DiceSource::Digital
        } else {
            self.session
                .desktop
                .as_ref()
                .map_or(DiceSource::Physical, |d| d.dice_source)
        }
    }
    pub fn has_draft(&self) -> bool {
        self.dice.faces != self.committed_faces || self.dice.correction_due.is_some()
    }
    pub fn decision_pending(&self) -> bool {
        self.dice.correction_due.is_some()
            || matches!(self.session.position.phase, Phase::Decision { dice, .. }
            if Dice::from_faces(&self.dice.faces).ok() != Some(dice))
    }
    pub fn human_can_play(&self) -> bool {
        !self.editing
            && self.control() == PlayerControl::Human
            && !self.decision_pending()
            && self.session.validate().is_ok()
            && !matches!(self.session.position.phase, Phase::Finished)
    }
    pub fn physical_entry_allowed(&self) -> bool {
        !matches!(self.session.position.phase, Phase::Finished)
            && (self.editing
                || (self.control() == PlayerControl::Human
                    && self.dice_source() == DiceSource::Physical))
    }
    pub fn set_editing(&mut self, editing: bool) {
        if editing != self.editing {
            self.editing = editing;
            self.invalidate_jobs();
        }
    }
    /// Analysis controls do not change the position or frozen evaluation policy.
    /// Keep the revision so pausing or changing only the stopping rule retains evidence.
    /// Desktop global advice overlay; deliberately does not create an undo entry.
    pub fn set_global_advice(&mut self, engine: &Settings) {
        let perspective = self.session.settings.perspective;
        self.session.settings = engine.clone();
        self.session.settings.perspective = perspective;
        if !self.session.settings.opponent_policies.is_empty() {
            self.session
                .settings
                .opponent_policies
                .resize(self.session.position.players.len(), engine.policy);
        }
    }
    pub fn set_advisor_controls(&mut self, mode: AnalysisMode, paused: bool) {
        if let Some(desktop) = &mut self.session.desktop {
            desktop.advisor.mode = mode;
            desktop.advisor.paused = paused;
        }
    }
    /// Invalidates both advice and executable jobs, including queued presentation actions.
    pub fn invalidate_jobs(&mut self) {
        self.revision = self.revision.checked_add(1).expect("revision exhausted");
        self.scheduled = None;
        self.automatic_after = None;
    }
    pub fn token(&self, purpose: JobPurpose) -> JobToken {
        JobToken {
            revision: self.revision,
            actor: self.session.position.active_player,
            purpose,
        }
    }
    pub fn accepts(&self, token: JobToken) -> bool {
        token.revision == self.revision && token.actor == self.session.position.active_player
    }
    fn sync_dice(&mut self) {
        self.dice = DiceDraft::default();
        match self.session.position.phase {
            Phase::Decision { dice, .. } => self.dice.faces.copy_from_slice(&dice.faces()),
            Phase::AwaitingRoll { kept, .. } => {
                let faces = kept.faces();
                self.dice.faces[..faces.len()].copy_from_slice(&faces);
                self.dice.locked[..faces.len()].fill(true);
            }
            _ => (),
        }
        self.committed_faces = self.dice.faces;
    }
    fn frame(&self) -> Frame {
        let mut dice = self.dice.clone();
        dice.faces = self.committed_faces;
        dice.correction_due = None;
        dice.selected = None;
        Frame {
            session: self.session.clone(),
            dice,
            events: self.events.clone(),
        }
    }
    fn restore(&mut self, frame: Frame) {
        let paused = self.session.desktop.as_ref().map(|d| d.computer.paused);
        self.session = frame.session;
        if let (Some(paused), Some(desktop)) = (paused, &mut self.session.desktop) {
            desktop.computer.paused = paused;
        }
        self.dice = frame.dice;
        self.committed_faces = self.dice.faces;
        self.events = frame.events;
        self.invalidate_jobs();
    }
    fn record(
        &mut self,
        next: Session,
        dice: DiceDraft,
        events: Vec<GameEvent>,
        human_boundary: bool,
    ) {
        self.undo.push(UndoEntry {
            before: self.frame(),
            human_boundary,
        });
        self.redo.clear();
        self.session = next;
        self.dice = dice;
        self.committed_faces = self.dice.faces;
        self.events.extend(events);
        self.invalidate_jobs();
    }
    fn undo_index(&self) -> Option<usize> {
        let computer_game = self
            .session
            .desktop
            .as_ref()
            .is_some_and(|d| d.controllers.contains(&PlayerControl::Computer));
        if computer_game && !self.editing {
            self.undo.iter().rposition(|entry| entry.human_boundary)
        } else {
            self.undo.len().checked_sub(1)
        }
    }
    pub fn undo(&mut self) -> bool {
        // Even a no-op Undo invalidates an outstanding worker request.
        self.invalidate_jobs();
        if self.has_draft() {
            self.dice.faces = self.committed_faces;
            self.dice.selected = None;
            self.dice.correction_due = None;
            return true;
        }
        let Some(index) = self.undo_index() else {
            return false;
        };
        let after = self.frame();
        let undone = self.undo.split_off(index);
        self.restore(undone[0].before.clone());
        self.redo.push(RedoEntry { after, undone });
        true
    }
    pub fn redo(&mut self) -> bool {
        self.invalidate_jobs();
        if self.has_draft() {
            return false;
        }
        let Some(entry) = self.redo.pop() else {
            return false;
        };
        self.restore(entry.after);
        self.undo.extend(entry.undone);
        true
    }
    /// Load/new-game is a new history, even when the contents happen to be identical.
    pub fn replace_session(&mut self, mut session: Session) -> Result<()> {
        session.validate()?;
        if session.desktop.is_none() && !session.position.hypothetical {
            session.settings.perspective = session.position.active_player;
        }
        self.editing = session.desktop.is_none();
        self.session = session;
        self.undo.clear();
        self.redo.clear();
        self.events.clear();
        self.sync_dice();
        self.invalidate_jobs();
        Ok(())
    }
    pub fn load(&mut self, path: &Path) -> Result<()> {
        self.replace_session(Session::load(path)?)
    }
    /// Historical edits can temporarily violate turn order, but never indexing invariants.
    pub fn edit_session(&mut self, mut next: Session) -> Result<()> {
        if next == self.session {
            return Ok(());
        }
        if !(1..=8).contains(&next.position.players.len())
            || next.position.active_player >= next.position.players.len()
        {
            return Err(Error::Invalid(
                "Need 1–8 players and a valid active player".into(),
            ));
        }
        next.settings.validate(&next.position)?;
        let mut order = next.position.turn_order.clone();
        order.sort_unstable();
        if order != (0..next.position.players.len()).collect::<Vec<_>>() {
            return Err(Error::Invalid(
                "Turn order must contain each player once".into(),
            ));
        }
        for player in &next.position.players {
            player.sheet.validate()?;
        }
        let valid_phase = match next.position.phase {
            Phase::Decision { dice, rerolls_left } => {
                dice.0.iter().map(|&n| n as usize).sum::<usize>() == 5 && rerolls_left <= 2
            }
            Phase::AwaitingRoll {
                kept,
                rerolls_after,
            } => kept.0.iter().map(|&n| n as usize).sum::<usize>() < 5 && rerolls_after <= 1,
            _ => true,
        };
        if !valid_phase {
            return Err(Error::Invalid("Invalid dice or reroll count".into()));
        }
        if let Some(desktop) = &next.desktop {
            desktop.validate(&next)?;
        }
        if next.position != self.session.position && !self.editing {
            return Err(Error::Invalid(
                "Enable Edit position before changing historical entries".into(),
            ));
        }
        let turns_changed = next.position.turn_order != self.session.position.turn_order
            || next
                .position
                .players
                .iter()
                .map(|p| p.sheet.open())
                .collect::<Vec<_>>()
                != self
                    .session
                    .position
                    .players
                    .iter()
                    .map(|p| p.sheet.open())
                    .collect::<Vec<_>>();
        if self.position_editor() {
            if turns_changed {
                next.position.phase = Phase::BeforeFirstRoll;
            }
            if next.position.open() == 0 {
                next.position.phase = Phase::Finished;
            } else if !next.position.hypothetical {
                if let Ok(Some(player)) = next.position.infer_next_player() {
                    next.position.active_player = player;
                    next.settings.perspective = player;
                }
                if self.session.position.hypothetical != next.position.hypothetical {
                    next.position.phase = Phase::BeforeFirstRoll;
                }
            }
        }
        let reset = (self.position_editor() && turns_changed)
            || next.position.phase != self.session.position.phase
            || next.position.active_player != self.session.position.active_player;
        let actor = self.session.position.active_player;
        let human_boundary = self.editing || self.control() == PlayerControl::Human;
        let draft = self.dice.clone();
        let faces = self.committed_faces;
        self.record(
            next,
            self.dice.clone(),
            vec![GameEvent {
                actor,
                kind: GameEventKind::Edit,
            }],
            human_boundary,
        );
        if reset {
            self.sync_dice();
        } else {
            self.dice = draft;
            self.committed_faces = faces;
        }
        Ok(())
    }
    pub fn select_die(&mut self, slot: usize, toggle_keep: bool) {
        if slot >= 5 || self.dice.locked[slot] {
            return;
        }
        self.dice.selected = Some(slot);
        if toggle_keep {
            self.toggle_keep(slot);
        }
    }
    pub fn toggle_keep(&mut self, slot: usize) {
        if slot < 5
            && !self.dice.locked[slot]
            && !self.decision_pending()
            && (self.editing || self.control() == PlayerControl::Human)
            && matches!(
                self.session.position.phase,
                Phase::Decision {
                    rerolls_left: 1..=2,
                    ..
                }
            )
        {
            self.dice.keep[slot] = !self.dice.keep[slot];
        }
    }
    pub fn choose_face(&mut self, face: u8, now: Instant) -> Result<()> {
        if !(1..=6).contains(&face) {
            return Err(Error::Invalid("Die faces must be 1–6".into()));
        }
        if !self.physical_entry_allowed() {
            return Err(Error::Invalid(
                "Physical dice entry is unavailable for this turn".into(),
            ));
        }
        let slot = self
            .dice
            .selected
            .filter(|&i| !self.dice.locked[i])
            .or_else(|| (0..5).find(|&i| !self.dice.locked[i] && self.dice.faces[i] == 0));
        let Some(slot) = slot else { return Ok(()) };
        self.dice.faces[slot] = face;
        self.dice.selected = (0..5).find(|&i| !self.dice.locked[i] && self.dice.faces[i] == 0);
        self.dice_changed(now)
    }
    pub fn clear_die(&mut self, now: Instant) -> Result<()> {
        if !self.physical_entry_allowed() {
            return Err(Error::Invalid(
                "Physical dice entry is unavailable for this turn".into(),
            ));
        }
        if let Some(slot) = (0..5)
            .rev()
            .find(|&i| !self.dice.locked[i] && self.dice.faces[i] != 0)
        {
            self.dice.faces[slot] = 0;
            self.dice.selected = Some(slot);
            self.dice_changed(now)?;
        }
        Ok(())
    }
    fn dice_changed(&mut self, now: Instant) -> Result<()> {
        self.invalidate_jobs();
        if matches!(self.session.position.phase, Phase::Decision { .. }) {
            self.dice.correction_due = Some(now + Duration::from_millis(250));
        } else if !self.dice.faces.contains(&0) {
            self.commit_physical_roll()?;
        }
        Ok(())
    }
    fn commit_physical_roll(&mut self) -> Result<()> {
        let dice = Dice::from_faces(&self.dice.faces)?;
        if dice.len() != 5 {
            return Err(Error::Invalid("Enter all five dice".into()));
        }
        let mut next = self.session.clone();
        let kind = if let Phase::Decision { dice: current, .. } = &mut next.position.phase {
            *current = dice;
            GameEventKind::Correction {
                faces: self.dice.faces,
            }
        } else {
            next.position.enter_roll(dice)?;
            GameEventKind::Roll {
                faces: self.dice.faces,
                generated: false,
            }
        };
        let mut draft = self.dice.clone();
        draft.locked = [false; 5];
        draft.correction_due = None;
        draft.selected = None;
        if next == self.session {
            self.dice = draft;
            self.committed_faces = self.dice.faces;
        } else {
            let actor = self.session.position.active_player;
            self.record(next, draft, vec![GameEvent { actor, kind }], true);
        }
        Ok(())
    }
    pub fn flush_correction(&mut self, now: Instant) -> Result<()> {
        if self.dice.correction_due.is_some_and(|due| now >= due) && !self.dice.faces.contains(&0) {
            self.commit_physical_roll()?;
        }
        Ok(())
    }
    pub fn prepare_save(&mut self) -> Result<()> {
        if self.has_draft() {
            if self.dice.faces.contains(&0) {
                return Err(Error::Invalid(
                    "Complete the dice entry before saving, or use Undo to reset the draft".into(),
                ));
            }
            self.commit_physical_roll()?;
        }
        Ok(())
    }
    pub fn save(&mut self, path: &Path) -> Result<()> {
        self.prepare_save()?;
        self.session.save(path)
    }
    fn retained_draft(&self, keep: Dice) -> DiceDraft {
        let mut draft = DiceDraft::default();
        let selected: Vec<_> = self
            .dice
            .faces
            .iter()
            .zip(self.dice.keep)
            .filter_map(|(&f, k)| k.then_some(f))
            .collect();
        let use_selection = Dice::from_faces(&selected).ok() == Some(keep);
        let mut remaining = keep.0;
        for (i, &face) in self.dice.faces.iter().enumerate() {
            let retain = if use_selection {
                self.dice.keep[i]
            } else {
                remaining[face as usize - 1] > 0
            };
            if retain {
                remaining[face as usize - 1] -= 1;
                draft.faces[i] = face;
                draft.locked[i] = true;
            }
        }
        draft
    }
    fn generate(next: &mut Session, draft: &mut DiceDraft) -> Result<GameEventKind> {
        if !matches!(
            next.position.phase,
            Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. }
        ) {
            return Err(Error::Invalid("No actual roll is pending".into()));
        }
        let desktop = next
            .desktop
            .as_mut()
            .ok_or_else(|| Error::Invalid("Digital dice require a desktop game".into()))?;
        desktop.rng.fill(&mut draft.faces)?;
        next.position.enter_roll(Dice::from_faces(&draft.faces)?)?;
        draft.locked = [false; 5];
        draft.selected = None;
        draft.correction_due = None;
        Ok(GameEventKind::Roll {
            faces: draft.faces,
            generated: true,
        })
    }
    pub fn roll_digital(&mut self, now: Instant) -> Result<()> {
        if !self.human_can_play() || self.dice_source() != DiceSource::Digital || self.has_draft() {
            return Err(Error::Invalid(
                "An active human with digital dice must request the roll".into(),
            ));
        }
        self.generate_roll(true, now)
    }
    fn generate_roll(&mut self, human: bool, now: Instant) -> Result<()> {
        self.session.validate()?;
        let mut next = self.session.clone();
        let mut draft = self.dice.clone();
        let kind = Self::generate(&mut next, &mut draft)?;
        let actor = self.session.position.active_player;
        self.record(next, draft, vec![GameEvent { actor, kind }], human);
        self.delay_automatic(now);
        Ok(())
    }
    pub fn play(&mut self, action: Action, now: Instant) -> Result<()> {
        if !self.human_can_play() {
            return Err(Error::Invalid(
                "Finish correcting dice and leave Edit position before playing a human move".into(),
            ));
        }
        if action == Action::RollAll {
            return if self.dice_source() == DiceSource::Digital {
                self.roll_digital(now)
            } else {
                Err(Error::Invalid(
                    "Enter the physical roll using the face palette".into(),
                ))
            };
        }
        self.apply_action(action, true, now)
    }
    pub fn reroll_selected(&mut self, now: Instant) -> Result<()> {
        let faces: Vec<_> = self
            .dice
            .faces
            .iter()
            .zip(self.dice.keep)
            .filter_map(|(&f, k)| k.then_some(f))
            .collect();
        self.play(
            Action::Reroll {
                keep: Dice::from_faces(&faces)?,
            },
            now,
        )
    }
    fn apply_action(&mut self, action: Action, human: bool, now: Instant) -> Result<()> {
        self.session.validate()?;
        if !self.session.position.actions().contains(&action) {
            return Err(Error::Invalid(
                "Action is not legal in this position".into(),
            ));
        }
        let mut next = self.session.clone();
        next.position.apply(action)?;
        let actor = self.session.position.active_player;
        let mut events = vec![GameEvent {
            actor,
            kind: GameEventKind::Action(action),
        }];
        let mut draft = if let Action::Reroll { keep } = action {
            self.retained_draft(keep)
        } else {
            DiceDraft::default()
        };
        if human
            && self.dice_source() == DiceSource::Digital
            && matches!(action, Action::Reroll { .. })
        {
            let kind = Self::generate(&mut next, &mut draft)?;
            events.push(GameEvent { actor, kind });
        }
        self.record(next, draft, events, human);
        self.delay_automatic(now);
        Ok(())
    }
    fn computer_ready(&self) -> bool {
        !self.editing
            && self.control() == PlayerControl::Computer
            && !self.has_draft()
            && self
                .session
                .desktop
                .as_ref()
                .is_some_and(|d| !d.computer.paused)
            && self.session.validate().is_ok()
    }
    fn delay(&self) -> Duration {
        if self
            .session
            .desktop
            .as_ref()
            .is_some_and(|d| d.computer.fast_play)
        {
            Duration::ZERO
        } else {
            Duration::from_millis(400)
        }
    }
    fn delay_automatic(&mut self, now: Instant) {
        self.automatic_after = Some(now + self.delay());
    }
    pub fn set_computer_paused(&mut self, paused: bool) {
        if let Some(desktop) = &mut self.session.desktop
            && desktop.computer.paused != paused
        {
            desktop.computer.paused = paused;
            self.invalidate_jobs();
        }
    }
    pub fn set_fast_play(&mut self, fast: bool, now: Instant) {
        if let Some(desktop) = &mut self.session.desktop
            && desktop.computer.fast_play != fast
        {
            desktop.computer.fast_play = fast;
            self.automatic_after = Some(now + self.delay());
            let due = now + self.delay();
            if let Some(scheduled) = &mut self.scheduled {
                scheduled.due = due;
            }
        }
    }
    /// Polling never rolls a human's dice and never blocks the caller.
    pub fn automatic_job(&self, now: Instant) -> Option<AutomaticJob> {
        if !self.computer_ready() {
            return None;
        }
        if let Some(pending) = self.scheduled {
            return (now >= pending.due).then_some(AutomaticJob {
                token: pending.token,
                step: AutomaticStep::Apply(pending.action),
            });
        }
        if self.automatic_after.is_some_and(|due| now < due) {
            return None;
        }
        let (purpose, step) = match self.session.position.phase {
            Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } => {
                (JobPurpose::ComputerRoll, AutomaticStep::Roll)
            }
            Phase::Decision { .. } => (JobPurpose::ComputerDecision, AutomaticStep::Decide),
            Phase::Finished => return None,
        };
        Some(AutomaticJob {
            token: self.token(purpose),
            step,
        })
    }
    /// Only a current computer-decision job may enqueue a move. Stale/cancelled jobs do nothing.
    pub fn accept_computer_decision(
        &mut self,
        token: JobToken,
        action: Action,
        now: Instant,
    ) -> Result<bool> {
        if !self.accepts(token)
            || token.purpose != JobPurpose::ComputerDecision
            || !self.computer_ready()
            || self.scheduled.is_some()
        {
            return Ok(false);
        }
        if !self.session.position.actions().contains(&action) || action == Action::RollAll {
            return Err(Error::Invalid(
                "Computer returned an illegal decision".into(),
            ));
        }
        self.scheduled = Some(ScheduledAction {
            action,
            due: now + self.delay(),
            token: self.token(JobPurpose::ComputerAction),
        });
        if let Action::Reroll { keep } = action {
            self.dice.keep = self.retained_draft(keep).locked;
        }
        Ok(true)
    }
    pub fn execute_automatic(&mut self, job: AutomaticJob, now: Instant) -> Result<bool> {
        if self.automatic_job(now) != Some(job) || !self.accepts(job.token) {
            return Ok(false);
        }
        match job.step {
            AutomaticStep::Roll => self.generate_roll(false, now)?,
            AutomaticStep::Apply(action) => self.apply_action(action, false, now)?,
            AutomaticStep::Decide => return Ok(false), // Computation belongs to a worker.
        }
        Ok(true)
    }
}
