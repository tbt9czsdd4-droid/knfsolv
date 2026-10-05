//! The interface's [`Computation`] in the browser: analyses run in a Web Worker
//! (`src/bin/worker.rs`), so the page stays responsive and exact endgames get their full time.
//!
//! The page keeps its own [`Scheduler`] as the record of current requests and accepted reports.
//! A newly started worker gets those requests replayed once it reports `Ready`, which also
//! covers restarts after a crash. While the page is hidden, every request is paused.
use crate::{
    page,
    protocol::{self, FromWorker, ToWorker, VERSION},
};
use knfsolv::{
    Result,
    coaching::{AnalysisKey, AnalysisRequest, AnalysisUpdate, Computation, Scheduler, Wake},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

/// Crashes within this window count towards giving up instead of restarting again.
const CRASH_WINDOW_MS: f64 = 60_000.0;
const MAX_CRASHES: usize = 3;

/// Cheap to clone; all clones control the same worker.
#[derive(Clone)]
pub struct WorkerComputation(Rc<Inner>);

struct Inner {
    scheduler: Scheduler,
    wake: Wake,
    worker: RefCell<Option<Connection>>,
    /// The previous worker's callbacks; a crash report may still be running inside them.
    retired: RefCell<Option<Connection>>,
    /// Identifies the current worker, so events from a replaced one are ignored.
    generation: Cell<u32>,
    crashes: RefCell<Vec<f64>>,
    hidden: Cell<bool>,
    /// Accepted reports, for the `?debug` panel.
    reports: Cell<u64>,
}

struct Connection {
    worker: web_sys::Worker,
    ready: bool,
    _on_message: Closure<dyn Fn(web_sys::MessageEvent)>,
    _on_error: Closure<dyn Fn(web_sys::ErrorEvent)>,
}

impl WorkerComputation {
    pub fn start(wake: Wake) -> Self {
        let document = web_sys::window().and_then(|w| w.document());
        let computation = Self(Rc::new(Inner {
            scheduler: Scheduler::default(),
            wake,
            worker: RefCell::new(None),
            retired: RefCell::new(None),
            generation: Cell::new(0),
            crashes: RefCell::new(Vec::new()),
            hidden: Cell::new(document.as_ref().is_some_and(|d| d.hidden())),
            reports: Cell::new(0),
        }));
        computation.spawn();
        if let Some(document) = document {
            let weak = Rc::downgrade(&computation.0);
            let on_visibility = Closure::<dyn Fn()>::new(move || {
                if let Some(inner) = weak.upgrade() {
                    let hidden = web_sys::window()
                        .and_then(|w| w.document())
                        .is_some_and(|d| d.hidden());
                    WorkerComputation(inner).set_hidden(hidden);
                }
            });
            let _ = document.add_event_listener_with_callback(
                "visibilitychange",
                on_visibility.as_ref().unchecked_ref(),
            );
            on_visibility.forget();
        }
        computation
    }

    /// Debug aid: lets the worker panic, which must lead to a quiet restart.
    pub fn crash_worker(&self) {
        self.send(&ToWorker::Crash);
    }

    /// Crashes within the last minute, accepted reports, and whether work is paused as hidden.
    pub fn diagnostics(&self) -> (usize, u64, bool) {
        (
            self.0.crashes.borrow().len(),
            self.0.reports.get(),
            self.0.hidden.get(),
        )
    }

    fn spawn(&self) {
        let generation = self.0.generation.get() + 1;
        self.0.generation.set(generation);
        let worker = match web_sys::Worker::new("./worker_loader.js") {
            Ok(worker) => worker,
            Err(error) => return self.crashed(generation, &format!("{error:?}")),
        };
        let weak = Rc::downgrade(&self.0);
        let on_message = Closure::<dyn Fn(web_sys::MessageEvent)>::new({
            let weak = weak.clone();
            move |event: web_sys::MessageEvent| {
                if let (Some(inner), Some(json)) = (weak.upgrade(), event.data().as_string()) {
                    WorkerComputation(inner).received(generation, &json);
                }
            }
        });
        let on_error =
            Closure::<dyn Fn(web_sys::ErrorEvent)>::new(move |event: web_sys::ErrorEvent| {
                if let Some(inner) = weak.upgrade() {
                    WorkerComputation(inner).crashed(generation, &event.message());
                }
            });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        *self.0.worker.borrow_mut() = Some(Connection {
            worker,
            ready: false,
            _on_message: on_message,
            _on_error: on_error,
        });
    }

    fn received(&self, generation: u32, json: &str) {
        if generation != self.0.generation.get() {
            return;
        }
        match protocol::decode::<FromWorker>(json) {
            Ok(FromWorker::Ready { version }) if version == VERSION => {
                if let Some(connection) = &mut *self.0.worker.borrow_mut() {
                    connection.ready = true;
                }
                for request in self.0.scheduler.requests() {
                    self.send(&ToWorker::Submit(Box::new(self.effective(request))));
                }
            }
            Ok(FromWorker::Ready { version }) => {
                self.stop();
                page::show_error(
                    "Die App wurde aktualisiert. Bitte neu laden.",
                    &format!("Worker version {version}, page version {VERSION}"),
                );
            }
            Ok(FromWorker::Update(update)) => {
                if self.0.scheduler.accept(*update) {
                    self.0.reports.set(self.0.reports.get() + 1);
                    (self.0.wake)();
                }
            }
            Ok(FromWorker::Panicked(details)) => self.crashed(generation, &details),
            Ok(FromWorker::Line(_) | FromWorker::SpeedDone) => {}
            Err(error) => log::warn!("Unreadable worker message: {error}"),
        }
    }

    /// Restarts the worker with the current requests, unless it keeps crashing.
    fn crashed(&self, generation: u32, details: &str) {
        if generation != self.0.generation.get() {
            return;
        }
        log::warn!("Analysis worker crashed: {details}");
        self.stop();
        let now = js_sys::Date::now();
        let mut crashes = self.0.crashes.borrow_mut();
        crashes.retain(|&at| now - at < CRASH_WINDOW_MS);
        crashes.push(now);
        if crashes.len() >= MAX_CRASHES {
            page::show_error("Die Berechnung ist mehrmals abgestürzt.", details);
            return;
        }
        drop(crashes);
        self.spawn();
    }

    fn stop(&self) {
        if let Some(connection) = self.0.worker.borrow_mut().take() {
            connection.worker.terminate();
            self.0.retired.replace(Some(connection));
        }
        // Late events from the stopped worker no longer match.
        self.0.generation.set(self.0.generation.get() + 1);
    }

    /// Messages before `Ready` are dropped; `Ready` replays the requests instead.
    fn send(&self, message: &ToWorker) {
        if let Some(connection) = &*self.0.worker.borrow()
            && connection.ready
        {
            let _ = connection
                .worker
                .post_message(&JsValue::from_str(&protocol::encode(message)));
        }
    }

    fn effective(&self, mut request: AnalysisRequest) -> AnalysisRequest {
        request.paused |= self.0.hidden.get();
        request
    }

    fn set_hidden(&self, hidden: bool) {
        if self.0.hidden.replace(hidden) == hidden {
            return;
        }
        (self.0.wake)();
        for request in self.0.scheduler.requests() {
            self.send(&ToWorker::Controls {
                key: request.key,
                continuous: request.continuous,
                paused: request.paused || hidden,
            });
        }
    }
}

impl Computation for WorkerComputation {
    fn submit(&self, request: AnalysisRequest) -> Result<()> {
        self.0.scheduler.submit(request.clone())?;
        self.send(&ToWorker::Submit(Box::new(self.effective(request))));
        Ok(())
    }
    fn controls(&self, key: AnalysisKey, continuous: bool, paused: bool) {
        self.0.scheduler.controls(key, continuous, paused);
        self.send(&ToWorker::Controls {
            key,
            continuous,
            paused: paused || self.0.hidden.get(),
        });
    }
    fn clear(&self) {
        self.0.scheduler.clear();
        self.send(&ToWorker::Clear);
    }
    fn take_updates(&self) -> Vec<AnalysisUpdate> {
        self.0.scheduler.take_updates()
    }
}
