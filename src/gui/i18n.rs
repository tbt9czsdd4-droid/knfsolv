//! Interface language. Code writes the English text, `t("Undo")`, and German comes from
//! [`GERMAN`]. The desktop app is English; the web app switches to German at startup.
//!
//! A test reads every source file of the interface and fails when a text passed to `t` or `tf`
//! has no German entry, or when an entry is no longer used.
use std::{cell::Cell, collections::HashMap, fmt::Display, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    German,
}

thread_local! {
    // Per thread, so that tests running in parallel each see their own language.
    static LANGUAGE: Cell<Language> = const { Cell::new(Language::English) };
}

pub fn set_language(language: Language) {
    LANGUAGE.with(|l| l.set(language));
}
pub fn language() -> Language {
    LANGUAGE.with(Cell::get)
}

/// The interface text for `english` in the current language.
pub fn t(english: &'static str) -> &'static str {
    match language() {
        Language::English => english,
        Language::German => german(english).unwrap_or(english),
    }
}

/// Like [`t`], then replaces each `{name}` with its value.
pub fn tf(english: &'static str, values: &[(&str, &dyn Display)]) -> String {
    values
        .iter()
        .fold(t(english).to_string(), |text, (name, value)| {
            text.replace(&format!("{{{name}}}"), &value.to_string())
        })
}

/// The name of score category `category` (0 = Ones … 12 = Chance), as on the paper sheet.
pub fn category_name(category: usize) -> &'static str {
    const GERMAN_CATEGORIES: [&str; 13] = [
        "Einser",
        "Zweier",
        "Dreier",
        "Vierer",
        "Fünfer",
        "Sechser",
        "Dreierpasch",
        "Viererpasch",
        "Full House",
        "Kleine Straße",
        "Große Straße",
        "Kniffel",
        "Chance",
    ];
    match language() {
        Language::English => crate::rules::NAMES[category],
        Language::German => GERMAN_CATEGORIES[category],
    }
}

/// A short category name for narrow score sheets.
pub fn category_short(category: usize) -> &'static str {
    const ENGLISH: [&str; 13] = [
        "1s", "2s", "3s", "4s", "5s", "6s", "3 kind", "4 kind", "Full H.", "Sm. str.", "Lg. str.",
        "Kniffel", "Chance",
    ];
    const GERMAN_SHORT: [&str; 13] = [
        "1er",
        "2er",
        "3er",
        "4er",
        "5er",
        "6er",
        "3er-Pasch",
        "4er-Pasch",
        "Full H.",
        "Kl. Str.",
        "Gr. Str.",
        "Kniffel",
        "Chance",
    ];
    match language() {
        Language::English => ENGLISH[category],
        Language::German => GERMAN_SHORT[category],
    }
}

/// `value` with `decimals` places: a decimal comma in German.
pub fn decimal(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    match language() {
        Language::English => text,
        Language::German => text.replace('.', ","),
    }
}

/// A fraction as a percentage: `0.583` is "58,3 %" in German and "58.3%" in English.
pub fn percent(fraction: f64, decimals: usize) -> String {
    let number = decimal(fraction * 100.0, decimals);
    match language() {
        Language::English => format!("{number}%"),
        Language::German => format!("{number}\u{a0}%"),
    }
}

/// Thousands grouped: 6400 is "6 400" in German and "6,400" in English.
pub fn count(n: u64) -> String {
    let separator = match language() {
        Language::English => ",",
        Language::German => "\u{a0}",
    };
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push_str(separator);
        }
        grouped.push(digit);
    }
    grouped
}

/// An engine error or message for the interface: in German, messages users can trigger get their
/// own wording ([`ENGINE_GERMAN`]), messages with a known beginning keep their original detail, and
/// anything else is a German sentence with the original text.
pub fn error_text(error: &dyn Display) -> String {
    let text = error.to_string();
    if language() == Language::English {
        return text;
    }
    if let Some((_, german)) = ENGINE_GERMAN.iter().find(|(english, _)| *english == text) {
        return german.to_string();
    }
    for (english, german) in ENGINE_PREFIXES {
        if let Some(detail) = text.strip_prefix(english) {
            return format!("{german}{detail}");
        }
    }
    tf("Something went wrong: {error}", &[("error", &text)])
}

