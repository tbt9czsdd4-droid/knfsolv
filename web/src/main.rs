//! Browser entry point. Run `trunk serve --release` in this directory, or `./serve-lan.sh` to
//! test on a phone.

#[cfg(target_arch = "wasm32")]
mod keyboard;
// In the browser, the speed test runs in `src/bin/speed-worker.rs`.
#[cfg(not(target_arch = "wasm32"))]
mod speed;

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    let _ = eframe::WebLogger::init(log::LevelFilter::Info);
    let runner = eframe::WebRunner::new();
    // Chain after eframe's handler: without a Mac there is no Safari console, so show panics.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        show_status(&format!(
            "Die App ist abgestürzt.\nBitte die Seite neu laden.\n\n{info}"
        ));
    }));
    if let Some(games) = speed_games() {
        wasm_bindgen_futures::spawn_local(run_speed(games));
        return;
    }
    wasm_bindgen_futures::spawn_local(async move {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("app"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("index.html contains <canvas id=\"app\">");
        let (session, path) = startup_session();
        let text_fields = keyboard::Shared::default();
        if let Err(error) = keyboard::install(&canvas, text_fields.clone()) {
            log::warn!("Keyboard helper unavailable: {error:?}");
        }
        let result = runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |_cc| {
                    Ok(Box::new(WebApp {
                        desktop: knfsolv::gui::Desktop::startup(session, path),
                        text_fields,
                    }))
                }),
            )
            .await;
        match result {
            Ok(()) => {
                if let Some(status) = status_element() {
                    status.remove();
                }
            }
            Err(error) => show_status(&format!("Die App konnte nicht starten.\n\n{error:?}")),
        }
    });
}

/// The desktop interface plus browser-only work after each frame.
#[cfg(target_arch = "wasm32")]
struct WebApp {
    desktop: knfsolv::gui::Desktop,
    text_fields: keyboard::Shared,
}
#[cfg(target_arch = "wasm32")]
impl eframe::App for WebApp {
    fn clear_color(&self, visuals: &eframe::egui::Visuals) -> [f32; 4] {
        self.desktop.clear_color(visuals)
    }

    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        self.desktop.ui(ui, frame);
        keyboard::record(ui.ctx(), &self.text_fields);
    }
}

/// `?bench` opens a fixed two-player opening (1 1 2 5 5, two rerolls) to compare device speed.
#[cfg(target_arch = "wasm32")]
fn startup_session() -> (knfsolv::persistence::Session, Option<std::path::PathBuf>) {
    let bench = query().contains("bench");
    let mut session = knfsolv::persistence::Session::new(2);
    if !bench {
        return (session, None);
    }
    session.position.phase = knfsolv::state::Phase::Decision {
        dice: knfsolv::dice::Dice([2, 1, 0, 0, 2, 0]),
        rerolls_left: 2,
    };
    (session, Some("benchmark.json".into()))
}

#[cfg(target_arch = "wasm32")]
fn status_element() -> Option<web_sys::Element> {
    web_sys::window()?.document()?.get_element_by_id("status")
}

#[cfg(target_arch = "wasm32")]
fn show_status(text: &str) {
    if let Some(status) = status_element() {
        status.set_text_content(Some(text));
        let _ = status.set_attribute("class", "visible");
    }
}

#[cfg(target_arch = "wasm32")]
fn query() -> String {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default()
}

/// `?speed` or `?speed=<games>` opens the speed test instead of the app.
#[cfg(target_arch = "wasm32")]
fn speed_games() -> Option<u64> {
    let query = query();
    let value = query
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("speed"))?;
    Some(
        value
            .strip_prefix('=')
            .and_then(|n| n.parse().ok())
            .unwrap_or(1_000),
    )
}

/// Runs the speed checks and solver games in a Web Worker, where Phase 2 will run the solver.
#[cfg(target_arch = "wasm32")]
async fn run_speed(games: u64) {
    use std::{cell::RefCell, rc::Rc};
    let window = web_sys::window().expect("browser window");
    let mut header = vec![
        format!("Geschwindigkeitstest · {games} Partien"),
        window.navigator().user_agent().unwrap_or_default(),
    ];
    if !window.is_secure_context() {
        header.push(
            "Achtung: unverschlüsselte Verbindung. iPhones rechnen dann etwa 35-mal \
             langsamer; die Adresse muss mit https:// beginnen."
                .into(),
        );
    }
    let lines = Rc::new(RefCell::new(header));
    let show: Rc<dyn Fn()> = Rc::new({
        let lines = lines.clone();
        move || show_status(&lines.borrow().join("\n"))
    });
    show();
    if let Err(error) = run_worker(games, lines.clone(), show.clone()).await {
        lines.borrow_mut().push(format!("Fehler: {error:?}"));
    }
    lines.borrow_mut().push("Fertig.".into());
    show();
}

/// The worker posts `true` once it listens, then one string per line, then `null`.
#[cfg(target_arch = "wasm32")]
async fn run_worker(
    games: u64,
    lines: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    show: std::rc::Rc<dyn Fn()>,
) -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};
    let worker = web_sys::Worker::new("./speed-worker_loader.js")?;
    let done = js_sys::Promise::new(&mut |resolve, reject| {
        let (target, lines, show) = (worker.clone(), lines.clone(), show.clone());
        let on_message =
            Closure::<dyn Fn(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                let data = event.data();
                if data.as_bool() == Some(true) {
                    let _ = target.post_message(&JsValue::from_f64(games as f64));
                } else if let Some(line) = data.as_string() {
                    lines.borrow_mut().push(line);
                    show();
                } else {
                    let _ = resolve.call0(&JsValue::NULL);
                }
            });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();
        worker.set_onerror(Some(&reject));
    });
    let result = wasm_bindgen_futures::JsFuture::from(done).await;
    worker.terminate();
    result.map(|_| ())
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
