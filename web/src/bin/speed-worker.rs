//! Engine-only Web Worker for `?speed`. It posts `true` once it listens, receives the number of
//! games, then posts one string per line and finally `null`.

#[cfg(target_arch = "wasm32")]
#[path = "../speed.rs"]
mod speed;

#[cfg(target_arch = "wasm32")]
mod web {
    use super::speed;
    use wasm_bindgen::{JsCast, prelude::*};

    pub fn start() {
        let Ok(scope) = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>() else {
            return;
        };
        let worker = scope.clone();
        let on_message =
            Closure::<dyn Fn(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                let games = event.data().as_f64().unwrap_or(1_000.0) as u64;
                let post = |line: String| {
                    let _ = worker.post_message(&line.into());
                };
                for line in speed::quick_checks() {
                    post(line);
                }
                let mut run = speed::SolverRun::new(games);
                while !run.finished() {
                    if let Some(line) = run.advance() {
                        post(line);
                    }
                }
                post(run.summary());
                let _ = worker.post_message(&JsValue::NULL);
            });
        scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();
        let _ = scope.post_message(&JsValue::TRUE);
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
