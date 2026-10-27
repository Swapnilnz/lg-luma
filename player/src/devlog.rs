//! The player's log: the console always, and in dev builds the laptop too
//! (`node tools/devlog.mjs`), with URLs cut to their host so no debrid link or AIOStreams
//! config ever reaches a terminal.

use std::cell::RefCell;

thread_local! {
    static BUFFER: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static URL: RefCell<Option<String>> = const { RefCell::new(None) };
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::devlog::line(&format!($($arg)*)) };
}

/// Cuts every URL to `scheme://host/…`.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("http") {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        let scheme_end = if from.starts_with("https://") {
            8
        } else if from.starts_with("http://") {
            7
        } else {
            out.push_str("http");
            rest = &from[4..];
            continue;
        };
        let tail = &from[scheme_end..];
        let url_len = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>' | ','))
            .unwrap_or(tail.len());
        let url = &tail[..url_len];
        let host = url.split(['/', '?', '#']).next().unwrap_or_default();
        out.push_str(&from[..scheme_end]);
        out.push_str(host);
        if url.len() > host.len() {
            out.push_str("/…");
        }
        rest = &tail[url_len..];
    }
    out.push_str(rest);
    out
}

pub fn line(text: &str) {
    let text = redact(text);
    web_sys::console::log_1(&text.as_str().into());
    if cfg!(feature = "dev") && URL.with(|u| u.borrow().is_some()) {
        let stamp = js_sys::Date::new_0()
            .to_iso_string()
            .as_string()
            .unwrap_or_default();
        let stamp = stamp.get(11..23).unwrap_or_default().to_owned();
        BUFFER.with(|b| {
            let mut buffer = b.borrow_mut();
            buffer.push(format!("{stamp} {text}"));
            if buffer.len() > 500 {
                buffer.remove(0);
            }
        });
    }
}

pub fn flush() {
    let Some(url) = URL.with(|u| u.borrow().clone()) else {
        return;
    };
    let lines = BUFFER.with(|b| std::mem::take(&mut *b.borrow_mut()));
    if !lines.is_empty() {
        crate::net::post_no_cors(&url, &lines.join("\n"));
    }
}

/// Panics go to the console with their file and line, and to the laptop at once.
pub fn init(log_url: Option<String>) {
    if cfg!(feature = "dev") {
        URL.with(|u| *u.borrow_mut() = log_url);
        crate::dom::every(1000, flush);
    }
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        line(&format!("PANIC {info}"));
        flush();
    }));
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn cuts_urls_to_their_host() {
        assert_eq!(
            redact("GET https://aio.example/stremio/secret/manifest.json failed"),
            "GET https://aio.example/… failed"
        );
        assert_eq!(
            redact("src=\"http://rd.host/dl/x.mkv\""),
            "src=\"http://rd.host/…\""
        );
        assert_eq!(redact("httpd and https://a.b"), "httpd and https://a.b");
    }
}
