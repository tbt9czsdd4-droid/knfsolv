//! The phone's start screens: a one-page welcome with the ways in; „Wer spielt mit?“, which asks
//! for the players in playing order and opens the analyzer with empty sheets; and the setups of a
//! local game and a game against the computer. Names are optional everywhere and shared between
//! the screens; empty ones become „Ich“, „Spieler 2“, „Du“, and so on.
use super::{
    Desktop, Pending, Screen,
    i18n::{t, tf},
    policy_label,
};
use crate::game::{ComputerPolicy, DiceSource};
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
        player_number(index)
    }
}

/// „Spieler 3“ for index 2.
pub(super) fn player_number(index: usize) -> String {
    tf("Player {number}", &[("number", &(index + 1))])
}

/// The typed name, or `default` for an empty field.
fn name_or(name: &str, default: String) -> String {
    let name = name.trim();
    if name.is_empty() {
        default
    } else {
        name.to_string()
    }
}

/// Equally wide 44-point buttons in one row; returns the tapped index.
fn segmented(ui: &mut egui::Ui, labels: &[&str], selected: Option<usize>) -> Option<usize> {
    let spacing = 4.0;
    let width = (ui.available_width() - spacing * (labels.len() - 1) as f32) / labels.len() as f32;
    let mut tapped = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        for (i, label) in labels.iter().enumerate() {
            let button = egui::Button::new(*label)
                .selected(selected == Some(i))
                .min_size(egui::vec2(width, TARGET));
            if ui.add(button).clicked() {
                tapped = Some(i);
            }
        }
    });
    tapped
}

/// One optional name field per player, with the default as hint.
fn name_fields(ui: &mut egui::Ui, names: &mut [String], default: impl Fn(usize) -> String) {
    for (i, name) in names.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            // Sized like the field, so the number sits at the text's height.
            ui.add_sized([18.0, TARGET], egui::Label::new(format!("{}.", i + 1)));
            ui.add(
                egui::TextEdit::singleline(name)
                    .id_salt(("setup_player", i))
                    .hint_text(default(i))
                    .char_limit(30)
                    .desired_width(ui.available_width())
                    .margin(super::touch_field_margin(ui)),
            );
        });
    }
}

fn dice_choice(ui: &mut egui::Ui, source: &mut DiceSource) {
    let sources = [DiceSource::Physical, DiceSource::Digital];
    let labels = [t("Real dice"), t("The app rolls")];
    if let Some(i) = segmented(ui, &labels, sources.iter().position(|s| s == source)) {
        *source = sources[i];
    }
}

