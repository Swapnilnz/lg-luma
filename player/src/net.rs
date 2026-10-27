//! Sends luma-core's requests with `fetch`.

use crate::dom::{Timer, window};
use luma_core::http::Request;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortController, Headers, RequestInit, RequestMode, Response};

pub struct Reply {
    pub status: u16,
    pub text: String,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Why a request failed without a reply.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    Timeout,
    Network,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Failure::Timeout => "timed out",
            Failure::Network => "network error",
        })
    }
}

fn init(request: &Request, keepalive: bool) -> RequestInit {
    let init = RequestInit::new();
    init.set_method(request.method.as_str());
    if let Ok(headers) = Headers::new() {
        for (name, value) in &request.headers {
            let _ = headers.set(name, value);
        }
        init.set_headers(&headers);
    }
    if let Some(body) = &request.body {
        init.set_body(&JsValue::from_str(body));
    }
    if keepalive {
        // Not bound by web-sys: a plain dictionary field.
        crate::dom::set(init.as_ref(), "keepalive", &JsValue::TRUE);
    }
    init
}

async fn fetch(request: &Request, timeout_ms: i32) -> Result<Response, Failure> {
    let init = init(request, false);
    let controller = AbortController::new().ok();
    if let Some(controller) = &controller {
        init.set_signal(Some(&controller.signal()));
    }
    let timer = controller
        .clone()
        .map(|controller| Timer::new(timeout_ms, move || controller.abort()));
    let result = JsFuture::from(window().fetch_with_str_and_init(&request.url, &init)).await;
    let timed_out = timer.as_ref().is_some_and(Timer::fired);
    drop(timer);
    match result {
        Ok(value) => value.dyn_into::<Response>().map_err(|_| Failure::Network),
        Err(_) if timed_out => Err(Failure::Timeout),
        Err(_) => Err(Failure::Network),
    }
}

/// The reply's status and text.
pub async fn send(request: &Request, timeout_ms: i32) -> Result<Reply, Failure> {
    let response = fetch(request, timeout_ms).await?;
    let status = response.status();
    let text = match response.text() {
        Ok(promise) => JsFuture::from(promise)
            .await
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default(),
        Err(_) => String::new(),
    };
    Ok(Reply { status, text })
}

/// The reply's bytes (subtitle files, which may not be UTF-8).
pub async fn bytes(url: &str, timeout_ms: i32) -> Result<(u16, Vec<u8>), Failure> {
    let response = fetch(&Request::get(url), timeout_ms).await?;
    let status = response.status();
    let buffer = match response.array_buffer() {
        Ok(promise) => JsFuture::from(promise)
            .await
            .map_err(|_| Failure::Network)?,
        Err(_) => return Err(Failure::Network),
    };
    Ok((status, js_sys::Uint8Array::new(&buffer).to_vec()))
}

/// Fire and forget, surviving the app closing (`keepalive`): exit scrobbles.
pub fn keepalive(request: &Request) {
    let init = init(request, true);
    let _ = window().fetch_with_str_and_init(&request.url, &init);
}

/// A plain-text POST that needs no CORS answer (the dev log server).
pub fn post_no_cors(url: &str, body: &str) {
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_mode(RequestMode::NoCors);
    init.set_body(&JsValue::from_str(body));
    if let Ok(headers) = Headers::new() {
        let _ = headers.set("Content-Type", "text/plain");
        init.set_headers(&headers);
    }
    let _ = window().fetch_with_str_and_init(url, &init);
}
