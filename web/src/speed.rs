//! `?speed` measures where solver time goes, without the interface, so devices can be compared.
//! The same checks run natively with `cargo run --release -p knfsolv-web -- speed`.
use knfsolv::{
    dice::{Dice, tables},
    persistence::Session,
    rules,
    solver::{AnalysisSession, Settings, WorkSlice},
    state::Phase,
};
use std::{hint::black_box, time::Duration};
use web_time::Instant;

/// Small, fixed workloads that isolate the clock, plain integer code, and scoring.
pub fn quick_checks() -> Vec<String> {
    let calls = 1_000_000;
    let start = Instant::now();
    let mut last = start;
    for _ in 0..calls {
        last = black_box(Instant::now());
    }
    let clock = (last - start).as_secs_f64();

    let rounds = 100_000_000_u64;
    let start = Instant::now();
    let mut x = black_box(0x9e37_79b9_7f4a_7c15_u64);
    let mut sum = 0_u64;
    for _ in 0..rounds {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        sum = sum.wrapping_add(x >> 61);
    }
    black_box(sum);
    let integer = start.elapsed().as_secs_f64();

    let full = &tables().full;
    let repeats = 20_000;
    let start = Instant::now();
    let mut total = 0_u64;
    for _ in 0..repeats {
        for &dice in black_box(full) {
            for category in 0..13 {
                total += rules::ordinary_score(dice, category) as u64;
            }
        }
    }
    black_box(total);
    let scoring = start.elapsed().as_secs_f64();
    let scored = (repeats * full.len() * 13) as f64;

    vec![
        format!("Uhr: {:.0} ns pro Aufruf", clock / calls as f64 * 1e9),
        format!(
            "Ganzzahl-Schleife: {:.2} s ({:.2} ns/Runde)",
            integer,
            integer / rounds as f64 * 1e9
        ),
        format!(
            "Wertung: {:.2} s ({:.1} ns/Wertung)",
            scoring,
            scoring / scored * 1e9
        ),
    ]
}

/// A fixed number of coaching games on the opening `1 1 2 5 5` (two players, two rerolls).
pub struct SolverRun {
    session: AnalysisSession,
    games: u64,
    started: Instant,
    checkpoint: (u64, Duration),
    next_report: u64,
}
impl SolverRun {
    pub fn new(games: u64) -> Self {
        let mut position = Session::new(2).position;
        position.phase = Phase::Decision {
            dice: Dice([2, 1, 0, 0, 2, 0]),
            rerolls_left: 2,
        };
        let settings = Settings {
            simulations: games,
            time_ms: 0,
            ..Settings::default()
        };
        let started = Instant::now();
        let mut session =
            AnalysisSession::new(position, settings).expect("valid benchmark position");
        session.set_continuous(false);
        Self {
            session,
            games,
            started,
            checkpoint: (0, Duration::ZERO),
            next_report: games / 10,
        }
    }
    pub fn finished(&self) -> bool {
        self.session.finished()
    }
    /// One 40 ms slice, as the desktop worker runs; returns a line at each tenth of the games.
    pub fn advance(&mut self) -> Option<String> {
        self.session.advance(WorkSlice::default());
        let games = self.session.samples();
        if games < self.next_report && !self.finished() {
            return None;
        }
        let computing = self.session.computation_time();
        let (before, then) = self.checkpoint;
        self.checkpoint = (games, computing);
        self.next_report = games + self.games / 10;
        Some(format!(
            "{games:>6} Partien · {:>6.2} s · zuletzt {:.0} Partien/s",
            computing.as_secs_f64(),
            (games - before) as f64 / (computing - then).as_secs_f64().max(1e-9)
        ))
    }
    pub fn summary(&self) -> String {
        let report = self.session.snapshot();
        let computing = self.session.computation_time().as_secs_f64();
        format!(
            "Ergebnis: {} Partien in {:.2} s Rechenzeit ({:.2} s gesamt) = {:.0} Partien/s · Empfehlung: {}",
            self.session.samples(),
            computing,
            self.started.elapsed().as_secs_f64(),
            self.session.samples() as f64 / computing.max(1e-9),
            report
                .best_action
                .map_or_else(|| "–".into(), |action| action.to_string()),
        )
    }
}
