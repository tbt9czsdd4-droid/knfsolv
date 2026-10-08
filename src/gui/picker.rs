//! The score picker: tap a cell, then tap its value. Only legal values are offered, so nothing
//! needs typing; one tap applies the value and closes the picker, and Undo reverts it. In games,
//! the entry choice offers what the roll allows in one cell beyond its full score: crossing out,
//! or fewer dice in the upper section.
use super::{
    Desktop,
    i18n::{category_name, t, tf},
};
use crate::{rules, state::Phase};
use eframe::egui;
use web_time::Instant;

/// Large enough to hit with a finger.
const TARGET: f32 = 44.0;
const COLUMNS: usize = 6;

/// The nonzero scores a category can hold; zero is offered separately as crossing out.
pub(super) fn values(category: usize) -> impl Iterator<Item = u16> {
    rules::category_scores(category)
        .iter()
        .copied()
        .filter(|&v| v != 0)
}

/// One picker button; tests find it by `key` (a value or a button name) instead of its label,
/// which the sheet behind the picker may show too.
fn choice(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    label: String,
    selected: bool,
    key: impl std::hash::Hash,
) -> bool {
    let response = ui.add_sized(size, egui::Button::new(label).selected(selected));
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new(("picker", key)), response.rect));
    #[cfg(not(test))]
    let _ = key;
    response.clicked()
}

impl Desktop {
    /// Points the active player's current roll would score in `category`, if dice are complete.
    fn current_roll_points(&self, player: usize, category: usize) -> Option<u16> {
        let position = &self.game.session().position;
        match position.phase {
            Phase::Decision { dice, .. } if player == position.active_player => Some(rules::score(
                &position.players[player].sheet,
                dice,
                category,
            )),
            _ => None,
        }
    }

    pub(super) fn picker_ui(&mut self, ctx: &egui::Context) {
        let Some((player, category)) = self.edit else {
            return;
        };
        let current = self.game.session().position.players[player].sheet.cells[category];
        let roll = self.current_roll_points(player, category);
        let title = format!(
            "{} · {}",
            self.game.session().position.players[player].name,
            category_name(category)
        );
        let mut chosen = None;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Window::new(title)
            .id(egui::Id::new("score_picker"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                // Six 44-point columns must fit 310-point-wide screens (iPhone, large text).
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                let wide = egui::vec2(
                    COLUMNS as f32 * TARGET + (COLUMNS - 1) as f32 * ui.spacing().item_spacing.x,
                    TARGET,
                );
                if let Some(points) = roll {
                    let label = tf("{points} with the current roll", &[("points", &points)]);
                    if choice(ui, wide, label, current == Some(points), "roll") {
                        chosen = Some(Some(points));
                    }
                }
                ui.horizontal(|ui| {
                    let half = egui::vec2((wide.x - ui.spacing().item_spacing.x) / 2.0, TARGET);
                    if choice(ui, half, t("Open").into(), current.is_none(), "open") {
                        chosen = Some(None);
                    }
                    let crossed = current == Some(0);
                    if choice(ui, half, t("Cross out (0)").into(), crossed, "cross_out") {
                        chosen = Some(Some(0));
                    }
                });
                egui::Grid::new("score_picker_values").show(ui, |ui| {
                    for (i, value) in values(category).enumerate() {
                        let size = egui::vec2(TARGET, TARGET);
                        if choice(ui, size, value.to_string(), current == Some(value), value) {
                            chosen = Some(Some(value));
                        }
                        if (i + 1) % COLUMNS == 0 {
                            ui.end_row();
                        }
                    }
                });
                if choice(ui, wide, t("Cancel").into(), false, "cancel") {
                    close = true;
                }
            });
        if let Some(value) = chosen {
            let mut next = self.game.session().clone();
            next.position.players[player].sheet.cells[category] = value;
            let result = next.position.players[player].sheet.validate();
            if result.is_ok() {
                self.commit(next);
            }
            self.outcome(result);
            close = true;
        }
        if close {
            self.edit = None;
        }
    }

    /// Every entry the current roll allows in `entry_choice`, highest first, in a game.
    pub(super) fn entry_ui(&mut self, ctx: &egui::Context) {
        let Some(category) = self.entry_choice else {
            return;
        };
        let position = &self.game.session().position;
        let player = position.active_player;
        let sheet = position.players[player].sheet;
        let Phase::Decision { dice, .. } = position.phase else {
            self.entry_choice = None;
            return;
        };
        if sheet.cells[category].is_some() || !self.game.human_can_play() {
            self.entry_choice = None;
            return;
        }
        let title = format!(
            "{} · {}",
            position.players[player].name,
            category_name(category)
        );
        let mut chosen = None;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Window::new(title)
            .id(egui::Id::new("entry_choice"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                let wide = egui::vec2(
                    COLUMNS as f32 * TARGET + (COLUMNS - 1) as f32 * ui.spacing().item_spacing.x,
                    TARGET,
                );
                for points in rules::entry_points(&sheet, dice, category) {
                    let label = match points {
                        0 => t("Cross out (0)").to_string(),
                        1 => t("1 point").to_string(),
                        _ => tf("{points} points", &[("points", &points)]),
                    };
                    if choice(ui, wide, label, false, ("entry", points)) {
                        chosen = Some(points);
                    }
                }
                if choice(ui, wide, t("Cancel").into(), false, "cancel") {
                    close = true;
                }
            });
        if let Some(points) = chosen {
            let result = self.game.play_entry(category, points, Instant::now());
            self.refresh_revision();
            if result.is_ok() {
                self.message = t("Move applied.").into();
            }
            self.outcome(result);
            close = true;
        }
        if close {
            self.entry_choice = None;
        }
    }
}
