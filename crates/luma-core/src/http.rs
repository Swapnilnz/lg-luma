//! A request as data: built here, sent by each app.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Delete => "DELETE",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// JSON text.
    pub body: Option<String>,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: vec![],
            body: None,
        }
    }

    pub fn post(url: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(body.into()),
        }
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// Percent-encodes one URL component (RFC 3986 unreserved characters pass through).
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Decodes `%XX` and `+` in a query value; bad escapes are kept as they are.
pub fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A query parameter's decoded value from a `?a=b&c=d` string (with or without the `?`).
pub fn query_param(query: &str, name: &str) -> Option<String> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| decode(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_round_trip() {
        let raw = r#"{"type":"movie","title":"Amélie & co"}"#;
        assert_eq!(decode(&encode(raw)), raw);
        assert_eq!(encode("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn reads_query_params() {
        assert_eq!(
            query_param("?dev&t=3600&x=a%20b", "t").as_deref(),
            Some("3600")
        );
        assert_eq!(query_param("dev&x=a%20b", "x").as_deref(), Some("a b"));
        assert_eq!(query_param("?dev", "dev").as_deref(), Some(""));
        assert_eq!(query_param("?dev", "mock"), None);
        // A truncated escape is kept, not dropped.
        assert_eq!(decode("50%"), "50%");
        assert_eq!(decode("50%2"), "50%2");
    }
}
