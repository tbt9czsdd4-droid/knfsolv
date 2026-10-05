//! Browser entry point. Run `trunk serve --release` in this directory, or `./serve-lan.sh` to
//! test on a phone.

#[cfg(target_arch = "wasm32")]
mod computation;
#[cfg(target_arch = "wasm32")]
mod keyboard;
#[cfg(target_arch = "wasm32")]
mod page;
#[cfg(any(target_arch = "wasm32", test))]
mod protocol;
// Natively the speed test runs here; in the browser in `src/bin/speed-worker.rs`.
#[cfg(not(target_arch = "wasm32"))]
mod speed;
#[cfg(target_arch = "wasm32")]
mod speed_page;

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    let _ = eframe::WebLogger::init(log::LevelFilter::Info);
    let runner = eframe::WebRunner::new();
    // Chain after eframe's handler; without a Mac there is no Safari console, so show panics.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        page::show_error("Die App ist abgestürzt.", &info.to_string());
    }));
    if let Some(games) = speed_page::requested(&query()) {
        wasm_bindgen_futures::spawn_local(speed_page::run(games));
        return;
    }
    wasm_bindgen_futures::spawn_local(async move {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("app"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("index.html contains <canvas id=\"app\">");
        let (session, path) = startup_session();
        let debug = query().contains("debug");
        let text_fields = keyboard::Shared::default();
        if let Err(error) = keyboard::install(&canvas, text_fields.clone()) {
            log::warn!("Keyboard helper unavailable: {error:?}");
        }
        let result = runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |_cc| {
                    let mut computation = None;
                    // Preferences last for the visit until browser storage (Phase 3).
                    let desktop = knfsolv::gui::Desktop::startup_with(
                        session,
                        path,
                        knfsolv::persistence::MemoryStorage::default(),
                        |wake| {
                            let worker = computation::WorkerComputation::start(wake);
                            computation = Some(worker.clone());
                            Box::new(worker)
                        },
                    );
                    Ok(Box::new(WebApp {
                        desktop,
                        text_fields,
                        debug: computation.filter(|_| debug),
                    }))
                }),
            )
            .await;
        match result {
            Ok(()) => page::hide_loading(),
            Err(error) => page::show_error("Die App konnte nicht starten.", &format!("{error:?}")),
        }
    });
}

/// The desktop interface plus browser-only work after each frame.
#[cfg(target_arch = "wasm32")]
struct WebApp {
    desktop: knfsolv::gui::Desktop,
    text_fields: keyboard::Shared,
    /// `?debug` shows worker diagnostics and a button that crashes the worker.
    debug: Option<computation::WorkerComputation>,
}
#[cfg(target_arch = "wasm32")]
impl eframe::App for WebApp {
    fn clear_color(&self, visuals: &eframe::egui::Visuals) -> [f32; 4] {
        self.desktop.clear_color(visuals)
    }

    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        use eframe::egui;
        self.desktop.ui(ui, frame);
        if let Some(computation) = &self.debug {
            egui::Area::new(egui::Id::new("worker_debug"))
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-8.0, -8.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        let (crashes, reports, hidden) = computation.diagnostics();
                        ui.label(format!(
                            "Worker: {crashes} Abstürze · {reports} Berichte{}",
                            if hidden { " · pausiert" } else { "" }
                        ));
                        if ui.button("Worker abstürzen lassen").clicked() {
                            computation.crash_worker();
                        }
                    });
                });
        }
        keyboard::record(ui.ctx(), &self.text_fields);
    }
}

/// Test positions that open straight in the analyzer: `?bench`, a two-player opening
/// (1 1 2 5 5, two rerolls) to compare device speed, and `?endgame`, one player with only
/// Dreierpasch and Kniffel open, which the exact solver needs about 0.1–0.4 s for.
#[cfg(target_arch = "wasm32")]
fn startup_session() -> (knfsolv::persistence::Session, Option<std::path::PathBuf>) {
    use knfsolv::{dice::Dice, persistence::Session, state::Phase};
    let query = query();
    if query.contains("bench") {
        let mut session = Session::new(2);
        session.position.phase = Phase::Decision {
            dice: Dice([2, 1, 0, 0, 2, 0]),
            rerolls_left: 2,
        };
        return (session, Some("benchmark.json".into()));
    }
    if query.contains("endgame") {
        let mut session = Session::new(1);
        let sheet = &mut session.position.players[0].sheet;
        sheet.cells = [Some(0); 13];
        sheet.cells[6] = None;
        sheet.cells[11] = None;
        session.position.phase = Phase::Decision {
            dice: Dice([0, 1, 1, 0, 0, 3]),
            rerolls_left: 2,
        };
        return (session, Some("endgame.json".into()));
    }
    (Session::new(2), None)
}

#[cfg(target_arch = "wasm32")]
fn query() -> String {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("speed") {
        eprintln!("Build the browser app with Trunk: cd web && trunk serve --release");
        eprintln!("For a phone on the same network: web/serve-lan.sh");
        eprintln!(
            "Compare device speed natively: cargo run --release -p knfsolv-web -- speed [games]"
        );
        return;
    }
    let games = args.next().and_then(|n| n.parse().ok()).unwrap_or(1_000);
    for line in speed::quick_checks() {
        println!("{line}");
    }
    let mut run = speed::SolverRun::new(games);
    while !run.finished() {
        if let Some(line) = run.advance() {
            println!("{line}");
        }
    }
    println!("{}", run.summary());
}
