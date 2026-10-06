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
    (
        "Analysis waits until the sheets fit together.",
        "Die Analyse wartet, bis die Blätter zusammenpassen.",
    ),
    // Phone: dice.
    ("{name} is up", "{name} ist dran"),
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
    ("Thinking time", "Rechenzeit"),
    ("Quick", "Schnell"),
    ("Thorough", "Gründlich"),
    ("No limit", "Ohne Limit"),
    ("Save", "Speichern"),
    ("Advanced …", "Erweitert …"),
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
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("i18n.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
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