/// Engine messages users can trigger, and their German wording.
const ENGINE_GERMAN: &[(&str, &str)] = &[
    (
        "Physical dice entry is unavailable for this turn",
        "Hier können gerade keine Würfel eingegeben werden.",
    ),
    (
        "This entry is not legal for these dice",
        "Dieser Eintrag passt nicht zu diesen Würfeln.",
    ),
    (
        "Inconsistent score sheets: completed-turn counts must follow the configured turn order. Correct the entries to resume analysis.",
        "Die Spielblätter passen nicht zusammen: Die Zahl der Einträge muss zur Reihenfolge passen. Korrigiere die Einträge, dann geht die Analyse weiter.",
    ),
    (
        "Finish correcting dice and leave Edit position before playing a human move",
        "Korrigiere zuerst die Würfel und beende „Stellung bearbeiten“, bevor du ziehst.",
    ),
    (
        "Extra Kniffels require 50 in Kniffel and enough completed turns",
        "Zusatz-Kniffel brauchen 50 Punkte beim Kniffel und genug ausgefüllte Zeilen.",
    ),
    (
        "Enter up to five dice, each from 1 to 6",
        "Gib bis zu fünf Würfel mit 1 bis 6 Augen ein.",
    ),
    (
        "Enter the physical roll using the face palette",
        "Gib den Wurf mit den Würfeln unten ein.",
    ),
    (
        "Enter five dice for a consistent position before applying advice",
        "Gib alle fünf Würfel ein und korrigiere die Stellung, bevor du einen Vorschlag übernimmst.",
    ),
    (
        "Enter a name for every player.",
        "Gib jedem Spieler einen Namen.",
    ),
    (
        "Enter all five dice, including retained dice",
        "Gib alle fünf Würfel ein, auch die behaltenen.",
    ),
    ("Enter all five dice", "Gib alle fünf Würfel ein."),
    (
        "Enable Edit position before changing historical entries",
        "Schalte „Stellung bearbeiten“ ein, um frühere Einträge zu ändern.",
    ),
    ("Die faces must be 1–6", "Würfel zeigen 1 bis 6 Augen."),
    (
        "Complete the dice entry before saving, or use Undo to reset the draft",
        "Gib den Wurf vollständig ein oder setze ihn mit Rückgängig zurück.",
    ),
    (
        "Choose 2–8 players and a starting player.",
        "Wähle 2–8 Spieler und wer anfängt.",
    ),
    (
        "Category is already filled or invalid",
        "Dieses Feld ist schon ausgefüllt.",
    ),
    (
        "Action is not legal in this position",
        "Dieser Zug ist hier nicht möglich.",
    ),
    (
        "Action is not legal in this phase",
        "Dieser Zug ist gerade nicht möglich.",
    ),
];

/// Engine messages that end in a detail, such as a storage error.
const ENGINE_PREFIXES: &[(&str, &str)] = &[
    (
        "Preferences could not be loaded; using defaults. ",
        "Die Einstellungen ließen sich nicht laden; es gelten die Standardwerte. ",
    ),
    (
        "Preferences were not saved. ",
        "Die Einstellungen wurden nicht gespeichert. ",
    ),
];

fn german(english: &str) -> Option<&'static str> {
    static TABLE: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    TABLE
        .get_or_init(|| GERMAN.iter().copied().collect())
        .get(english)
        .copied()
}

