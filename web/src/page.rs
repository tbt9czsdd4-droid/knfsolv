//! The page around the canvas (see `index.html`): loading screen, error overlay, and the
//! plain-text report of the speed test.
use wasm_bindgen::JsCast;

fn element(id: &str) -> Option<web_sys::HtmlElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id(id)?
        .dyn_into()
        .ok()
}

/// Once removed, the page's own script no longer reports errors as failed loading.
pub fn hide_loading() {
    if let Some(loading) = element("loading") {
        loading.remove();
    }
}

/// Covers the app with a German summary, a reload button, and copyable technical details.
pub fn show_error(summary: &str, details: &str) {
    let (Some(error), Some(text), Some(pre)) = (
        element("error"),
        element("error-summary"),
        element("error-details"),
    ) else {
        return;
    };
    text.set_text_content(Some(summary));
    pre.set_text_content(Some(details));
    error.set_hidden(false);
    hide_loading();
}

pub fn show_report(text: &str) {
    if let Some(report) = element("report") {
        report.set_text_content(Some(text));
        report.set_hidden(false);
    }
    hide_loading();
}
