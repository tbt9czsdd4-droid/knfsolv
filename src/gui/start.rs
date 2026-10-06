//! The phone's start screens: a one-page welcome with the ways in, and „Wer spielt mit?“, which
//! asks for the players in playing order and opens the analyzer with empty sheets. Names are
//! optional; empty ones become „Ich“, „Spieler 2“, and so on.
use super::{
    Desktop, Pending, Screen,
    i18n::{t, tf},
};
use eframe::egui;

const TARGET: f32 = 44.0;

fn wide_button(ui: &mut egui::Ui, label: &str, primary: bool) -> bool {
    let text = if primary {
        egui::RichText::new(label).strong()
    } else {
        egui::RichText::new(label)
    };
    let mut button = egui::Button::new(text).min_size(egui::vec2(ui.available_width(), TARGET));
    if primary {
        button = button.fill(ui.visuals().selection.bg_fill);
    }
    ui.add(button).clicked()
}

/// The name a player gets when the field is left empty.
pub(super) fn default_name(index: usize) -> String {
    if index == 0 {
        t("Me").to_string()
    } else {
        tf("Player {number}", &[("number", &(index + 1))])
    }
}

impl Desktop {
    pub(super) fn phone_start_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("phone_start")
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 10.0;
                ui.add_space(12.0);
                ui.heading("Kniffel-Coach");
                ui.label(t(
                    "Copy the score sheets, tap in the dice, and see the best move with its chance of winning.",
                ));
                ui.add_space(8.0);
                if self.session_active && wide_button(ui, t("Back to the position"), true) {
                    self.navigate(Screen::Play);
                }
                if wide_button(ui, t("Analyze a position"), !self.session_active) {
                    self.navigate(Screen::AnalysisSetup);
                }
                if let Some(browser) = &self.browser
                    && wide_button(ui, t("Open position"), false)
                {
                    browser.open();
                }
                ui.add_space(8.0);
                ui.label(t("Play instead of analyzing:"));
                if wide_button(ui, t("Local game"), false) {
                    self.navigate(Screen::LocalSetup);
                }
                if wide_button(ui, t("Against the computer"), false) {
                    self.navigate(Screen::ComputerSetup);
                }
                if !self.message.is_empty() {
                    ui.small(&self.message);
                }
            });
    }

    /// „Wer spielt mit?“: player count and optional names, in playing order.
    pub(super) fn phone_players_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("phone_players")
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.add_space(12.0);
                ui.heading(t("Who is playing?"));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for n in 1..=8 {
                        let button = egui::Button::new(n.to_string())
                            .selected(self.analysis_players == n)
                            .min_size(egui::vec2(TARGET, TARGET));
                        if ui.add(button).clicked() {
                            self.analysis_players = n;
                        }
                    }
                });
                ui.small(t(
                    "In playing order, starting with whoever began. Names are optional.",
                ));
                self.setup_names
                    .resize(self.analysis_players, String::new());
                for (i, name) in self.setup_names.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}.", i + 1));
                        ui.add(
                            egui::TextEdit::singleline(name)
                                .id_salt(("setup_player", i))
                                .hint_text(default_name(i))
                                .char_limit(30)
                                .desired_width(ui.available_width())
                                .margin(egui::vec2(8.0, 12.0)),
                        );
                    });
                }
                ui.add_space(8.0);
                if wide_button(ui, t("Start"), true) {
                    let names = self
                        .setup_names
                        .iter()
                        .enumerate()
                        .map(|(i, name)| {
                            let name = name.trim();
                            if name.is_empty() {
                                default_name(i)
                            } else {
                                name.to_string()
                            }
                        })
                        .collect();
                    self.request(Pending::AnalyzeNamed(names));
                }
                if wide_button(ui, t("Back"), false) {
                    self.navigate(Screen::Home);
                }
            });
    }
}
