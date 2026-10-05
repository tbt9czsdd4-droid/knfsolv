//! Opens the iOS keyboard on the first tap into a text field.
//!
//! iOS shows the keyboard only when an input gets focus while a tap is being handled, as in
//! `touchend`. egui often focuses a text field only after the release, in a later frame, so
//! eframe focuses its hidden text input too late; its own `touchend` fix only helps once the
//! input already has focus, which is why a second tap used to be needed.
//!
//! For a tap on a text field this module therefore keeps the page unfocused during the touch,
//! then in `touchend` focuses the input and tells egui which field it belongs to, so the next
//! frame does not move focus away again. eframe's `pointerup` handling, which would move focus
//! to the canvas and back, is held back for that tap; egui still gets the release from eframe's
//! `touchend` handler.
use eframe::egui;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

/// Text fields of the latest frame, in egui points.
#[derive(Default)]
pub struct TextFields {
    fields: Vec<(egui::Id, egui::Rect)>,
    zoom: f32,
    ctx: Option<egui::Context>,
}
pub type Shared = Rc<RefCell<TextFields>>;

/// Call at the end of each frame. Only `TextEdit` keeps a `TextEditState` under its widget id.
pub fn record(ctx: &egui::Context, fields: &Shared) {
    let widgets: Vec<_> = ctx.viewport(|viewport| {
        viewport
            .this_pass
            .widgets
            .layers()
            .flat_map(|(_, widgets)| widgets.iter())
            .filter(|widget| widget.enabled)
            .map(|widget| (widget.id, widget.layer_id, widget.interact_rect))
            .collect()
    });
    let found = widgets
        .into_iter()
        .filter(|&(id, _, _)| egui::TextEdit::load_state(ctx, id).is_some())
        .map(|(id, layer, rect)| {
            let rect = ctx
                .layer_transform_to_global(layer)
                .map_or(rect, |transform| transform * rect);
            (id, rect)
        })
        .collect();
    *fields.borrow_mut() = TextFields {
        fields: found,
        zoom: ctx.zoom_factor(),
        ctx: Some(ctx.clone()),
    };
}

/// Where and when a touch on a text field began, and which field it was.
#[derive(Clone, Copy)]
struct FieldTouch {
    x: f64,
    y: f64,
    time: f64,
    id: egui::Id,
}

/// Listens in the capture phase on `window`, before any of eframe's handlers.
pub fn install(canvas: &web_sys::HtmlCanvasElement, fields: Shared) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    // eframe appends its text agent, the page's only <input>, to <body>.
    let input = {
        let document = document.clone();
        move || {
            document
                .query_selector("body > input")
                .ok()
                .flatten()
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        }
    };
    let field_at = {
        let (canvas, fields) = (canvas.clone(), fields.clone());
        move |x: f64, y: f64| {
            let fields = fields.borrow();
            let bounds = canvas.get_bounding_client_rect();
            let zoom = f64::from(fields.zoom.max(0.01));
            let pos = egui::pos2(
                ((x - bounds.left()) / zoom) as f32,
                ((y - bounds.top()) / zoom) as f32,
            );
            fields
                .fields
                .iter()
                .find(|(_, rect)| rect.contains(pos))
                .map(|&(id, _)| id)
        }
    };
    // Kept until the next touch, because browsers order `pointerup` and `touchend` differently.
    let field_touch = Rc::new(Cell::new(None::<FieldTouch>));

    let on_down = Closure::<dyn Fn(web_sys::PointerEvent)>::new({
        let (field_touch, document, input) = (field_touch.clone(), document.clone(), input.clone());
        move |event: web_sys::PointerEvent| {
            if event.pointer_type() != "touch" || !event.is_primary() {
                return;
            }
            let (x, y) = (event.client_x() as f64, event.client_y() as f64);
            let touch = field_at(x, y).map(|id| FieldTouch {
                x,
                y,
                time: event.time_stamp(),
                id,
            });
            field_touch.set(touch);
            // With neither canvas nor input focused, eframe leaves the input alone until touchend.
            let active = document.active_element();
            if touch.is_some()
                && active.as_ref() != input().as_deref().map(AsRef::as_ref)
                && let Some(active) = active.and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = active.blur();
            }
        }
    });
    let on_up = Closure::<dyn Fn(web_sys::PointerEvent)>::new({
        let field_touch = field_touch.clone();
        move |event: web_sys::PointerEvent| {
            if event.pointer_type() == "touch" && field_touch.get().is_some() {
                event.stop_propagation();
            }
        }
    });
    // A tap must not move far or last long; egui treats those as scrolling or a long press.
    let on_end = Closure::<dyn Fn(web_sys::TouchEvent)>::new(move |event: web_sys::TouchEvent| {
        let (Some(start), Some(touch)) = (field_touch.get(), event.changed_touches().get(0)) else {
            return;
        };
        let (x, y) = (touch.client_x() as f64, touch.client_y() as f64);
        if (x - start.x).hypot(y - start.y) > 10.0 || event.time_stamp() - start.time > 600.0 {
            return;
        }
        if let Some(input) = input() {
            let _ = input.focus();
        }
        if let Some(ctx) = &fields.borrow().ctx {
            ctx.memory_mut(|memory| memory.request_focus(start.id));
            ctx.request_repaint();
        }
    });
    for (name, listener) in [
        ("pointerdown", on_down.as_ref()),
        ("pointerup", on_up.as_ref()),
        ("touchend", on_end.as_ref()),
    ] {
        window.add_event_listener_with_callback_and_bool(name, listener.unchecked_ref(), true)?;
    }
    on_down.forget();
    on_up.forget();
    on_end.forget();
    Ok(())
}
