//! Browser storage for the documents the desktop app keeps in files.
use knfsolv::{Error, Result, persistence::Storage};
use wasm_bindgen::JsValue;

pub const POSITION: &str = "kniffel-coach/position";
pub const PREFERENCES: &str = "kniffel-coach/preferences";

/// One `localStorage` entry. Safari keeps it separately for the Home Screen app and for tabs.
pub struct LocalStorage(pub &'static str);

impl Storage for LocalStorage {
    fn read(&self) -> Result<Option<String>> {
        local_storage()?.get_item(self.0).map_err(unavailable)
    }
    fn write(&self, contents: &str) -> Result<()> {
        local_storage()?
            .set_item(self.0, contents)
            .map_err(unavailable)
    }
}

fn local_storage() -> Result<web_sys::Storage> {
    web_sys::window()
        .ok_or_else(|| unavailable(JsValue::NULL))?
        .local_storage()
        .map_err(unavailable)?
        .ok_or_else(|| unavailable(JsValue::NULL))
}

fn unavailable(error: JsValue) -> Error {
    Error::Invalid(format!(
        "Der Speicher des Browsers ist nicht verfügbar. {error:?}"
    ))
}