impl Desktop {
    pub(super) fn phone_start_ui(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().interaction.selectable_labels = false;
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
                ui.add_space(8.0);
                if wide_button(ui, t("Help & rules"), false) {
                    self.show_help = true;
                }
                if !self.message.is_empty() {
                    ui.small(&self.message);
                }
            });
    }

    /// „Wer spielt mit?“: player count and optional names, in playing order.
    pub(super) fn phone_players_ui(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().interaction.selectable_labels = false;
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
                name_fields(ui, &mut self.setup_names, default_name);
                ui.add_space(8.0);
                if wide_button(ui, t("Start"), true) {
                    let names = self
                        .setup_names
                        .iter()
                        .enumerate()
                        .map(|(i, name)| name_or(name, default_name(i)))
                        .collect();
                    self.request(Pending::AnalyzeNamed(names));
                }
                if wide_button(ui, t("Back"), false) {
                    self.navigate(Screen::Home);
                }
            });
    }

    /// A local game: 2–8 people share the phone and one score sheet.
    pub(super) fn phone_local_ui(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().interaction.selectable_labels = false;
        egui::ScrollArea::vertical()
            .id_salt("phone_local")
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.add_space(12.0);
                ui.heading(t("Local game"));
                ui.label(t("How many are playing?"));
                let mut count = self.local_setup.names.len();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for n in 2..=8 {
                        let button = egui::Button::new(n.to_string())
                            .selected(count == n)
                            .min_size(egui::vec2(TARGET, TARGET));
                        if ui.add(button).clicked() {
                            count = n;
                        }
                    }
                });
                let setup = &mut self.local_setup;
                for i in setup.names.len()..count {
                    setup.names.push(player_number(i));
                }
                setup.names.truncate(count);
                setup.starting_player = setup.starting_player.min(count - 1);
                ui.small(t("In playing order. Names are optional."));
                self.setup_names.resize(count, String::new());
                name_fields(ui, &mut self.setup_names, player_number);
                let names: Vec<String> = self
                    .setup_names
                    .iter()
                    .enumerate()
                    .map(|(i, name)| name_or(name, player_number(i)))
                    .collect();
                ui.label(t("Who starts?"));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (i, name) in names.iter().enumerate() {
                        let button = egui::Button::new(name.as_str())
                            .selected(self.local_setup.starting_player == i)
                            .min_size(egui::vec2(TARGET * 2.0, TARGET));
                        if ui.add(button).clicked() {
                            self.local_setup.starting_player = i;
                        }
                    }
                });
                ui.label(t("Dice"));
                dice_choice(ui, &mut self.local_setup.dice_source);
                ui.small(match self.local_setup.dice_source {
                    DiceSource::Physical => t("Roll real dice and tap in their faces."),
                    DiceSource::Digital => t("Tap “Roll the dice” and the app rolls for you."),
                });
                ui.add_space(8.0);
                if wide_button(ui, t("Start"), true) {
                    self.local_setup.names = names;
                    self.request(Pending::Local(self.local_setup.clone()));
                }
                if wide_button(ui, t("Back"), false) {
                    self.navigate(Screen::Home);
                }
            });
    }

    /// A game against the computer, with the phone owner's name shared with the other screens.
    pub(super) fn phone_computer_ui(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().interaction.selectable_labels = false;
        egui::ScrollArea::vertical()
            .id_salt("phone_computer")
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.add_space(12.0);
                ui.heading(t("Against the computer"));
                ui.label(t("Your name"));
                if self.setup_names.is_empty() {
                    self.setup_names.push(String::new());
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.setup_names[0])
                        .id_salt(("setup_player", 0))
                        .hint_text(t("You"))
                        .char_limit(30)
                        .desired_width(ui.available_width())
                        .margin(super::touch_field_margin(ui)),
                );
                let setup = &mut self.computer_setup;
                ui.label(t("Who starts?"));
                let starts = [t("You"), setup.computer_name.as_str()];
                if let Some(i) = segmented(ui, &starts, Some(usize::from(!setup.human_starts))) {
                    setup.human_starts = i == 0;
                }
                ui.label(t("Your dice"));
                dice_choice(ui, &mut setup.dice_source);
                ui.small(t("The app always rolls for the computer."));
                ui.label(t("Strength"));
                let policies = [
                    ComputerPolicy::Greedy,
                    ComputerPolicy::LegacyOpportunity,
                    ComputerPolicy::ContinuationV2,
                ];
                let labels = policies.map(policy_label);
                let current = (!self.custom_search)
                    .then(|| policies.iter().position(|&p| p == setup.computer.policy))
                    .flatten();
                if let Some(i) = segmented(ui, &labels, current) {
                    self.custom_search = false;
                    setup.computer.policy = policies[i];
                    setup.computer.search = None;
                }
                if self.custom_search {
                    ui.small(t("Custom search, set on a larger screen."));
                }
                ui.add_space(8.0);
                if wide_button(ui, t("Start"), true) {
                    let setup = &mut self.computer_setup;
                    setup.human_name = name_or(&self.setup_names[0], t("You").to_string());
                    if self.custom_search {
                        setup.computer.policy = ComputerPolicy::ContinuationV2;
                        setup.computer.search = Some(self.custom_settings.clone());
                    }
                    self.request(Pending::Computer(self.computer_setup.clone()));
                }
                if wide_button(ui, t("Back"), false) {
                    self.navigate(Screen::Home);
                }
            });
    }
}
