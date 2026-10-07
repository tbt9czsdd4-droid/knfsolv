//! Players in the position editor: the player sheet (name, Zusatz-Kniffel count, place in the
//! turn order, applied together as one Undo step) and „Wer ist dran?“ for sheets whose entry
//! counts do not fit the turn order.
use super::{
    Desktop,
    i18n::{t, tf},
};
use crate::{Result, state::Position};
use eframe::egui;

const TARGET: f32 = 44.0;

/// The player sheet's changes, applied only by „Fertig“.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlayerDraft {
    pub(super) player: usize,
    pub(super) name: String,
    pub(super) extra_kniffels: u8,
    /// Index in the turn order.
    pub(super) place: usize,
}

/// Players whose number of entries probably does not fit: those that differ from the most
/// common count or, without a single most common count, those with fewer entries than a
/// player after them in the turn order (an entry is likely missing there).
pub(super) fn misfits(position: &Position) -> Vec<usize> {
    let counts: Vec<usize> = position
        .players
        .iter()
        .map(|p| 13 - p.sheet.open())
        .collect();
    let frequency = |count: usize| counts.iter().filter(|&&c| c == count).count();
    let top = counts.iter().map(|&c| frequency(c)).max().unwrap_or(0);
    let mut modes: Vec<usize> = counts
        .iter()
        .copied()
        .filter(|&c| frequency(c) == top)
        .collect();
    modes.sort_unstable();
    modes.dedup();
    if let [mode] = modes[..] {
        return (0..counts.len()).filter(|&p| counts[p] != mode).collect();
    }
    let order = &position.turn_order;
    order
        .iter()
        .enumerate()
        .filter(|&(i, &p)| order[i + 1..].iter().any(|&q| counts[q] > counts[p]))
        .map(|(_, &p)| p)
        .collect()
}

fn wide_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(label).min_size(egui::vec2(TARGET * 2.0, TARGET)),
    )
    .clicked()
}

impl Desktop {
    pub(super) fn open_player_sheet(&mut self, player: usize) {
        let position = &self.game.session().position;
        self.player_sheet = Some(PlayerDraft {
            player,
            name: position.players[player].name.clone(),
            extra_kniffels: position.players[player].sheet.extra_kniffels,
            place: position
                .turn_order
                .iter()
                .position(|&p| p == player)
                .unwrap_or(0),
        });
    }

