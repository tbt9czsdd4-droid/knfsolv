//! The analysis Web Worker. It runs the page's analyses with the engine's own scheduler, one
//! slice at a time, and between slices returns to the event loop so newer messages can replace
//! or pause the work. The page side is `src/computation.rs`; messages are in `src/protocol.rs`.

#[cfg(target_arch = "wasm32")]
#[path = "../protocol.rs"]
mod protocol;
#[cfg(target_arch = "wasm32")]
#[path = "../speed.rs"]
mod speed;

#[cfg(target_arch = "wasm32")]
mod web {
    use super::{
        protocol::{self, FromWorker, ToWorker, VERSION},
        speed,
    };
    use knfsolv::{
        coaching::{Runner, Scheduler, Step},
        solver::WorkSlice,
    };
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};
    use web_sys::{DedicatedWorkerGlobalScope, MessageChannel, MessageEvent};

    struct Engine {
        scheduler: Scheduler,
        runner: Runner,
        /// A slice is queued on `wake`, so the next one runs right after pending messages.
        queued: bool,
        wake: MessageChannel,
    }

    fn post(scope: &DedicatedWorkerGlobalScope, message: &FromWorker) {
        let _ = scope.post_message(&JsValue::from_str(&protocol::encode(message)));
    }

    /// A message posted to our own channel runs after messages already waiting, without the
    /// minimum delay browsers add to nested timers.
    fn queue_slice(engine: &mut Engine) {
        if !engine.queued {
            engine.queued = true;
            let _ = engine.wake.port2().post_message(&JsValue::NULL);
        }
    }

    pub fn start() {
        let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
        let panic_scope = scope.clone();
        std::panic::set_hook(Box::new(move |info| {
            post(&panic_scope, &FromWorker::Panicked(info.to_string()));
        }));
        let engine = Rc::new(RefCell::new(Engine {
            scheduler: Scheduler::default(),
            runner: Runner::default(),
            queued: false,
            wake: MessageChannel::new().expect("message channel"),
        }));

        let on_slice = Closure::<dyn Fn(MessageEvent)>::new({
            let (engine, scope) = (engine.clone(), scope.clone());
            move |_: MessageEvent| {
                let mut engine = engine.borrow_mut();
                engine.queued = false;
                let Engine {
                    scheduler, runner, ..
                } = &mut *engine;
                let step = runner.step(scheduler, WorkSlice::default(), &|| {});
                for update in scheduler.take_updates() {
                    post(&scope, &FromWorker::Update(Box::new(update)));
                }
                if matches!(step, Step::Worked) {
                    queue_slice(&mut engine);
                }
            }
        });
        engine
            .borrow()
            .wake
            .port1()
            .set_onmessage(Some(on_slice.as_ref().unchecked_ref()));
        on_slice.forget();

        let on_message = Closure::<dyn Fn(MessageEvent)>::new({
            let scope = scope.clone();
            move |event: MessageEvent| {
                let Some(json) = event.data().as_string() else {
                    return;
                };
                let message = match protocol::decode::<ToWorker>(&json) {
                    Ok(message) => message,
                    Err(error) => {
                        log(&format!("Unreadable message: {error}"));
                        return;
                    }
                };
                let mut engine = engine.borrow_mut();
                match message {
                    ToWorker::Submit(request) => {
                        // The page validated the request against the same rules already.
                        if let Err(error) = engine.scheduler.submit(*request) {
                            log(&format!("Request rejected: {error}"));
                        }
                    }
                    ToWorker::Controls {
                        key,
                        continuous,
                        paused,
                    } => engine.scheduler.controls(key, continuous, paused),
                    ToWorker::Clear => engine.scheduler.clear(),
                    ToWorker::SpeedTest { games } => run_speed_test(&scope, games),
                    ToWorker::Crash => panic!("Absichtlicher Absturz des Rechenkerns"),
                }
                // Even after a clear, one step drops the replaced searches.
                queue_slice(&mut engine);
            }
        });
        scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();
        post(&scope, &FromWorker::Ready { version: VERSION });
    }

    fn run_speed_test(scope: &DedicatedWorkerGlobalScope, games: u64) {
        let line = |text: String| post(scope, &FromWorker::Line(text));
        for text in speed::quick_checks() {
            line(text);
        }
        let mut run = speed::SolverRun::new(games);
        while !run.finished() {
            if let Some(text) = run.advance() {
                line(text);
            }
        }
        line(run.summary());
        post(scope, &FromWorker::SpeedDone);
    }

    fn log(text: &str) {
        web_sys::console::warn_1(&JsValue::from_str(text));
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
