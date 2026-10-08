//! „Hilfe & Regeln“: how to use the app, what its numbers mean, the rules with the house
//! conventions of [rules.md](../../docs/rules.md), and in the browser a QR code of the app's
//! address. The prose is written out per language instead of going through `t()`.
use super::{
    Desktop,
    i18n::{Language, language, t},
};
use eframe::egui;

/// A heading and its paragraphs; paragraphs starting with "• " are list items.
type Section = (&'static str, &'static [&'static str]);

const GERMAN: &[Section] = &[
    (
        "So geht's",
        &[
            "• Spielblätter abschreiben: Tippe auf ein Feld und wähle den Wert. Offen heißt noch frei, 0 heißt gestrichen. Wer dran ist, ergibt sich aus der Zahl der Einträge.",
            "• Wurf eingeben: Wähle „1.“, „2.“ oder „3.“ Wurf und tippe die fünf Augenzahlen an. Nach ein paar Sekunden steht oben der beste Zug mit seiner Gewinnchance.",
            "• „Übernehmen“: Beim Neuwürfeln bleiben die behaltenen Würfel fest, du gibst nur die neuen ein. Beim Eintragen kommt der Wert aufs Blatt, und der Nächste ist dran.",
            "• Tippe auf einen Namen, um ihn zu ändern, Zusatz-Kniffel einzutragen oder die Reihenfolge anzupassen.",
            "• ↺ und ↻ nehmen Schritte zurück oder wiederholen sie. Die Stellung wird von selbst gespeichert.",
        ],
    ),
    (
        "Was die Zahlen bedeuten",
        &[
            "• Gewinnchance: wie oft du in vielen durchgespielten Partien gewinnst; ein Gleichstand zählt anteilig.",
            "• Ø Punkte: die erwartete Endpunktzahl. Spielst du allein, sucht die App den Zug mit den meisten Punkten.",
            "• „Knappe Sache“: Andere Züge sind fast gleich gut, es kommt kaum darauf an.",
            "• Die App rät vorsichtig: Sie weicht nur von einer bewährten Grundstrategie ab, wenn ein anderer Zug in den Simulationen klar vorn liegt. Unter „Andere Züge“ siehst du alle Schätzungen.",
        ],
    ),
    (
        "Regeln",
        &[
            "Jeder Zug beginnt mit fünf Würfeln; du darfst bis zu zweimal beliebige Würfel neu würfeln. Danach trägst du in ein freies Feld ein. Nach 13 Runden ist das Spiel vorbei.",
            "• Einser bis Sechser: die Summe dieser Augen. Ab 63 Punkten oben gibt es 35 Bonus.",
            "• Dreierpasch, Viererpasch, Chance: die Summe aller Würfel.",
            "• Full House (drei gleiche und ein Paar) 25, Kleine Straße (vier in Folge) 30, Große Straße (fünf in Folge) 40, Kniffel (fünf gleiche) 50.",
            "• Passt der Wurf in kein gewünschtes Feld, wird ein Feld gestrichen (0).",
        ],
    ),
    (
        "Unsere Hausregeln",
        &[
            "• Zusatz-Kniffel gibt es nur, wenn im Kniffel-Feld schon 50 stehen. Er bringt 50 Zusatzpunkte, und der Wurf zählt in jedem freien Feld voll: oben fünfmal die Augenzahl, Dreierpasch, Viererpasch und Chance 30, Full House 25, Straßen 30 und 40. Wurde Kniffel gestrichen, gibt es keine Zusatz-Kniffel.",
            "• Welcher Eintrag, entscheidest du: Streichen geht immer, auch wenn der Wurf passt, und oben darfst du weniger Würfel zählen (fünf Einser auch als 1 bis 4). Die App schlägt das nie vor, weil es nie mehr Punkte bringt. Im Spiel: Feld lange antippen.",
            "• Fünf gleiche Würfel zählen auch als Full House, mit den üblichen 25 Punkten.",
            "• Eine Große Straße zählt auch als Kleine Straße.",
        ],
    ),
];

const ENGLISH: &[Section] = &[
    (
        "How to use it",
        &[
            "• Copy the score sheets: tap a cell and choose its value. Open means free, 0 means crossed out. Who is up follows from the number of entries.",
            "• Enter the roll: choose the 1st, 2nd, or 3rd roll and tap the five faces. After a few seconds the best move appears with its chance of winning.",
            "• Apply: for a reroll, the kept dice are locked and only the new ones are entered. For a score, the value goes on the sheet and the next player is up.",
            "• Tap a name to rename the player, enter extra Kniffels, or change the turn order.",
            "• ↺ and ↻ undo and redo steps. The position is saved automatically.",
        ],
    ),
    (
        "What the numbers mean",
        &[
            "• Chance of winning: how often you win in many played-out games; a tie counts in part.",
            "• Ø points: the expected final score. Playing alone, the app looks for the move with the most points.",
            "• Close call: other moves are almost as good, so it hardly matters.",
            "• The advice is cautious: it only leaves a proven baseline strategy when another move is clearly ahead in the simulations. Other moves shows all estimates.",
        ],
    ),
    (
        "Rules",
        &[
            "Each turn starts with five dice; any dice may be rerolled up to twice. Then a free category is scored. The game ends after 13 rounds.",
            "• Ones to Sixes: the sum of those faces. 63 or more in the upper section earns a 35-point bonus.",
            "• Three of a kind, Four of a kind, Chance: the sum of all dice.",
            "• Full House (three of a kind and a pair) 25, Small Straight (four in a row) 30, Large Straight (five in a row) 40, Kniffel (five of a kind) 50.",
            "• When the roll fits no wanted category, one is crossed out (0).",
        ],
    ),
    (
        "House conventions",
        &[
            "• An extra Kniffel only counts with 50 already in the Kniffel row. It earns 50 extra points, and the roll scores in full in any open category: upper rows five times the face, Three and Four of a kind and Chance 30, Full House 25, straights 30 and 40. With Kniffel crossed out, there are no extra Kniffels.",
            "• The entry is your choice: crossing out is always allowed, also where the roll fits, and the upper section may count fewer dice (five ones also as 1 to 4). The app never suggests it, since it never scores more. In a game: long-press the cell.",
            "• Five of a kind also counts as a Full House, for the usual 25 points.",
            "• A Large Straight also counts as a Small Straight.",
        ],
    ),
];

impl Desktop {
    pub(super) fn help_ui(&mut self, ui: &mut egui::Ui) {
        let sections = match language() {
            Language::English => ENGLISH,
            Language::German => GERMAN,
        };
        for (heading, paragraphs) in sections {
            ui.add_space(6.0);
            ui.strong(*heading);
            for paragraph in *paragraphs {
                ui.label(*paragraph);
            }
        }
        // Only the browser has an address to pass on.
        let Some(url) = self.browser.as_ref().and_then(|b| b.app_url()) else {
            return;
        };
        ui.add_space(6.0);
        ui.strong(t("Share the app"));
        ui.label(t(
            "Let others scan this code with their camera. In Safari, they tap Share ⬆ and then “Add to Home Screen”.",
        ));
        qr_code(ui, &url);
        ui.small(&url);
        ui.small(t(
            "Safari and the Home Screen app keep separate positions; use the app from the Home Screen.",
        ));
    }

    /// „Hilfe & Regeln“ as a page of its own on the phone.
    pub(super) fn phone_help_ui(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal(|ui| {
            ui.set_min_height(44.0);
            let back = egui::Button::new(t("‹ Back")).min_size(egui::vec2(44.0, 44.0));
            if ui.add(back).clicked() {
                self.show_help = false;
            }
            ui.strong(t("Help & rules"));
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("phone_help")
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.help_ui(ui);
            });
    }
}

/// `text` as a QR code, dark on white whatever the theme, with the standard quiet zone.
fn qr_code(ui: &mut egui::Ui, text: &str) {
    let Ok(code) = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium) else {
        return;
    };
    let modules = code.size() + 8;
    let side = ui.available_width().min(220.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
    let module = side / modules as f32;
    for y in 0..code.size() {
        for x in 0..code.size() {
            if code.get_module(x, y) {
                let min = rect.min + egui::vec2((x + 4) as f32, (y + 4) as f32) * module;
                painter.rect_filled(
                    egui::Rect::from_min_size(min, egui::vec2(module, module)),
                    0.0,
                    egui::Color32::BLACK,
                );
            }
        }
    }
}
