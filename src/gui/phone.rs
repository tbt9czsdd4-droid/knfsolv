//! The phone layout of the play screen, below [`PHONE_WIDTH`]: a fixed top bar, then one
//! vertical scroll with the dice, the advice card, and a compact score sheet. Wider windows keep
//! the desktop layouts. Every text is translated, and touch targets are at least 44 points.
use super::{
    Desktop, Pending, die_button,
    i18n::{category_name, category_short, count, decimal, percent, t, tf},
    small_die,
};
use crate::{
    game::{AnalysisMode, DiceSource, PlayerControl},
    rules::{self, Action},
    solver::{Alternative, Metrics, Report},
    state::Phase,
};
use eframe::egui;
use web_time::Instant;

pub(super) const PHONE_WIDTH: f32 = 600.0;
const TARGET: f32 = 44.0;
const LABEL_WIDTH: f32 = 72.0;
/// Player columns are at least this wide, so four players fit at 375 points; with fewer players
/// they grow up to [`WIDE_COLUMN`] so names like „Computer“ fit.
const COLUMN_WIDTH: f32 = 52.0;
const WIDE_COLUMN: f32 = 90.0;

/// How long each position is analyzed, as offered in the menu.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ThinkingTime {
    Quick,
    Thorough,
    NoLimit,
}

/// „Anna ist dran“, but „Ich bin dran“ for the analyzer's default first player and „Du bist
/// dran“ for the computer game's default human ("You").
fn is_up(name: &str) -> String {
    if name == super::start::default_name(0) {
        t("I am up").to_string()
    } else if name == "You" || name == t("You") {
        t("You are up").to_string()
    } else {
        tf("{name} is up", &[("name", &name)])
    }
}

/// One of several options, as a 44-point button that shows whether it is chosen.
fn choice(ui: &mut egui::Ui, selected: bool, label: &str) -> bool {
    ui.add(
        egui::Button::new(label)
            .selected(selected)
            .min_size(egui::vec2(TARGET, TARGET)),
    )
    .clicked()
}

fn big_button(ui: &mut egui::Ui, label: impl Into<egui::WidgetText>, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(label).min_size(egui::vec2(TARGET, TARGET)),
    )
    .clicked()
}