    pub(super) fn player_sheet_ui(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.player_sheet.clone() else {
            return;
        };
        let position = &self.game.session().position;
        let players = position.players.len();
        let original = position.players[draft.player].name.clone();
        let mut sheet = position.players[draft.player].sheet;
        sheet.extra_kniffels = draft.extra_kniffels;
        let valid = sheet.validate().is_ok();
        let mut apply = false;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        // A fixed width keeps the separator from stretching the window to the screen's edges;
        // on narrow screens it leaves a gutter of about 16 points on each side.
        let width = (ctx.content_rect().width() - 48.0).clamp(200.0, 280.0);
        egui::Window::new(t("Player"))
            .id(egui::Id::new("player_sheet"))
            .collapsible(false)
            .resizable(false)
            .min_width(width)
            .max_width(width)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                ui.label(t("Name"));
                let mut field = egui::TextEdit::singleline(&mut draft.name)
                    .char_limit(30)
                    .desired_width(width)
                    .margin(super::touch_field_margin(ui))
                    .show(ui);
                // Until the name is changed, it stays selected, so typing replaces a default
                // like "Player 2" (a tap's release would otherwise place the cursor).
                let all = egui::text_selection::CCursorRange::select_all(&field.galley);
                if field.response.has_focus()
                    && draft.name == original
                    && field.state.cursor.char_range() != Some(all)
                {
                    field.state.cursor.set_char_range(Some(all));
                    field.state.store(ui.ctx(), field.response.id);
                }
                ui.horizontal(|ui| {
                    ui.label(t("Extra Kniffels"));
                    ui.small(t("50 points each"));
                });
                ui.horizontal(|ui| {
                    if wide_button(ui, "−", draft.extra_kniffels > 0) {
                        draft.extra_kniffels -= 1;
                    }
                    ui.strong(draft.extra_kniffels.to_string());
                    if wide_button(ui, "+", draft.extra_kniffels < 12) {
                        draft.extra_kniffels += 1;
                    }
                });
                if !valid {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        t("Extra Kniffels need 50 points in Kniffel and enough filled rows."),
                    );
                }
                // The place is part of the heading, so the buttons fit next to each other even
                // in German on a 312-point screen.
                ui.horizontal(|ui| {
                    ui.label(t("Turn order"));
                    ui.strong(tf(
                        "Place {place} of {count}",
                        &[("place", &(draft.place + 1)), ("count", &players)],
                    ));
                });
                ui.horizontal(|ui| {
                    if wide_button(ui, t("Earlier"), draft.place > 0) {
                        draft.place -= 1;
                    }
                    if wide_button(ui, t("Later"), draft.place + 1 < players) {
                        draft.place += 1;
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    apply = wide_button(ui, t("Done"), valid);
                    if wide_button(ui, t("Cancel"), true) {
                        close = true;
                    }
                });
            });
        if apply {
            let result = self.apply_player_draft(&draft);
            close = result.is_ok();
            self.outcome(result);
        }
        self.player_sheet = (!close).then_some(draft);
    }

    fn apply_player_draft(&mut self, draft: &PlayerDraft) -> Result<()> {
        let mut next = self.game.session().clone();
        let player = &mut next.position.players[draft.player];
        let name = draft.name.trim();
        if !name.is_empty() {
            player.name = name.into();
        }
        player.sheet.extra_kniffels = draft.extra_kniffels;
        player.sheet.validate()?;
        let order = &mut next.position.turn_order;
        order.retain(|&p| p != draft.player);
        order.insert(draft.place.min(order.len()), draft.player);
        self.commit(next);
        Ok(())
    }

    /// „Wer ist dran?“: analyze anyway, as a hypothetical position with this player rolling.
    pub(super) fn assume_roller(&mut self, player: usize) {
        let mut next = self.game.session().clone();
        next.position.hypothetical = true;
        next.position.active_player = player;
        next.settings.perspective = player;
        self.commit(next);
    }

    /// Back to inferring the roller from the sheets.
    pub(super) fn infer_roller(&mut self) {
        let mut next = self.game.session().clone();
        next.position.hypothetical = false;
        self.commit(next);
    }

    /// Shown instead of advice while the sheets do not fit together.
    pub(super) fn who_is_up_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(t(
            "The number of entries does not fit the turn order – is an entry missing somewhere?",
        ));
        ui.strong(t("Analyze anyway – who is up?"));
        let position = self.game.session().position.clone();
        ui.horizontal_wrapped(|ui| {
            for &player in &position.turn_order {
                if wide_button(ui, &position.players[player].name, true) {
                    self.assume_roller(player);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(entries: &[usize], order: &[usize]) -> Position {
        let mut position = Position::new(entries.len());
        for (player, &n) in position.players.iter_mut().zip(entries) {
            for cell in player.sheet.cells.iter_mut().take(n) {
                *cell = Some(0);
            }
        }
        position.turn_order = order.to_vec();
        position
    }

    #[test]
    fn misfits_point_at_the_likely_mistake() {
        assert_eq!(misfits(&counts(&[3, 2, 3], &[0, 1, 2])), [1]);
        assert_eq!(misfits(&counts(&[2, 3, 2], &[0, 1, 2])), [1]);
        assert_eq!(misfits(&counts(&[3, 3, 1], &[0, 1, 2])), [2]);
        // No single most common count: players behind a later player are flagged.
        assert_eq!(misfits(&counts(&[1, 3], &[0, 1])), [0]);
        assert_eq!(misfits(&counts(&[2, 2, 3, 3], &[0, 1, 2, 3])), [0, 1]);
    }
}