/// English interface text and its German translation, grouped by screen.
const GERMAN: &[(&str, &str)] = &[
    // Browser: sharing and opening positions.
    ("Share position", "Stellung teilen"),
    ("Open position", "Stellung öffnen"),
    ("Opened the position.", "Stellung geöffnet."),
    (
        "This file is not a saved position. {error}",
        "Diese Datei ist keine gespeicherte Stellung. {error}",
    ),
    ("Replace position", "Stellung ersetzen"),
    (
        "Discard the current position?",
        "Aktuelle Stellung verwerfen?",
    ),
    ("Discard", "Verwerfen"),
    ("Go back", "Zurück"),
    // Score picker.
    (
        "{points} with the current roll",
        "{points} mit aktuellem Wurf",
    ),
    ("Open", "Offen"),
    ("Cross out (0)", "Streichen (0)"),
    ("Cancel", "Abbrechen"),
    // Advice.
    ("Apply suggestion", "Übernehmen"),
    ("Apply this move", "Diesen Zug übernehmen"),
    ("Compute longer", "Länger rechnen"),
    ("Play suggestion", "Zug ausführen"),
    ("Play this move", "Diesen Zug ausführen"),
    ("Other moves", "Andere Züge"),
    ("Computing …", "Rechnet …"),
    (
        "No suggestion for this position.",
        "Kein Vorschlag für diese Stellung.",
    ),
    ("Chance of winning {percent}", "Gewinnchance {percent}"),
    ("Ø {points} points", "Ø {points} Punkte"),
    ("Keep", "Behalte"),
    ("· reroll {count}", "· würfle {count} neu"),
    ("Reroll all five dice", "Alle 5 Würfel neu würfeln"),
    (
        "Score {category}: {points} points",
        "{category} eintragen: {points} Punkte",
    ),
    (
        "Cross out {category} (0 points)",
        "{category} streichen (0 Punkte)",
    ),
    ("incl. 50 bonus points", "inkl. 50 Zusatzpunkte"),
    ("Forecast before the roll", "Prognose vor dem Wurf"),
    ("✔ calculated exactly", "✔ exakt berechnet"),
    (
        "✔ done · {count} games simulated",
        "✔ fertig · {count} Partien simuliert",
    ),
    ("computing · {count} games", "rechnet · {count} Partien"),
    (
        "provisional, still computing …",
        "vorläufig, rechnet noch …",
    ),
    ("paused", "pausiert"),
    (
        "Close call: other moves are almost as good",
        "Knappe Sache: Andere Züge sind fast gleich gut",
    ),
    ("Technical details", "Technische Details"),
    (
        "{count} games · {seconds} s computing",
        "{count} Partien · {seconds} s Rechenzeit",
    ),
    ("Win {win} · tie {tie}", "Sieg {win} · Gleichstand {tie}"),
    ("The computer is playing …", "Der Computer spielt …"),
    // Phone: dice.
    ("{name} is up", "{name} ist dran"),
    ("I am up", "Ich bin dran"),
    ("You", "Du"),
    ("You are up", "Du bist dran"),
    ("Assumed: I am up", "Annahme: Ich bin dran"),
    ("Game finished", "Spiel beendet"),
    (
        "The sheets do not fit together",
        "Die Blätter passen nicht zusammen",
    ),
    ("roll", "Wurf"),
    (
        "Enter all five dice for advice.",
        "Für einen Rat alle fünf Würfel eingeben.",
    ),
    ("Roll the dice", "Würfeln"),
    (
        "Roll the dice for advice.",
        "Würfle, dann gibt es einen Rat.",
    ),
    ("Reroll {count} dice", "{count} Würfel neu würfeln"),
    // Phone: score sheet.
    ("Bonus", "Bonus"),
    ("Total", "Endsumme"),
    // Phone: menu.
    (
        "New position (same players)",
        "Neue Stellung (gleiche Spieler)",
    ),
    ("New game …", "Neues Spiel …"),
    ("Appearance", "Darstellung"),
    ("Light", "Hell"),
    ("Dark", "Dunkel"),
    ("System", "System"),
    ("Text size", "Schriftgröße"),
    ("Normal", "Normal"),
    ("Large", "Groß"),
    ("Thinking time", "Rechenzeit"),
    ("Quick", "Schnell"),
    ("Thorough", "Gründlich"),
    ("No limit", "Ohne Limit"),
    ("Save", "Speichern"),
    ("Advanced …", "Erweitert …"),
    // Players.
    ("Player", "Spieler"),
    ("Name", "Name"),
    ("Extra Kniffels", "Zusatz-Kniffel"),
    ("50 points each", "je 50 Punkte"),
    (
        "Extra Kniffels need 50 points in Kniffel and enough filled rows.",
        "Zusatz-Kniffel brauchen 50 Punkte beim Kniffel und genug ausgefüllte Zeilen.",
    ),
    ("Turn order", "Reihenfolge"),
    ("Earlier", "Früher"),
    ("Later", "Später"),
    ("Place {place} of {count}", "Platz {place} von {count}"),
    ("Done", "Fertig"),
    (
        "The number of entries does not fit the turn order – is an entry missing somewhere?",
        "Die Anzahl der Einträge passt nicht zur Reihenfolge – fehlt irgendwo ein Eintrag?",
    ),
    (
        "Analyze anyway – who is up?",
        "Trotzdem analysieren – wer ist dran?",
    ),
    ("Assumed: {name} is up", "Annahme: {name} ist dran"),
    ("Determine automatically", "Automatisch bestimmen"),
    // Messages when a session starts.
    (
        "Edit scores to forecast the next player. Dice are optional; enter five for move advice.",
        "Trage die Punkte ein; die App sagt voraus, wer gewinnt. Mit fünf Würfeln gibt es einen Zugvorschlag.",
    ),
    (
        "New local game. Choose which dice to keep; click a score to end each turn.",
        "Neues lokales Spiel. Tippe Würfel zum Behalten an; ein Eintrag im Blatt beendet den Zug.",
    ),
    (
        "New computer game. Human rolls wait for your input; computer dice are generated automatically.",
        "Neues Spiel gegen den Computer. Bei deinen Würfen wartet die App auf dich; der Computer würfelt automatisch.",
    ),
    // Phone start screens.
    (
        "Copy the score sheets, tap in the dice, and see the best move with its chance of winning.",
        "Übertrage die Spielblätter, tippe die Würfel ein und sieh den besten Zug mit seiner Gewinnchance.",
    ),
    ("Back to the position", "Zurück zur Stellung"),
    ("Analyze a position", "Stellung analysieren"),
    ("Play instead of analyzing:", "Spielen statt analysieren:"),
    ("Local game", "Lokales Spiel"),
    ("Against the computer", "Gegen den Computer"),
    ("Who is playing?", "Wer spielt mit?"),
    (
        "In playing order, starting with whoever began. Names are optional.",
        "In Spielreihenfolge, beginnend mit dem, der angefangen hat. Namen sind freiwillig.",
    ),
    ("Me", "Ich"),
    ("Player {number}", "Spieler {number}"),
    ("Start", "Los geht's"),
    ("Back", "Zurück"),
    ("How many are playing?", "Wie viele spielen mit?"),
    (
        "In playing order. Names are optional.",
        "In Spielreihenfolge. Namen sind freiwillig.",
    ),
    ("Who starts?", "Wer fängt an?"),
    ("Dice", "Würfel"),
    ("Real dice", "Echte Würfel"),
    ("The app rolls", "Die App würfelt"),
    (
        "Roll real dice and tap in their faces.",
        "Würfelt mit echten Würfeln und tippt die Augen ein.",
    ),
    (
        "Tap “Roll the dice” and the app rolls for you.",
        "Tippe auf „Würfeln“, dann würfelt die App.",
    ),
    ("Your name", "Dein Name"),
    ("Your dice", "Deine Würfel"),
    (
        "The app always rolls for the computer.",
        "Für den Computer würfelt immer die App.",
    ),
    ("Strength", "Spielstärke"),
    (
        "Custom search, set on a larger screen.",
        "Eigene Suche, auf einem größeren Bildschirm eingestellt.",
    ),
    // Computer strength.
    ("Easy", "Leicht"),
    ("Standard", "Mittel"),
    ("Strong", "Stark"),
    // Results.
    ("{name} wins", "{name} gewinnt"),
    ("Tie: {names}", "Gleichstand: {names}"),
    ("{points} points", "{points} Punkte"),
    (
        "Upper section {upper} · bonus {bonus} · extra Kniffels {extra}",
        "Oben {upper} · Bonus {bonus} · Zusatz-Kniffel {extra}",
    ),
    ("New game", "Neues Spiel"),
    (
        "↺ takes back the last entry.",
        "↺ nimmt den letzten Eintrag zurück.",
    ),
    ("Winner: {names}", "Sieger: {names}"),
    ("Tied winners: {names}", "Gemeinsame Sieger: {names}"),
    ("Review score sheets", "Spielblätter ansehen"),
    (
        "Totals include the upper bonus and extra Kniffel points. Undo restores the last scoring decision.",
        "Die Summen enthalten den Bonus und die Zusatz-Kniffel. Rückgängig nimmt den letzten Eintrag zurück.",
    ),
    ("Upper subtotal", "Summe oben"),
    ("Upper bonus", "Bonus"),
    ("Extra points", "Zusatzpunkte"),
    ("TOTAL", "ENDSUMME"),
    ("Result", "Ergebnis"),
    ("Winner", "Sieger"),
    ("Show results", "Ergebnis zeigen"),
    // Larger screens: appearance and setup.
    ("💻 System", "💻 System"),
    ("☀ Light", "☀ Hell"),
    ("🌙 Dark", "🌙 Dunkel"),
    (
        "Standard rules · two rerolls per turn · 13 scoring categories",
        "Standardregeln · zweimal nachwürfeln pro Zug · 13 Felder",
    ),
    ("Choose how to play", "Wie möchtest du spielen?"),
    (
        "2–8 people sharing a score sheet, with physical or digital dice.",
        "2–8 Personen mit einem gemeinsamen Spielblatt, mit echten Würfeln oder App-Würfeln.",
    ),
    ("Play computer", "Gegen den Computer"),
    ("Players", "Spieler"),
    ("Starting player", "Beginnt"),
    (
        "Play follows the listed order, wrapping around from the starting player.",
        "Gespielt wird in der Reihenfolge der Liste, beginnend beim gewählten Spieler.",
    ),
    ("Physical dice", "Echte Würfel"),
    ("Digital dice", "App-Würfel"),
    (
        "Roll your own dice, then enter their faces with the palette or keys 1–6.",
        "Würfle mit echten Würfeln und gib die Augen mit der Palette oder den Tasten 1–6 ein.",
    ),
    (
        "Click Roll when you are ready. The app generates the dice.",
        "Klicke auf „Würfeln“, wenn du bereit bist. Die App würfelt.",
    ),
    ("Start local game", "Lokales Spiel starten"),
    (
        "Start with empty sheets, then edit any scores, enter dice, and set the active player.",
        "Beginne mit leeren Blättern, trage Punkte und Würfel ein und wähle, wer dran ist.",
    ),
    ("Open position editor", "Stellung eingeben"),
    ("Return to session", "Zurück zur Stellung"),
    ("Computer name", "Name des Computers"),
    ("Starts", "Beginnt"),
    ("You first", "Du zuerst"),
    ("Computer first", "Computer zuerst"),
    (
        "Computer rolls are always generated by the app. Your digital rolls wait for Roll.",
        "Für den Computer würfelt immer die App. Mit App-Würfeln wartet sie bei dir auf „Würfeln“.",
    ),
    ("Difficulty", "Spielstärke"),
    ("Custom search", "Eigene Suche"),
    (
        "Custom search · strength uncalibrated. More thinking does not guarantee stronger play.",
        "Eigene Suche · Spielstärke nicht gemessen. Mehr Rechenzeit heißt nicht unbedingt stärkeres Spiel.",
    ),
    (
        "Continuation-v2 baseline with bounded MCTS and independent validation. Coaching approximates it with the baseline.",
        "Grundstrategie Continuation-v2 mit begrenzter MCTS-Suche und unabhängiger Prüfung. Die Beratung nähert sie mit der Grundstrategie an.",
    ),
    ("Time (ms; 0 = unlimited)", "Zeit (ms; 0 = unbegrenzt)"),
    ("Simulations", "Simulationen"),
    ("Depth (0 = unlimited)", "Tiefe (0 = unbegrenzt)"),
    ("Nodes", "Knoten"),
    ("Conservative advice", "Vorsichtige Ratschläge"),
    ("Model: {name}", "Modell: {name}"),
    (
        "Presets use different decision policies, evaluated in held-out games.",
        "Die Stufen spielen nach verschiedenen Strategien, geprüft in eigenen Testpartien.",
    ),
    ("Fast play", "Schnell spielen"),
    ("Start computer game", "Spiel gegen den Computer starten"),
    // Larger screens: toolbar.
    ("Edit position", "Stellung bearbeiten"),
    ("Digital play", "Spiel mit App-Würfeln"),
    ("Physical play", "Spiel mit echten Würfeln"),
    ("Play", "Spielen"),
    ("Undo", "Rückgängig"),
    (
        "Ctrl/Cmd-Z · first resets unfinished dice corrections",
        "Strg/Cmd-Z · setzt zuerst unfertige Würfelkorrekturen zurück",
    ),
    ("Redo", "Wiederholen"),
    ("File", "Datei"),
    ("Settings", "Einstellungen"),
    ("Resume computer", "Computer fortsetzen"),
    ("Pause computer", "Computer pausieren"),
    (
        "Position needs attention: {error}",
        "Die Stellung braucht eine Korrektur: {error}",
    ),
    // Larger screens: dice.
    ("Dice (optional)", "Würfel (optional)"),
    ("Next: {name}", "Als Nächstes: {name}"),
    ("Next player unresolved", "Unklar, wer dran ist"),
    ("Turn: {name}", "Am Zug: {name}"),
    ("1 reroll left", "noch 1× würfeln"),
    ("{count} rerolls left", "noch {count}× würfeln"),
    (
        "Forecast averages over future rolls. Optionally enter five faces for move advice.",
        "Die Prognose mittelt über künftige Würfe. Mit fünf eingegebenen Würfeln gibt es einen Zugvorschlag.",
    ),
    (
        "The computer rolls and plays automatically.",
        "Der Computer würfelt und spielt selbst.",
    ),
    (
        "Ready when you are. Roll to continue.",
        "Würfle, wenn du bereit bist.",
    ),
    (
        "Enter five faces with the picker or keys 1–6.",
        "Gib fünf Augenzahlen mit der Palette oder den Tasten 1–6 ein.",
    ),
    (
        "Enter new faces only. Retained dice are locked.",
        "Gib nur die neuen Würfel ein. Behaltene Würfel bleiben fest.",
    ),
    (
        "Click dice to keep. Use the picker or 1–6 to correct the selected die.",
        "Klicke Würfel zum Behalten an. Mit der Palette oder 1–6 korrigierst du den gewählten Würfel.",
    ),
    (
        "Click dice to keep, then reroll or choose a score.",
        "Klicke Würfel zum Behalten an, dann würfle neu oder trage ein.",
    ),
    ("Game finished.", "Spiel beendet."),
    ("Locked", "Fest"),
    ("Kept", "Behalten"),
    ("Enter", "Eingeben"),
    ("Ready", "Bereit"),
    ("Reroll", "Neu würfeln"),
    ("Rolled", "Gewürfelt"),
    (
        "Click to select for correction and toggle Keep. Advice is the engine recommendation only.",
        "Klicken wählt den Würfel zum Korrigieren und schaltet Behalten um. ★ ist nur der Vorschlag der App.",
    ),
    ("★ Advice", "★ Vorschlag"),
    ("Keep {face}", "Behalte {face}"),
    (
        "Enter {face} · key {face}",
        "{face} eingeben · Taste {face}",
    ),
    ("Clear", "Löschen"),
    (
        "Backspace: clear the last editable die",
        "Rücktaste: den letzten änderbaren Würfel löschen",
    ),
    ("Roll", "Würfeln"),
    ("Reroll 1 die", "1 Würfel neu würfeln"),
    (
        "Retain selected dice, then enter the new physical faces.",
        "Gewählte Würfel behalten, dann die neuen Augen eingeben.",
    ),
    (
        "Generate the unkept dice now.",
        "Die übrigen Würfel jetzt würfeln.",
    ),
    (
        "Analyze before first roll",
        "Vor dem ersten Wurf analysieren",
    ),
    (
        "Complete the roll to continue.",
        "Gib den Wurf vollständig ein, um weiterzumachen.",
    ),
    ("Updating roll…", "Wurf wird übernommen …"),
    // Larger screens: score sheets.
    ("Score sheets", "Spielblätter"),
    (
        "Click a score to pick its value, or hover and scroll. Open = unfilled; 0 = crossed out. Filled entries determine the next player.",
        "Klicke auf ein Feld, um den Wert zu wählen, oder scrolle darüber. Offen = leer; 0 = gestrichen. Die Einträge bestimmen, wer dran ist.",
    ),
    (
        "Edit mode: click any entry. Empty = open; 0 = crossed out. Turns are paused.",
        "Bearbeiten: Klicke auf einen Eintrag. Leer = offen; 0 = gestrichen. Die Züge ruhen.",
    ),
    (
        "Click an available score to end the turn, including 0. Undo restores it. ★ = advice.",
        "Klicke auf ein freies Feld, um den Zug zu beenden, auch mit 0. Rückgängig nimmt es zurück. ★ = Vorschlag.",
    ),
    ("Category", "Feld"),
    (" (turn)", " (dran)"),
    (
        "Scroll up/down through legal scores, or click to pick one. Open = unfilled; 0 = crossed out. Wheel limits: Settings → Game.",
        "Scrolle durch die möglichen Werte oder klicke, um einen zu wählen. Offen = leer; 0 = gestrichen. Grenzen des Mausrads: Einstellungen → Spiel.",
    ),
    (
        "Score and advance; also awards 50 extra Kniffel points.",
        "Eintragen und weiter; gibt zusätzlich 50 Punkte für den Zusatz-Kniffel.",
    ),
    (
        "Score and advance to the next player.",
        "Eintragen und weiter zum nächsten Spieler.",
    ),
    // Larger screens: advice.
    ("Engine settings", "Rechen-Einstellungen"),
    ("Preparing…", "Wird vorbereitet …"),
    ("Tree depth {depth}", "Baumtiefe {depth}"),
    (
        "Tree depth {depth} · validating",
        "Baumtiefe {depth} · Prüfung",
    ),
    ("Exact depth {depth}", "Exakte Tiefe {depth}"),
    ("Full-game rollouts", "Ganze Partien simuliert"),
    (
        "Deepest explored branch, not a depth completed uniformly across every move. Counts the analyzed player's decisions (root = 1), excluding dice outcomes, opponents and baseline rollout continuation.",
        "Tiefster untersuchter Zweig, nicht eine für jeden Zug vollständig erreichte Tiefe. Zählt die Entscheidungen des analysierten Spielers (Wurzel = 1), ohne Würfelergebnisse, Gegner und die Fortsetzung mit der Grundstrategie.",
    ),
    ("Computer turn", "Der Computer ist dran"),
    (
        "Custom search · strength uncalibrated",
        "Eigene Suche · Spielstärke nicht gemessen",
    ),
    ("Computer paused.", "Computer pausiert."),
    ("Choice: {move}", "Wahl: {move}"),
    (
        "Coaching resumes for the next human turn.",
        "Die Beratung geht beim nächsten Zug eines Menschen weiter.",
    ),
    (
        "Analysis paused · inconsistent sheets",
        "Analyse pausiert · Blätter passen nicht zusammen",
    ),
    (
        "Before-roll forecast · {name}",
        "Prognose vor dem Wurf · {name}",
    ),
    ("Advice", "Vorschlag"),
    ("Continuous", "Fortlaufend"),
    ("Limited", "Begrenzt"),
    ("Resume analysis", "Analyse fortsetzen"),
    ("Pause analysis", "Analyse pausieren"),
    ("Restart analysis", "Analyse neu starten"),
    (
        "Analysis waits: correct the position, or complete five dice or choose Analyze before first roll.",
        "Die Analyse wartet: Korrigiere die Stellung, gib alle fünf Würfel ein oder wähle „Vor dem ersten Wurf analysieren“.",
    ),
    (
        "Coaching waits for a complete, valid human roll.",
        "Die Beratung wartet auf einen vollständigen, gültigen Wurf.",
    ),
    ("Paused", "Pausiert"),
    ("Refining", "Wird verfeinert"),
    ("Stopped", "Beendet"),
    ("Provisional baseline · ", "Vorläufig, Grundstrategie · "),
    ("Confidence: {confidence}", "Sicherheit: {confidence}"),
    ("incomplete", "unvollständig"),
    ("estimated", "geschätzt"),
    ("exact under model", "exakt im Modell"),
    ("policy decision", "Strategieentscheidung"),
    ("insufficient", "unzureichend"),
    ("low", "niedrig"),
    ("separated under model", "im Modell klar getrennt"),
    ("not applicable", "entfällt"),
    (
        "Averaged over possible future rolls and modeled play.",
        "Gemittelt über mögliche künftige Würfe und modelliertes Spiel.",
    ),
    ("No recommendation yet.", "Noch kein Vorschlag."),
    ("Game forecast", "Prognose"),
    ("Show fewer", "Weniger zeigen"),
    ("Show all moves", "Alle Züge zeigen"),
    ("Analysis details", "Technische Details"),
    (
        "Paused · waiting for an estimate.",
        "Pausiert · wartet auf eine Schätzung.",
    ),
    (
        "Preparing provisional baseline advice…",
        "Vorläufiger Vorschlag wird vorbereitet …",
    ),
    ("Estimating…", "Wird geschätzt …"),
    ("Win", "Sieg"),
    ("Tie", "Gleichstand"),
    ("Lose", "Niederlage"),
    ("Expected final game score", "Erwartete Endpunktzahl"),
    (
        "Win: outright first. Tie: tied for first. Lose: all remaining outcomes.",
        "Sieg: allein Erster. Gleichstand: gemeinsam Erster. Niederlage: alles andere.",
    ),
    (
        "Expected total at the end of the game, including future turns and bonuses.",
        "Erwartete Summe am Spielende, mit allen künftigen Zügen und Boni.",
    ),
    (
        "Score {category} · {points} points now",
        "{category} eintragen · jetzt {points} Punkte",
    ),
    (" (+50 extra Kniffel)", " (+50 Zusatz-Kniffel)"),
    ("Roll all five dice", "Alle fünf Würfel würfeln"),
    ("Recommended", "Empfohlen"),
    ("Alternative", "Alternative"),
    ("Keep none", "Nichts behalten"),
    // Settings („Erweitert“).
    ("‹ Back", "‹ Zurück"),
    ("Advanced", "Erweitert"),
    ("Engine", "Berechnung"),
    ("Game", "Spiel"),
    (
        "Edit position (enable historical and player editing)",
        "Stellung bearbeiten (frühere Einträge und Spieler ändern)",
    ),
    (
        "Score wheel limits (global editor preference)",
        "Mausrad an den Grenzwerten (für alle Stellungen)",
    ),
    ("Stop at ends", "Anhalten"),
    ("Wrap around", "Von vorn beginnen"),
    ("Human dice", "Würfel der Menschen"),
    ("Physical", "Echt"),
    ("Digital", "App"),
    ("Active player", "Wer ist dran"),
    ("Advice for", "Rat für"),
    ("Hypothetical position", "Angenommene Stellung"),
    ("Phase", "Phase"),
    ("Before first roll", "Vor dem ersten Wurf"),
    ("Finished", "Beendet"),
    ("Rerolls left", "Würfe übrig"),
    ("Extra Kniffels (count)", "Zusatz-Kniffel (Anzahl)"),
    (
        "Turn order (move players left):",
        "Reihenfolge (Spieler nach vorn schieben):",
    ),
    ("Seat {number} model default", "Platz {number}: Modell"),
    ("Opportunity", "Chancen"),
    ("Greedy", "Gierig"),
    ("Legacy opportunity", "Chancen (alt)"),
    ("Objective", "Ziel"),
    (
        "Shared first (split ties)",
        "Gewinnchance (Gleichstand geteilt)",
    ),
    ("Outright first", "Allein gewinnen"),
    ("Expected score", "Punkte"),
    ("Method", "Methode"),
    ("Auto", "Automatisch"),
    ("Exact", "Exakt"),
    ("Rollout", "Simulation"),
    (
        "Time limit (ms; 0 unlimited)",
        "Zeitlimit (ms; 0 = unbegrenzt)",
    ),
    ("Search nodes", "Suchknoten"),
    (
        "MCTS depth limit (0 unlimited)",
        "MCTS-Tiefe (0 = unbegrenzt)",
    ),
    ("Random seed", "Startwert für den Zufall"),
    (
        "Conservative advice: retain baseline unless improvement is established",
        "Vorsichtige Ratschläge: bei der Grundstrategie bleiben, bis eine Verbesserung gesichert ist",
    ),
    (
        "Depth counts your decisions, not dice or opponents. Simulations finish the whole game. Exact search uses time/nodes.",
        "Die Tiefe zählt deine Entscheidungen, nicht Würfel oder Gegner. Simulationen spielen die ganze Partie zu Ende. Die exakte Suche nutzt Zeit und Knoten.",
    ),
    // Messages.
    (
        "Something went wrong: {error}",
        "Etwas ist schiefgegangen: {error}",
    ),
    ("Move applied.", "Zug ausgeführt."),
    (
        "Correct the position and complete or discard partial dice entry before analysis.",
        "Korrigiere die Stellung und gib den Wurf vollständig ein oder verwirf ihn, dann beginnt die Analyse.",
    ),
    (
        "Choosing the declared baseline after a search failure…",
        "Die Suche ist fehlgeschlagen; der Computer spielt nach seiner Grundstrategie …",
    ),
    ("Thinking…", "Denkt nach …"),
    (
        "Thinking… {count} completed games",
        "Denkt nach … {count} Partien",
    ),
    (
        "The search budget ended without advice.",
        "Die Rechenzeit war um, ohne dass ein Zug gefunden wurde.",
    ),
    (
        "Computer paused: its legal baseline could not be obtained. {reason}",
        "Computer pausiert: Er konnte keinen gültigen Zug bestimmen. {reason}",
    ),
    (
        "Computer search fallback: {reason} Using the declared legal baseline.",
        "Die Suche des Computers ist fehlgeschlagen: {reason} Er spielt nach seiner Grundstrategie.",
    ),
    ("{name} rolled {dice}.", "{name} würfelt {dice}."),
    ("{name}: {move}.", "{name}: {move}."),
    ("Preferences", "Einstellungen"),
    ("Retry", "Erneut versuchen"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Every string literal passed directly to `t(` or `tf(` in the interface sources.
    fn translated_literals() -> HashSet<String> {
        let mut found = HashSet::new();
        for entry in std::fs::read_dir("src/gui").unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let mut source = std::fs::read_to_string(&path).unwrap();
            if path.ends_with("i18n.rs") {
                // This file's own texts, without the scanner below.
                source.truncate(source.find("#[cfg(test)]").unwrap());
            }
            for call in ["t(", "tf("] {
                for (at, _) in source.match_indices(call) {
                    let before = source[..at].chars().next_back();
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                        continue;
                    }
                    // `cargo fmt` may move the text onto the next line.
                    if let Some(rest) = source[at + call.len()..].trim_start().strip_prefix('"') {
                        found.insert(literal(rest));
                    }
                }
            }
        }
        found
    }

    /// Reads a Rust string literal up to its closing quote, resolving the escapes we use.
    fn literal(rest: &str) -> String {
        let mut text = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return text,
                '\\' => match chars.next() {
                    Some('n') => text.push('\n'),
                    Some('\n') => {
                        // A line continuation skips the newline and the next line's indentation.
                        let skipped: String =
                            chars.clone().take_while(|c| c.is_whitespace()).collect();
                        for _ in skipped.chars() {
                            chars.next();
                        }
                    }
                    Some(other) => text.push(other),
                    None => break,
                },
                _ => text.push(c),
            }
        }
        panic!("unterminated literal: {}", &rest[..rest.len().min(40)]);
    }

    #[test]
    fn every_interface_text_has_a_german_entry_and_every_entry_is_used() {
        let used = translated_literals();
        let missing: Vec<_> = used.iter().filter(|text| german(text).is_none()).collect();
        assert!(missing.is_empty(), "missing German entries: {missing:#?}");
        let unused: Vec<_> = GERMAN
            .iter()
            .map(|(english, _)| english)
            .filter(|english| !used.contains(**english))
            .collect();
        assert!(unused.is_empty(), "unused German entries: {unused:#?}");
        let distinct: HashSet<_> = GERMAN.iter().map(|(english, _)| english).collect();
        assert_eq!(distinct.len(), GERMAN.len(), "duplicate English keys");
    }

    #[test]
    fn engine_messages_exist_and_unknown_ones_keep_the_original() {
        let mut sources = String::new();
        let mut directories = vec![std::path::PathBuf::from("src")];
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(&directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() && !path.ends_with("gui") {
                    directories.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    sources += &std::fs::read_to_string(&path).unwrap();
                }
            }
        }
        for (english, _) in ENGINE_GERMAN.iter().chain(ENGINE_PREFIXES) {
            assert!(
                sources.contains(english.trim_end()),
                "no engine message: {english}"
            );
        }
        assert_eq!(error_text(&"Enter all five dice"), "Enter all five dice");
        set_language(Language::German);
        assert_eq!(
            error_text(&"Enter all five dice"),
            "Gib alle fünf Würfel ein."
        );
        assert_eq!(
            error_text(&"Preferences were not saved. Disk full"),
            "Die Einstellungen wurden nicht gespeichert. Disk full"
        );
        assert_eq!(
            error_text(&"Disk full"),
            "Etwas ist schiefgegangen: Disk full"
        );
        set_language(Language::English);
    }

    #[test]
    fn placeholders_survive_translation() {
        for (english, german) in GERMAN {
            let names = |text: &str| {
                let mut names: Vec<String> = text
                    .split('{')
                    .skip(1)
                    .filter_map(|part| part.split_once('}').map(|(name, _)| name.to_string()))
                    .collect();
                names.sort();
                names
            };
            assert_eq!(names(english), names(german), "{english}");
        }
    }

    #[test]
    fn numbers_follow_the_language() {
        assert_eq!(percent(0.583, 1), "58.3%");
        assert_eq!(count(6400), "6,400");
        assert_eq!(count(999), "999");
        set_language(Language::German);
        assert_eq!(percent(0.583, 1), "58,3\u{a0}%");
        assert_eq!(percent(0.5, 0), "50\u{a0}%");
        assert_eq!(count(1_234_567), "1\u{a0}234\u{a0}567");
        assert_eq!(decimal(241.25, 1), "241,2");
        assert_eq!(category_short(6), "3er-Pasch");
        set_language(Language::English);
    }

    #[test]
    fn languages_are_per_thread_and_fill_placeholders() {
        assert_eq!(t("Discard"), "Discard");
        set_language(Language::German);
        assert_eq!(t("Discard"), "Verwerfen");
        assert_eq!(
            tf(
                "This file is not a saved position. {error}",
                &[("error", &"x")]
            ),
            "Diese Datei ist keine gespeicherte Stellung. x"
        );
        std::thread::spawn(|| assert_eq!(t("Discard"), "Discard"))
            .join()
            .unwrap();
        set_language(Language::English);
    }
}