impl Desktop {
    pub(super) fn phone_ui(&mut self, ui: &mut egui::Ui) {
        // Touching text should scroll, not start a text selection.
        ui.style_mut().interaction.selectable_labels = false;
        self.phone_top_bar(ui);
        egui::ScrollArea::vertical()
            .id_salt("phone_play")
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !self.message.is_empty() {
                    ui.small(&self.message);
                }
                if self.game.session().position.phase == Phase::Finished && !self.game.editing() {
                    self.phone_results(ui);
                } else {
                    self.phone_dice(ui);
                    ui.separator();
                    self.phone_advice(ui);
                }
                ui.separator();
                self.phone_sheet(ui);
            });
    }

    /// The final standings, best first; the sheets follow below.
    fn phone_results(&mut self, ui: &mut egui::Ui) {
        let Some(mut results) = self.game.results() else {
            return;
        };
        ui.heading(t("Game finished"));
        let winners: Vec<&str> = (results.iter().filter(|r| r.winner))
            .map(|r| r.name.as_str())
            .collect();
        ui.strong(match winners[..] {
            [name] => tf("{name} wins", &[("name", &name)]),
            _ => tf("Tie: {names}", &[("names", &winners.join(", "))]),
        });
        results.sort_by_key(|r| std::cmp::Reverse(r.total));
        for (place, result) in results.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}.", place + 1));
                ui.strong(&result.name);
                if result.winner {
                    ui.label("🏆");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.strong(tf("{points} points", &[("points", &result.total)]));
                });
            });
            ui.small(tf(
                "Upper section {upper} · bonus {bonus} · extra Kniffels {extra}",
                &[
                    ("upper", &result.upper),
                    ("bonus", &result.bonus),
                    ("extra", &result.extra_points),
                ],
            ));
        }
        let new_game =
            egui::Button::new(t("New game")).min_size(egui::vec2(ui.available_width(), TARGET));
        if ui.add(new_game).clicked() {
            self.navigate(super::Screen::Home);
        }
        ui.small(t("↺ takes back the last entry."));
    }

    fn phone_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.set_min_height(TARGET);
            egui::containers::menu::MenuButton::from_button(
                egui::Button::new(egui::RichText::new("☰").size(22.0))
                    .min_size(egui::vec2(TARGET, TARGET)),
            )
            .ui(ui, |ui| self.phone_menu(ui));
            ui.strong("Kniffel-Coach");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if big_button(
                    ui,
                    egui::RichText::new("↻").size(20.0),
                    self.game.can_redo(),
                ) {
                    self.undo(true);
                }
                if big_button(
                    ui,
                    egui::RichText::new("↺").size(20.0),
                    self.game.can_undo(),
                ) {
                    self.undo(false);
                }
            });
        });
        ui.separator();
    }

    fn phone_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(240.0);
        let item = |ui: &mut egui::Ui, label: &str| {
            ui.add(egui::Button::new(label).min_size(egui::vec2(ui.available_width(), TARGET)))
                .clicked()
        };
        if self.game.position_editor() && item(ui, t("New position (same players)")) {
            self.request(Pending::Fresh);
            ui.close();
        }
        if item(ui, t("New game …")) {
            self.navigate(super::Screen::Home);
            ui.close();
        }
        ui.separator();
        ui.label(t("Appearance"));
        ui.horizontal(|ui| {
            let mut theme = ui.ctx().options(|o| o.theme_preference);
            let before = theme;
            for (value, label) in [
                (egui::ThemePreference::Light, t("Light")),
                (egui::ThemePreference::Dark, t("Dark")),
                (egui::ThemePreference::System, t("System")),
            ] {
                if choice(ui, theme == value, label) {
                    theme = value;
                }
            }
            if theme != before {
                ui.ctx().set_theme(theme);
            }
        });
        ui.label(t("Text size"));
        ui.horizontal(|ui| {
            use crate::preferences::TextSize;
            for (value, label) in [
                (TextSize::Normal, t("Normal")),
                (TextSize::Large, t("Large")),
            ] {
                if choice(ui, self.text_size == value, label) {
                    self.text_size = value;
                }
            }
        });
        ui.label(t("Thinking time"));
        ui.horizontal(|ui| {
            let current = self.thinking_time();
            for (value, label) in [
                (ThinkingTime::Quick, t("Quick")),
                (ThinkingTime::Thorough, t("Thorough")),
                (ThinkingTime::NoLimit, t("No limit")),
            ] {
                if choice(ui, current == value, label) && current != value {
                    self.set_thinking_time(value);
                }
            }
        });
        ui.separator();
        if self.browser.is_some() {
            if item(ui, t("Share position")) {
                let result = self.share();
                self.outcome(result);
                ui.close();
            }
            if item(ui, t("Open position")) {
                if let Some(browser) = &self.browser {
                    browser.open();
                }
                ui.close();
            }
        } else if item(ui, t("Save")) {
            let result = self.save();
            self.outcome(result);
            ui.close();
        }
        ui.separator();
        if item(ui, t("Advanced …")) {
            self.show_settings = true;
            ui.close();
        }
    }

    fn engine(&self) -> crate::solver::Settings {
        self.preferences
            .as_ref()
            .map(|p| p.engine.clone())
            .unwrap_or_else(|| self.game.session().settings.clone())
    }
    fn thinking_time(&self) -> ThinkingTime {
        if self.analysis_mode == AnalysisMode::Continuous {
            ThinkingTime::NoLimit
        } else if self.engine().time_ms > 3_000 {
            ThinkingTime::Thorough
        } else {
            ThinkingTime::Quick
        }
    }
    /// Quick is the phone default (3 s); Thorough allows 15 s; No limit refines until changed.
    fn set_thinking_time(&mut self, choice: ThinkingTime) {
        let mut engine = self.engine();
        match choice {
            ThinkingTime::Quick => (engine.time_ms, engine.simulations) = (3_000, 20_000),
            ThinkingTime::Thorough => (engine.time_ms, engine.simulations) = (15_000, 100_000),
            ThinkingTime::NoLimit => {}
        }
        if engine != self.engine() {
            self.engine_changed(engine);
        }
        let mode = if choice == ThinkingTime::NoLimit {
            AnalysisMode::Continuous
        } else {
            AnalysisMode::Limited
        };
        self.analysis_controls(mode, false);
    }

    /// Whose turn it is, which roll it is, the five dice, and the face palette.
    fn phone_dice(&mut self, ui: &mut egui::Ui) {
        let position = &self.game.session().position;
        let phase = position.phase;
        let editor = self.game.position_editor();
        let roller = if editor && !position.hypothetical {
            position.infer_next_player()
        } else {
            Ok(Some(position.active_player))
        };
        // The headline, and whether it reports a problem.
        let headline = match roller {
            Ok(Some(player)) if editor && position.hypothetical => {
                (position.players[player].name.clone(), false)
            }
            Ok(Some(player)) => (is_up(&position.players[player].name), false),
            Ok(None) => (t("Game finished").to_string(), false),
            Err(_) => (t("The sheets do not fit together").to_string(), true),
        };
        ui.horizontal_wrapped(|ui| {
            ui.set_min_height(TARGET);
            match headline {
                (text, false) => {
                    ui.strong(text);
                }
                (text, true) => {
                    ui.colored_label(ui.visuals().error_fg_color, text);
                }
            }
            self.roll_selector(ui, phase, editor);
        });
        if editor && self.game.session().position.hypothetical {
            let name = &self.game.session().position.players
                [self.game.session().position.active_player]
                .name;
            let note = if *name == super::start::default_name(0) {
                t("Assumed: I am up").to_string()
            } else {
                tf("Assumed: {name} is up", &[("name", name)])
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(note);
                if big_button(ui, t("Determine automatically"), true) {
                    self.infer_roller();
                }
            });
        }
        let physical = self.game.physical_entry_allowed();
        let suggested = self.suggested_keeps();
        let spacing = 6.0;
        let tile = ((ui.available_width() - TARGET - 5.0 * spacing) / 5.0).clamp(TARGET, 64.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = spacing;
            for (i, hint) in suggested.into_iter().enumerate() {
                ui.push_id(("phone_die", i), |ui| {
                    ui.vertical(|ui| {
                        let dice = self.game.dice();
                        let (face, locked, keep) = (dice.faces[i], dice.locked[i], dice.keep[i]);
                        let selected = dice.selected == Some(i);
                        let enabled = !locked
                            && phase != Phase::Finished
                            && (self.game.editing() || self.game.control() == PlayerControl::Human);
                        let label = format!("{}", i + 1);
                        let response = ui
                            .add_enabled_ui(enabled, |ui| {
                                die_button(ui, face, tile, keep || locked, selected, label)
                            })
                            .inner;
                        if response.clicked() {
                            self.game.select_die(i, true);
                        }
                        let star = egui::RichText::new(if hint { "★" } else { " " }).small();
                        ui.add_sized([tile, 12.0], egui::Label::new(star));
                    });
                });
            }
            if physical && big_button(ui, egui::RichText::new("⬅").size(18.0), true) {
                self.clear_die(Instant::now());
            }
        });
        if physical {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for face in 1..=6 {
                    let response = ui
                        .push_id(("phone_face", face), |ui| {
                            die_button(ui, face, TARGET, false, false, face.to_string())
                        })
                        .inner;
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(("phone_face_rect", face)), response.rect)
                    });
                    if response.clicked() {
                        self.choose_face(face, Instant::now());
                    }
                }
            });
        }
        self.phone_play_buttons(ui, phase, physical);
        if self.dice_draft() && self.game.dice().faces.contains(&0) {
            ui.small(t("Enter all five dice for advice."));
        }
    }

    /// „1. 2. 3. Wurf“: which roll the entered dice are. Only editable in the position editor.
    fn roll_selector(&mut self, ui: &mut egui::Ui, phase: Phase, editor: bool) {
        let current = match phase {
            Phase::Decision { rerolls_left, .. } => 3 - rerolls_left.min(2),
            Phase::AwaitingRoll { rerolls_after, .. } => 2 - rerolls_after.min(1),
            _ => 1,
        };
        let editable = editor && matches!(phase, Phase::Decision { .. });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(t("roll"));
            for number in (1..=3_u8).rev() {
                let button = egui::Button::new(format!("{number}."))
                    .selected(number == current)
                    .min_size(egui::vec2(TARGET, TARGET));
                if ui.add_enabled(editable, button).clicked() && number != current {
                    let mut next = self.game.session().clone();
                    if let Phase::Decision { rerolls_left, .. } = &mut next.position.phase {
                        *rerolls_left = 3 - number;
                    }
                    self.commit(next);
                }
            }
        });
    }

    /// Games (not the position editor) roll digital dice or reroll the unkept ones here.
    fn phone_play_buttons(&mut self, ui: &mut egui::Ui, phase: Phase, physical: bool) {
        if self.game.editing() {
            if self.game.position_editor()
                && phase != Phase::Finished
                && (phase != Phase::BeforeFirstRoll || self.game.dice().faces != [0; 5])
                && big_button(ui, t("Forecast before the roll"), true)
            {
                let result = self.game.analyze_before_roll();
                self.outcome(result);
                self.refresh_revision();
            }
            return;
        }
        let can_play = self.game.human_can_play();
        match phase {
            Phase::BeforeFirstRoll | Phase::AwaitingRoll { .. } if !physical => {
                if big_button(ui, t("Roll the dice"), can_play) {
                    let result = self.game.roll_digital(Instant::now());
                    self.outcome(result);
                    self.refresh_revision();
                }
            }
            Phase::Decision { rerolls_left, .. } => {
                let count = self.game.dice().keep.iter().filter(|&&k| !k).count();
                let label = tf("Reroll {count} dice", &[("count", &count)]);
                if big_button(ui, label, can_play && rerolls_left > 0 && count > 0) {
                    let result = self.game.reroll_selected(Instant::now());
                    self.outcome(result);
                    self.refresh_revision();
                }
            }
            _ => {}
        }
    }

    /// The move phrase: „Behalte ⚃⚃ · würfle 3 neu“, „Dreierpasch eintragen: 18 Punkte“, …
    fn phone_move(&self, ui: &mut egui::Ui, action: Action) {
        let position = &self.game.session().position;
        match action {
            Action::Reroll { keep } if keep.is_empty() => {
                ui.strong(t("Reroll all five dice"));
            }
            Action::Reroll { keep } => {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(t("Keep"));
                    for face in keep.faces() {
                        small_die(ui, face);
                    }
                    let count = 5 - keep.len();
                    ui.strong(tf("· reroll {count}", &[("count", &count)]));
                });
            }
            Action::Write { category } | Action::CrossOut { category } => {
                let sheet = &position.players[position.active_player].sheet;
                let (points, extra) = match position.phase {
                    Phase::Decision { dice, .. } => (
                        rules::score(sheet, dice, category),
                        rules::extra_eligible(sheet, dice),
                    ),
                    _ => (0, false),
                };
                let name = category_name(category);
                let mut text = if points == 0 {
                    tf("Cross out {category} (0 points)", &[("category", &name)])
                } else {
                    tf(
                        "Score {category}: {points} points",
                        &[("category", &name), ("points", &points)],
                    )
                };
                if extra {
                    text.push(' ');
                    text.push_str(t("incl. 50 bonus points"));
                }
                ui.strong(text);
            }
            Action::RollAll => {
                ui.strong(t("Roll the dice"));
            }
        }
    }

    /// „Gewinnchance 58 % · Ø 241 Punkte“; alone, only the expected final score counts.
    fn phone_metrics(&self, ui: &mut egui::Ui, metrics: Option<Metrics>) {
        let Some(m) = metrics else {
            ui.small(t("Computing …"));
            return;
        };
        let points = tf(
            "Ø {points} points",
            &[("points", &decimal(m.expected_score, 0))],
        );
        if self.game.session().position.players.len() == 1 {
            ui.label(points);
        } else {
            let chance = tf(
                "Chance of winning {percent}",
                &[("percent", &percent(m.utility, 0))],
            );
            ui.label(format!("{chance} · {points}"));
        }
    }

    fn phone_advice(&mut self, ui: &mut egui::Ui) {
        if self.game.control() == PlayerControl::Computer && !self.game.editing() {
            ui.strong(t("The computer is playing …"));
            if let Some(action) = self.game.pending_computer_action() {
                self.phone_move(ui, action);
            }
            return;
        }
        if !self.analysis_eligible() {
            if self.game.position_editor()
                && !self.game.session().position.hypothetical
                && self.game.session().position.infer_next_player().is_err()
            {
                self.who_is_up_ui(ui);
            } else if self.game.session().position.phase == Phase::Finished {
                ui.label(t("Game finished"));
            } else if self.game.dice_source() == DiceSource::Digital {
                ui.label(t("Roll the dice for advice."));
            } else {
                ui.label(t("Enter all five dice for advice."));
            }
            return;
        }
        let Some(report) = self.report.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t("Computing …"));
            });
            return;
        };
        if self.before_roll_forecast() {
            ui.strong(t("Forecast before the roll"));
            let forecast = report.forecast.or_else(|| {
                report
                    .alternatives
                    .iter()
                    .find(|a| a.action == Action::RollAll)
                    .and_then(|a| a.metrics)
            });
            self.phone_metrics(ui, forecast);
            self.phone_status(ui, &report, false);
            self.technical_details(ui, &report);
            return;
        }
        let Some(best) = report.best_action else {
            if self.running.is_some() && !self.analysis_done && !self.analysis_paused {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t("Computing …"));
                });
            } else {
                // A finished analysis without a move must say so instead of spinning forever.
                ui.label(t("No suggestion for this position."));
                if report.forecast.is_some() {
                    self.phone_metrics(ui, report.forecast);
                }
            }
            self.technical_details(ui, &report);
            return;
        };
        let estimate = |action| report.alternatives.iter().find(|a| a.action == action);
        let mut play = None;
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(egui::Stroke::new(
                2.0_f32,
                ui.visuals().selection.stroke.color,
            ))
            .corner_radius(8)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.strong("★");
                    self.phone_move(ui, best);
                });
                self.phone_metrics(ui, estimate(best).and_then(|a| a.metrics));
                ui.horizontal(|ui| {
                    if self.move_button(ui, best, true) {
                        play = Some(best);
                    }
                    let others = report.alternatives.len().saturating_sub(1);
                    if others > 0 {
                        let label = format!("{} ⏷", t("Other moves"));
                        if ui
                            .add(
                                egui::Button::new(label)
                                    .selected(self.show_all_moves)
                                    .min_size(egui::vec2(TARGET, TARGET)),
                            )
                            .clicked()
                        {
                            self.show_all_moves = !self.show_all_moves;
                        }
                    }
                });
                self.phone_status(
                    ui,
                    &report,
                    close_call(&report, best, self.game.session().position.players.len()),
                );
            });
        if self.show_all_moves {
            for alternative in report.alternatives.iter().filter(|a| a.action != best) {
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(8)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.phone_move(ui, alternative.action);
                        self.phone_metrics(ui, alternative.metrics);
                        if self.move_button(ui, alternative.action, false) {
                            play = Some(alternative.action);
                        }
                    });
            }
        }
        self.technical_details(ui, &report);
        if let Some(action) = play {
            let result = self.apply(action);
            self.outcome(result);
        }
    }

    /// „Übernehmen“ in the position editor; in games the move is played.
    fn move_button(&self, ui: &mut egui::Ui, action: Action, recommended: bool) -> bool {
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
        let legal = self.game.session().position.actions().contains(&action);
        let response = ui.add_enabled(
            allowed && legal,
            egui::Button::new(label).min_size(egui::vec2(TARGET * 2.5, TARGET)),
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new(("phone_move", action)), response.rect));
        response.clicked()
    }

    /// „✔ exakt berechnet“, „✔ fertig · 6 400 Partien simuliert“, „rechnet · …“, „vorläufig …“.
    fn phone_status(&mut self, ui: &mut egui::Ui, report: &Report, close: bool) {
        let games = count(report.samples);
        ui.horizontal_wrapped(|ui| {
            if report.status == "exact_under_model" {
                ui.small(t("✔ calculated exactly"));
            } else if self.analysis_paused {
                ui.small(t("paused"));
            } else if self.running.is_some() && !self.analysis_done {
                if report.recommendation_basis.starts_with("baseline_") {
                    ui.small(t("provisional, still computing …"));
                } else {
                    ui.small(tf("computing · {count} games", &[("count", &games)]));
                }
                ui.spinner();
            } else {
                ui.small(tf("✔ done · {count} games simulated", &[("count", &games)]));
                if self.analysis_mode == AnalysisMode::Limited
                    && big_button(ui, t("Compute longer"), true)
                {
                    self.compute_longer();
                }
            }
        });
        if close {
            ui.small(t("Close call: other moves are almost as good"));
        }
    }

    /// Collapsed engine details, behind a 44-point toggle rather than a small disclosure line.
    fn technical_details(&self, ui: &mut egui::Ui, report: &Report) {
        let id = egui::Id::new("phone_details_open");
        let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
        let label = format!(
            "{} {}",
            if open { "⏷" } else { "⏵" },
            t("Technical details")
        );
        if ui
            .add(
                egui::Button::new(label)
                    .frame(false)
                    .min_size(egui::vec2(TARGET, TARGET)),
            )
            .clicked()
        {
            open = !open;
            ui.data_mut(|d| d.insert_temp(id, open));
        }
        if open {
            ui.small(format!(
                "{} · {} · {}",
                report.method,
                report.status.replace('_', " "),
                report.confidence.replace('_', " ")
            ));
            ui.small(tf(
                "{count} games · {seconds} s computing",
                &[
                    ("count", &count(report.samples)),
                    ("seconds", &decimal(report.elapsed_ms as f64 / 1000.0, 1)),
                ],
            ));
            if let Some(m) = report.forecast {
                ui.small(tf(
                    "Win {win} · tie {tie}",
                    &[
                        ("win", &percent(m.outright_win, 1)),
                        ("tie", &percent(m.tied_first, 1)),
                    ],
                ));
            }
            ui.small(&report.stop_reason);
            ui.small(&report.selection_reason);
        }
    }

    /// Short row names, players in columns. With more players than fit, the player columns
    /// scroll sideways while the row names stay.
    fn phone_sheet(&mut self, ui: &mut egui::Ui) {
        let position = &self.game.session().position;
        let players = position.players.len();
        let misfits = if self.game.position_editor()
            && !position.hypothetical
            && position.infer_next_player().is_err()
        {
            super::players::misfits(position)
        } else {
            vec![]
        };
        let row = |ui: &mut egui::Ui, text: &str, strong: bool| {
            ui.allocate_ui_with_layout(
                egui::vec2(LABEL_WIDTH, TARGET),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    if strong {
                        ui.strong(text);
                    } else {
                        ui.label(text);
                    }
                },
            );
        };
        ui.horizontal_top(|ui| {
            // Rows of 44-point cells with a small gap, so neighboring cells stay distinct.
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
            let gap = ui.spacing().item_spacing.x;
            let free = ui.available_width() - LABEL_WIDTH - gap;
            let width = ((free - gap * (players - 1) as f32) / players as f32)
                .clamp(COLUMN_WIDTH, WIDE_COLUMN);
            ui.vertical(|ui| {
                ui.set_width(LABEL_WIDTH);
                row(ui, "", false);
                for category in 0..13 {
                    row(ui, category_short(category), false);
                    if category == 5 {
                        row(ui, t("Bonus"), true);
                    }
                }
                row(ui, t("Total"), true);
            });
            egui::ScrollArea::horizontal()
                .id_salt("phone_sheet_columns")
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        for player in 0..players {
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                self.phone_column(ui, player, width, misfits.contains(&player));
                            });
                        }
                    });
                });
        });
    }

    fn phone_column(&mut self, ui: &mut egui::Ui, player: usize, width: f32, misfit: bool) {
        let position = self.game.session().position.clone();
        let sheet = position.players[player].sheet;
        let rolling = player == position.active_player && position.phase != Phase::Finished;
        let cell = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.allocate_ui_with_layout(
                egui::vec2(width, TARGET),
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| add(ui),
            );
        };
        let mut open_sheet = false;
        cell(ui, &mut |ui| {
            let name = egui::RichText::new(&position.players[player].name).strong();
            let name = if misfit {
                name.color(ui.visuals().error_fg_color)
            } else if rolling {
                name.background_color(ui.visuals().selection.bg_fill)
            } else {
                name
            };
            // In the position editor, a name opens the player sheet.
            if self.game.position_editor() {
                let response = ui.add(egui::Button::new(name).frame(false).truncate());
                #[cfg(test)]
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new(("phone_name", player)), response.rect)
                });
                open_sheet = response.clicked();
            } else {
                ui.add(egui::Label::new(name).truncate());
            }
        });
        if open_sheet {
            self.open_player_sheet(player);
        }
        let ready = self.game.human_can_play();
        let mut action = None;
        for category in 0..13 {
            let value = sheet.cells[category];
            cell(ui, &mut |ui| {
                if self.game.editing() {
                    let text = value.map_or_else(|| "–".to_string(), |v| v.to_string());
                    let response = ui.add(egui::Button::new(text));
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(
                            egui::Id::new(("phone_cell", player, category)),
                            response.rect,
                        )
                    });
                    if response.clicked() {
                        self.edit = Some((player, category));
                    }
                } else if let Some(value) = value {
                    ui.label(value.to_string());
                } else if rolling && let Phase::Decision { dice, .. } = position.phase {
                    let points = rules::score(&sheet, dice, category);
                    if ui
                        .add_enabled(ready, egui::Button::new(points.to_string()))
                        .clicked()
                    {
                        action = Some(if points == 0 {
                            Action::CrossOut { category }
                        } else {
                            Action::Write { category }
                        });
                    }
                } else {
                    ui.weak("–");
                }
            });
            if category == 5 {
                cell(ui, &mut |ui| {
                    if sheet.bonus() > 0 {
                        ui.strong("+35");
                    } else {
                        ui.small(format!("{}/63", sheet.upper()));
                    }
                });
            }
        }
        cell(ui, &mut |ui| {
            ui.strong(sheet.total().to_string());
        });
        if let Some(action) = action {
            let result = self.apply(action);
            self.outcome(result);
        }
    }
}

/// Whether the second-best move is within one percentage point (or, alone, one point).
fn close_call(report: &Report, best: Action, players: usize) -> bool {
    let value = |a: &Alternative| {
        a.metrics.map(|m| {
            if players == 1 {
                m.expected_score / 100.0
            } else {
                m.utility
            }
        })
    };
    let Some(best_value) = report
        .alternatives
        .iter()
        .find(|a| a.action == best)
        .and_then(value)
    else {
        return false;
    };
    report
        .alternatives
        .iter()
        .filter(|a| a.action != best)
        .filter_map(value)
        .any(|other| (best_value - other).abs() < 0.01)
}
