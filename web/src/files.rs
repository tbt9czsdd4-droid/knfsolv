//! Position files in the browser ([`Browser`]): sharing goes through the share sheet where the
//! browser offers one for files (iOS), otherwise through a download; opening uses the file
//! picker. Both must start while a tap is being handled, which eframe arranges by running the
//! app inside its `pointerup` handler.
use eframe::egui;
use knfsolv::{Error, Result, gui::Browser};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

/// Larger files cannot be positions; reading them could only waste memory.
const MAX_BYTES: f64 = 1_048_576.0;

#[derive(Clone)]
pub struct WebFiles(Rc<Inner>);

struct Inner {
    ctx: egui::Context,
    opened: RefCell<Option<std::result::Result<String, String>>>,
    /// Created on first use and kept, so picking the same file again still reports it.
    picker: RefCell<Option<web_sys::HtmlInputElement>>,
}

impl WebFiles {
    pub fn new(ctx: egui::Context) -> Self {
        Self(Rc::new(Inner {
            ctx,
            opened: RefCell::new(None),
            picker: RefCell::new(None),
        }))
    }

    fn picker(&self) -> std::result::Result<web_sys::HtmlInputElement, JsValue> {
        if let Some(picker) = &*self.0.picker.borrow() {
            return Ok(picker.clone());
        }
        let document = document()?;
        let picker: web_sys::HtmlInputElement = document.create_element("input")?.dyn_into()?;
        picker.set_type("file");
        picker.set_accept(".json,application/json");
        // Off screen but in the document: some Safari versions ignore detached pickers.
        picker.set_attribute("style", "position:fixed;left:-1000px;opacity:0")?;
        document.body().ok_or("no body")?.append_child(&picker)?;
        let weak = Rc::downgrade(&self.0);
        let on_change = Closure::<dyn Fn()>::new(move || {
            if let Some(inner) = weak.upgrade() {
                WebFiles(inner).picked();
            }
        });
        picker.set_onchange(Some(on_change.as_ref().unchecked_ref()));
        on_change.forget();
        *self.0.picker.borrow_mut() = Some(picker.clone());
        Ok(picker)
    }

    fn picked(&self) {
        let Some(picker) = self.0.picker.borrow().clone() else {
            return;
        };
        let Some(file) = picker.files().and_then(|files| files.get(0)) else {
            return;
        };
        picker.set_value("");
        if file.size() > MAX_BYTES {
            self.finish(Err("The file is larger than 1 MiB.".into()));
            return;
        }
        let files = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let text = wasm_bindgen_futures::JsFuture::from(file.text()).await;
            files.finish(
                text.ok()
                    .and_then(|text| text.as_string())
                    .ok_or_else(|| "The file could not be read.".to_string()),
            );
        });
    }

    fn finish(&self, result: std::result::Result<String, String>) {
        *self.0.opened.borrow_mut() = Some(result);
        self.0.ctx.request_repaint();
    }
}

impl Browser for WebFiles {
    fn share(&self, json: &str) -> Result<()> {
        let name = file_name();
        let parts = js_sys::Array::of1(&JsValue::from_str(json));
        let options = web_sys::FilePropertyBag::new();
        options.set_type("application/json");
        let file = web_sys::File::new_with_str_sequence_and_options(&parts, &name, &options)
            .map_err(js_error)?;
        let navigator = web_sys::window()
            .ok_or("no window")
            .map_err(js_error)?
            .navigator();
        let data = web_sys::ShareData::new();
        data.set_files(&js_sys::Array::of1(&file));
        data.set_title("Kniffel-Stellung");
        // Desktop browsers often lack file sharing (`canShare` may not even exist).
        if js_sys::Reflect::has(&navigator, &"canShare".into()).unwrap_or(false)
            && navigator.can_share_with_data(&data)
        {
            let shared = navigator.share_with_data(&data);
            // Closing the sheet without choosing a target rejects the promise; nothing to do.
            wasm_bindgen_futures::spawn_local(async move {
                let _ = wasm_bindgen_futures::JsFuture::from(shared).await;
            });
            return Ok(());
        }
        download(&file, &name).map_err(js_error)
    }

    fn open(&self) {
        match self.picker() {
            Ok(picker) => picker.click(),
            Err(error) => self.finish(Err(format!("The file picker is unavailable. {error:?}"))),
        }
    }

    fn take_opened(&self) -> Option<std::result::Result<String, String>> {
        self.0.opened.borrow_mut().take()
    }
}

/// For example `kniffel-stellung-2026-10-05-2130.json`, in local time.
fn file_name() -> String {
    let now = js_sys::Date::new_0();
    format!(
        "kniffel-stellung-{:04}-{:02}-{:02}-{:02}{:02}.json",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date(),
        now.get_hours(),
        now.get_minutes()
    )
}

fn download(file: &web_sys::File, name: &str) -> std::result::Result<(), JsValue> {
    let document = document()?;
    let url = web_sys::Url::create_object_url_with_blob(file)?;
    let link: web_sys::HtmlAnchorElement = document.create_element("a")?.dyn_into()?;
    link.set_href(&url);
    link.set_download(name);
    document.body().ok_or("no body")?.append_child(&link)?;
    link.click();
    link.remove();
    // The download may still read the URL for a moment after the click.
    let revoke = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    web_sys::window()
        .ok_or("no window")?
        .set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 60_000)?;
    Ok(())
}

fn document() -> std::result::Result<web_sys::Document, JsValue> {
    web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| "no document".into())
}

fn js_error(error: impl std::fmt::Debug) -> Error {
    Error::Invalid(format!("The position could not be shared. {error:?}"))
}
