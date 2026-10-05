//! `?speed` or `?speed=<games>`: timing checks and solver games in the analysis worker
//! (`src/bin/worker.rs`), shown as plain text instead of the app.
use crate::{
    page,
    protocol::{self, FromWorker, ToWorker},
};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

/// The number of games, if the page was opened as the speed test.
pub fn requested(query: &str) -> Option<u64> {
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

pub async fn run(games: u64) {
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
        move || page::show_report(&lines.borrow().join("\n"))
    });
    show();
    if let Err(error) = run_worker(games, lines.clone(), show.clone()).await {
        lines.borrow_mut().push(format!("Fehler: {error:?}"));
    }
    lines.borrow_mut().push("Fertig.".into());
    show();
}

async fn run_worker(
    games: u64,
    lines: Rc<RefCell<Vec<String>>>,
    show: Rc<dyn Fn()>,
) -> Result<(), JsValue> {
    let worker = web_sys::Worker::new("./worker_loader.js")?;
    let done = js_sys::Promise::new(&mut |resolve, reject| {
        let (target, lines, show) = (worker.clone(), lines.clone(), show.clone());
        worker.set_onerror(Some(&reject));
        let on_message =
            Closure::<dyn Fn(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                let message = event
                    .data()
                    .as_string()
                    .map(|json| protocol::decode::<FromWorker>(&json));
                match message {
                    Some(Ok(FromWorker::Ready { .. })) => {
                        let request = protocol::encode(&ToWorker::SpeedTest { games });
                        let _ = target.post_message(&JsValue::from_str(&request));
                    }
                    Some(Ok(FromWorker::Line(line))) => {
                        lines.borrow_mut().push(line);
                        show();
                    }
                    Some(Ok(FromWorker::SpeedDone)) => {
                        let _ = resolve.call0(&JsValue::NULL);
                    }
                    Some(Ok(FromWorker::Panicked(details))) => {
                        let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&details));
                    }
                    _ => {}
                }
            });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();
    });
    let result = wasm_bindgen_futures::JsFuture::from(done).await;
    worker.terminate();
    result.map(|_| ())
}
