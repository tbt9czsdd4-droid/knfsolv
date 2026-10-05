//! Keeps the current position in browser storage: 300 ms after each change, and at once when
//! the page is hidden or the app panics, because iOS may end a background page without warning.
use crate::storage::LocalStorage;
use knfsolv::persistence::Storage;
use std::{cell::RefCell, time::Duration};
use wasm_bindgen::{JsCast, closure::Closure};
use web_time::Instant;

const DELAY: Duration = Duration::from_millis(300);

struct Autosave {
    storage: LocalStorage,
    latest: Option<String>,
    written: Option<String>,
    due: Option<Instant>,
}

thread_local! {
    // Shared with the page's event listeners and the panic hook.
    static AUTOSAVE: RefCell<Option<Autosave>> = const { RefCell::new(None) };
}

/// Starts saving into `storage`, which already holds `restored` if the app reopened a position.
pub fn start(storage: LocalStorage, restored: Option<String>) {
    AUTOSAVE.with(|autosave| {
        *autosave.borrow_mut() = Some(Autosave {
            storage,
            latest: restored.clone(),
            written: restored,
            due: None,
        });
    });
    let Some(window) = web_sys::window() else {
        return;
    };
    let on_leave = Closure::<dyn Fn()>::new(|| {
        let hidden = web_sys::window()
            .and_then(|w| w.document())
            .is_some_and(|d| d.hidden());
        if hidden {
            flush();
        }
    });
    if let Some(document) = window.document() {
        let _ = document.add_event_listener_with_callback(
            "visibilitychange",
            on_leave.as_ref().unchecked_ref(),
        );
    }
    let on_pagehide = Closure::<dyn Fn()>::new(|| {
        flush();
    });
    let _ =
        window.add_event_listener_with_callback("pagehide", on_pagehide.as_ref().unchecked_ref());
    on_leave.forget();
    on_pagehide.forget();
}

/// Call after every frame with the interface's current position; returns how long until the
/// pending save is due, so the caller can schedule a repaint.
pub fn observe(snapshot: Option<String>, now: Instant) -> Option<Duration> {
    AUTOSAVE.with(|autosave| {
        let mut autosave = autosave.try_borrow_mut().ok()?;
        let autosave = autosave.as_mut()?;
        if snapshot.is_some() && snapshot != autosave.latest {
            autosave.latest = snapshot;
            autosave.due = Some(now + DELAY);
        }
        let due = autosave.due?;
        if now >= due {
            autosave.write();
            return None;
        }
        Some(due - now)
    })
}

/// Writes a pending change now. Returns whether a position is stored afterwards.
pub fn flush() -> bool {
    AUTOSAVE.with(|autosave| {
        let Ok(mut autosave) = autosave.try_borrow_mut() else {
            return false;
        };
        let Some(autosave) = autosave.as_mut() else {
            return false;
        };
        autosave.write();
        autosave.written.is_some() && autosave.written == autosave.latest
    })
}

impl Autosave {
    fn write(&mut self) {
        self.due = None;
        let Some(latest) = &self.latest else {
            return;
        };
        if self.written.as_ref() == Some(latest) {
            return;
        }
        match self.storage.write(latest) {
            Ok(()) => self.written = Some(latest.clone()),
            Err(error) => log::warn!("Position not saved: {error}"),
        }
    }
}
