//! Small browser helpers: elements, timers, events, the clock.

use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Document, HtmlElement, Window};

pub fn window() -> Window {
    web_sys::window().expect("no window")
}

pub fn document() -> Document {
    window().document().expect("no document")
}

pub fn by_id(id: &str) -> Option<HtmlElement> {
    document()
        .get_element_by_id(id)
        .and_then(|el| el.dyn_into::<HtmlElement>().ok())
}

pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

thread_local! {
    /// The HTML last written to each layer. Compared instead of `innerHTML`, which the
    /// browser re-serialises (`&#39;` comes back as `'`): reading it back made titles with
    /// an apostrophe rewrite their layer, and reload its images, on every tick.
    static WRITTEN: std::cell::RefCell<std::collections::HashMap<String, String>> =
        Default::default();
}

/// Like `set_html`, for a layer that holds another updated element (`child`): rewriting the
/// layer makes a fresh, empty child, so the child's cached HTML is forgotten too.
pub fn set_html_with(id: &str, html: &str, child: &str) {
    if set_html(id, html) {
        WRITTEN.with(|w| w.borrow_mut().remove(child));
    }
}

/// Sets an element's HTML only when it changed, so the TV doesn't lay out for nothing.
/// True when it was written.
pub fn set_html(id: &str, html: &str) -> bool {
    let changed = WRITTEN.with(|w| {
        let mut written = w.borrow_mut();
        if written.get(id).map(String::as_str) == Some(html) {
            false
        } else {
            written.insert(id.to_owned(), html.to_owned());
            true
        }
    });
    if changed && let Some(el) = by_id(id) {
        el.set_inner_html(html);
    }
    changed
}

/// Sets a CSS custom property on an element (only when it changed).
pub fn set_var(id: &str, name: &str, value: &str) {
    if let Some(el) = by_id(id) {
        let style = el.style();
        if style.get_property_value(name).ok().as_deref() != Some(value) {
            let _ = style.set_property(name, value);
        }
    }
}

pub fn set_class(id: &str, class: &str) {
    if let Some(el) = by_id(id)
        && el.class_name() != class
    {
        el.set_class_name(class);
    }
}

/// Text for HTML: `&`, `<`, `>`, `"` and `'` escaped.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            ch => out.push(ch),
        }
    }
    out
}

/// Runs `f` every `ms` for the life of the page.
pub fn every(ms: i32, f: impl FnMut() + 'static) {
    let closure = Closure::<dyn FnMut()>::new(f);
    let _ = window().set_interval_with_callback_and_timeout_and_arguments_0(
        closure.as_ref().unchecked_ref(),
        ms,
    );
    closure.forget();
}

/// Calls `f` on `event` from `target` for the life of the page.
pub fn listen(target: &web_sys::EventTarget, event: &str, f: impl FnMut(web_sys::Event) + 'static) {
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(f);
    let _ = target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
    closure.forget();
}

/// A one-shot timer, cancelled when dropped.
pub struct Timer {
    handle: i32,
    fired: Rc<Cell<bool>>,
}

impl Timer {
    pub fn new(ms: i32, f: impl FnOnce() + 'static) -> Self {
        let fired = Rc::new(Cell::new(false));
        let flag = fired.clone();
        let closure = Closure::once_into_js(move || {
            flag.set(true);
            f();
        });
        let handle = window()
            .set_timeout_with_callback_and_timeout_and_arguments_0(closure.unchecked_ref(), ms)
            .unwrap_or(0);
        Self { handle, fired }
    }

    pub fn fired(&self) -> bool {
        self.fired.get()
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if !self.fired.get() {
            window().clear_timeout_with_handle(self.handle);
        }
    }
}

pub async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(promise).await;
}

/// A property of a JS object, for the LG and media APIs web-sys doesn't bind.
pub fn get(target: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(target, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

pub fn set(target: &JsValue, key: &str, value: &JsValue) {
    let _ = js_sys::Reflect::set(target, &JsValue::from_str(key), value);
}

/// `PalmSystem`, on a TV.
pub fn palm() -> Option<JsValue> {
    let palm = get(&window(), "PalmSystem");
    (!palm.is_undefined() && !palm.is_null()).then_some(palm)
}

/// Calls `PalmSystem.<method>()` when there is one.
pub fn palm_call(method: &str) {
    if let Some(palm) = palm()
        && let Ok(f) = get(&palm, method).dyn_into::<js_sys::Function>()
    {
        let _ = f.call0(&palm);
    }
}
