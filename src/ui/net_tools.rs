//! Network detail logic without UI: JWT, HAR export and text search.

use std::ops::Range;

use base64::Engine as _;
use chrono::{Local, TimeZone, Utc};
use gpui_kit::{HighlightStyle, rgb};
use serde_json::{Value, json};

use crate::engine::NetEntry;

// ───────────────────────── JWT ─────────────────────────

pub struct Jwt {
    /// Where it appeared: "Request · authorization", "Response · body"…
    pub source: String,
    pub token: String,
    pub header: String,
    pub payload: String,
    /// exp / iat / nbf in readable form, with the relative time.
    pub times: Vec<(String, String)>,
    pub expired: bool,
}

/// JWTs in headers, payload and response. The signature is not verified (no key available).
pub fn find_jwts(e: &NetEntry) -> Vec<Jwt> {
    let mut sources: Vec<(String, &str)> = vec![];
    for (k, v) in &e.request_headers {
        sources.push((format!("Request · {}", k.to_lowercase()), v));
    }
    if let Some(p) = &e.post_data {
        sources.push(("Request · payload".into(), p));
    }
    for (k, v) in &e.response_headers {
        sources.push((format!("Response · {}", k.to_lowercase()), v));
    }
    if let Some(b) = &e.body {
        sources.push(("Response · body".into(), b));
    }

    let mut out: Vec<Jwt> = vec![];
    for (source, text) in sources {
        let words =
            text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')));
        for token in words.filter(|w| w.starts_with("eyJ") && w.matches('.').count() == 2) {
            if out.iter().any(|j| j.token == token) {
                continue;
            }
            if let Some(jwt) = decode_jwt(&source, token) {
                out.push(jwt);
            }
        }
    }
    out
}

fn decode_jwt(source: &str, token: &str) -> Option<Jwt> {
    let mut parts = token.split('.');
    let part = |p: &str| -> Option<Value> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(p.trim_end_matches('='))
            .ok()?;
        serde_json::from_slice::<Value>(&bytes)
            .ok()
            .filter(Value::is_object)
    };
    let header = part(parts.next()?)?;
    header.get("alg")?;
    let payload = part(parts.next()?)?;

    let now = Utc::now().timestamp();
    let mut times = vec![];
    let mut expired = false;
    for (claim, label) in [("exp", "Expires"), ("iat", "Issued"), ("nbf", "Valid from")] {
        let Some(t) = payload.get(claim).and_then(Value::as_i64) else {
            continue;
        };
        let when = Local
            .timestamp_opt(t, 0)
            .single()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default();
        let rel = relative(t - now);
        if claim == "exp" && t <= now {
            expired = true;
        }
        times.push((format!("{label} ({claim})"), format!("{when}  ·  {rel}")));
    }
    Some(Jwt {
        source: source.into(),
        token: token.into(),
        header: serde_json::to_string_pretty(&header).ok()?,
        payload: serde_json::to_string_pretty(&payload).ok()?,
        times,
        expired,
    })
}

/// "in 12 min" / "3 h ago".
fn relative(secs: i64) -> String {
    let a = secs.abs();
    let amount = match a {
        0..60 => format!("{a} s"),
        60..3600 => format!("{} min", a / 60),
        3600..86400 => format!("{} h", a / 3600),
        _ => format!("{} d", a / 86400),
    };
    if secs >= 0 {
        format!("in {amount}")
    } else {
        format!("{amount} ago")
    }
}

// ───────────────────────── HAR ─────────────────────────

/// HAR 1.2 (what Chrome, Firefox, Charles, Insomnia… import).
pub fn har(entries: &[NetEntry]) -> Value {
    let headers = |h: &[(String, String)]| {
        h.iter()
            .map(|(k, v)| json!({ "name": k, "value": v }))
            .collect::<Vec<_>>()
    };
    let entries: Vec<Value> = entries
        .iter()
        .map(|e| {
            let time = e
                .end_ts
                .map(|end| (end - e.start_ts) * 1000.0)
                .unwrap_or(0.0);
            let started = Utc
                .timestamp_millis_opt((e.wall_time * 1000.0) as i64)
                .single()
                .map(|t| t.to_rfc3339())
                .unwrap_or_default();
            let query: Vec<Value> = e
                .url
                .split_once('?')
                .map(|(_, q)| q.split('#').next().unwrap_or(""))
                .unwrap_or("")
                .split('&')
                .filter(|s| !s.is_empty())
                .map(|kv| {
                    let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                    json!({ "name": k, "value": v })
                })
                .collect();
            let req_mime = header(&e.request_headers, "content-type").unwrap_or_default();
            let http = e.protocol.clone().unwrap_or_else(|| "HTTP/1.1".into());
            let mut request = json!({
                "method": e.method,
                "url": e.url,
                "httpVersion": http,
                "cookies": [],
                "headers": headers(&e.request_headers),
                "queryString": query,
                "headersSize": -1,
                "bodySize": e.post_data.as_ref().map_or(0, |p| p.len()),
            });
            if let Some(p) = &e.post_data {
                request["postData"] = json!({ "mimeType": req_mime, "text": p });
            }
            let mut content = json!({ "size": e.size as i64, "mimeType": e.mime });
            // Binaries are not saved (the body is "<binary, N bytes>").
            if let Some(b) = e
                .body
                .as_ref()
                .filter(|b| e.image.is_none() && !is_placeholder(b))
            {
                content["text"] = b.clone().into();
            }
            json!({
                "startedDateTime": started,
                "time": time,
                "request": request,
                "response": {
                    "status": e.status.unwrap_or(0),
                    "statusText": e.status_text,
                    "httpVersion": http,
                    "cookies": [],
                    "headers": headers(&e.response_headers),
                    "content": content,
                    "redirectURL": header(&e.response_headers, "location").unwrap_or_default(),
                    "headersSize": -1,
                    "bodySize": e.size as i64,
                    "_error": e.error,
                },
                "cache": {},
                "timings": { "send": 0, "wait": time, "receive": 0 },
                "serverIPAddress": e.remote_addr.clone().unwrap_or_default(),
            })
        })
        .collect();
    json!({
        "log": {
            "version": "1.2",
            "creator": { "name": "modev", "version": env!("CARGO_PKG_VERSION") },
            "pages": [],
            "entries": entries,
        }
    })
}

pub fn header(h: &[(String, String)], name: &str) -> Option<String> {
    h.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

/// Placeholder texts the engine uses when there is no real body (binary, CDP error).
fn is_placeholder(body: &str) -> bool {
    body.starts_with("<binary") || body.starts_with("<no body")
}

// ───────────────────────── Search ─────────────────────────

/// Case-insensitive matches (ASCII only: so the indices stay valid).
pub fn search_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return vec![];
    }
    let hay = text.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    hay.match_indices(&needle)
        .map(|(i, m)| i..i + m.len())
        .collect()
}

/// Overlays the match background on the syntax colors
/// (GPUI doesn't support overlapping highlights: they are split into runs).
pub fn merge_highlights(
    syntax: Vec<(Range<usize>, HighlightStyle)>,
    matches: &[Range<usize>],
    match_bg: u32,
) -> Vec<(Range<usize>, HighlightStyle)> {
    if matches.is_empty() {
        return syntax;
    }
    let mut cuts: Vec<usize> = syntax
        .iter()
        .flat_map(|(r, _)| [r.start, r.end])
        .chain(matches.iter().flat_map(|r| [r.start, r.end]))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let bg = Some(rgb(match_bg).into());
    let (mut si, mut mi) = (0, 0);
    let mut out = vec![];
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        while si < syntax.len() && syntax[si].0.end <= a {
            si += 1;
        }
        while mi < matches.len() && matches[mi].end <= a {
            mi += 1;
        }
        let color = syntax
            .get(si)
            .filter(|(r, _)| r.start <= a)
            .and_then(|(_, s)| s.color);
        let hit = matches.get(mi).is_some_and(|r| r.start <= a);
        if color.is_some() || hit {
            out.push((
                a..b,
                HighlightStyle {
                    color,
                    background_color: hit.then_some(bg).flatten(),
                    ..Default::default()
                },
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(v: Value) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string())
    }

    fn token(payload: Value) -> String {
        format!(
            "{}.{}.sig",
            b64(json!({"alg": "HS256", "typ": "JWT"})),
            b64(payload)
        )
    }

    #[test]
    fn finds_valid_jwt_in_authorization_header() {
        let exp = Utc::now().timestamp() + 3600;
        let t = token(json!({"sub": "42", "exp": exp}));
        let mut e = NetEntry::sample("GET", "https://api.dev/me");
        e.request_headers = vec![("Authorization".into(), format!("Bearer {t}"))];
        let jwts = find_jwts(&e);
        assert_eq!(jwts.len(), 1);
        let j = &jwts[0];
        assert_eq!(j.source, "Request · authorization");
        assert_eq!(j.token, t);
        assert!(!j.expired);
        assert!(j.payload.contains("\"sub\": \"42\""));
        assert!(j.header.contains("HS256"));
        assert_eq!(j.times.len(), 1);
        assert_eq!(j.times[0].0, "Expires (exp)");
        assert!(j.times[0].1.contains("in "));
    }

    #[test]
    fn expired_jwt_is_flagged() {
        let t = token(json!({"exp": 1_000_000_000, "iat": 999_999_000}));
        let mut e = NetEntry::sample("GET", "https://api.dev/me");
        e.body = Some(format!("{{\"access_token\":\"{t}\"}}"));
        let jwts = find_jwts(&e);
        assert_eq!(jwts.len(), 1);
        assert!(jwts[0].expired);
        assert_eq!(jwts[0].source, "Response · body");
        assert_eq!(jwts[0].times.len(), 2);
        assert!(jwts[0].times[0].1.ends_with("ago"));
    }

    #[test]
    fn same_jwt_in_several_places_is_listed_once() {
        let t = token(json!({"sub": "1"}));
        let mut e = NetEntry::sample("POST", "https://api.dev/refresh");
        e.request_headers = vec![("authorization".into(), format!("Bearer {t}"))];
        e.post_data = Some(format!("token={t}"));
        let jwts = find_jwts(&e);
        assert_eq!(jwts.len(), 1);
        assert_eq!(jwts[0].source, "Request · authorization");
    }

    #[test]
    fn non_jwt_strings_are_ignored() {
        let mut e = NetEntry::sample("GET", "https://api.dev/");
        e.request_headers = vec![
            ("x-a".into(), "eyJ.not.base64!".into()),
            ("x-b".into(), "eyJhbGciOiJIUzI1NiJ9".into()),
            // Valid base64 JSON header without "alg".
            (
                "x-c".into(),
                format!("{}.{}.s", b64(json!({"typ": "JWT"})), b64(json!({}))),
            ),
            ("x-d".into(), "plain text, a.b.c".into()),
        ];
        // Payload that is not a JSON object.
        e.body = Some(format!(
            "{}.{}.s",
            b64(json!({"alg": "none"})),
            b64(json!([1, 2]))
        ));
        assert!(find_jwts(&e).is_empty());
    }

    #[test]
    fn relative_time_units() {
        assert_eq!(relative(0), "in 0 s");
        assert_eq!(relative(59), "in 59 s");
        assert_eq!(relative(-120), "2 min ago");
        assert_eq!(relative(7200), "in 2 h");
        assert_eq!(relative(-3 * 86400), "3 d ago");
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let h = vec![
            ("Content-Type".to_string(), "text/plain".to_string()),
            ("X-Dup".to_string(), "first".to_string()),
            ("x-dup".to_string(), "second".to_string()),
        ];
        assert_eq!(header(&h, "content-type").as_deref(), Some("text/plain"));
        assert_eq!(header(&h, "X-DUP").as_deref(), Some("first"));
        assert_eq!(header(&h, "missing"), None);
    }

    #[test]
    fn har_structure() {
        let mut get = NetEntry::sample("GET", "https://api.dev/items?page=2&q=a%20b&flag#top");
        get.body = Some("[1,2]".into());
        get.protocol = Some("h2".into());
        get.remote_addr = Some("127.0.0.1:443".into());
        get.response_headers = vec![("Location".into(), "/next".into())];

        let mut post = NetEntry::sample("POST", "https://api.dev/upload");
        post.request_headers = vec![("Content-Type".into(), "application/json".into())];
        post.post_data = Some("{\"a\":1}".into());
        post.body = Some("<binary, 10 bytes>".into());
        post.status = None;
        post.end_ts = None;
        post.error = Some("net::ERR_FAILED".into());

        let h = har(&[get, post]);
        let log = &h["log"];
        assert_eq!(log["version"], "1.2");
        assert_eq!(log["creator"]["name"], "modev");
        let entries = log["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);

        let g = &entries[0];
        assert_eq!(g["request"]["method"], "GET");
        assert_eq!(g["request"]["httpVersion"], "h2");
        assert_eq!(
            g["request"]["queryString"],
            json!([
                {"name": "page", "value": "2"},
                {"name": "q", "value": "a%20b"},
                {"name": "flag", "value": ""},
            ])
        );
        assert!(g["request"].get("postData").is_none());
        assert_eq!(g["time"], 250.0);
        assert_eq!(g["startedDateTime"], "2023-11-14T22:13:20+00:00");
        assert_eq!(g["response"]["status"], 200);
        assert_eq!(g["response"]["content"]["text"], "[1,2]");
        assert_eq!(g["response"]["redirectURL"], "/next");
        assert_eq!(g["serverIPAddress"], "127.0.0.1:443");

        let p = &entries[1];
        assert_eq!(p["request"]["httpVersion"], "HTTP/1.1");
        assert_eq!(p["request"]["bodySize"], 7);
        assert_eq!(p["request"]["postData"]["mimeType"], "application/json");
        assert_eq!(p["request"]["postData"]["text"], "{\"a\":1}");
        assert_eq!(p["time"], 0.0);
        assert_eq!(p["response"]["status"], 0);
        assert_eq!(p["response"]["_error"], "net::ERR_FAILED");
        // Binary placeholders are not exported as content.
        assert!(p["response"]["content"].get("text").is_none());
    }

    #[test]
    fn search_is_ascii_case_insensitive() {
        assert_eq!(search_ranges("Foo foo FOO", "foo"), vec![0..3, 4..7, 8..11]);
        assert_eq!(search_ranges("abc", "B"), vec![1..2]);
        assert!(search_ranges("abc", "").is_empty());
        assert!(search_ranges("abc", "x").is_empty());
        // Non-overlapping matches.
        assert_eq!(search_ranges("aaaa", "aa"), vec![0..2, 2..4]);
        // Byte indices stay valid with non-ASCII text.
        let text = "ñandú Ñ";
        let r = search_ranges(text, "dú");
        assert_eq!(&text[r[0].clone()], "dú");
    }

    fn colored(r: Range<usize>, c: u32) -> (Range<usize>, HighlightStyle) {
        (
            r,
            HighlightStyle {
                color: Some(rgb(c).into()),
                ..Default::default()
            },
        )
    }

    #[test]
    fn merge_highlights_without_matches_keeps_syntax() {
        let syntax = vec![colored(0..3, 0x111111)];
        assert_eq!(merge_highlights(syntax.clone(), &[], 0xffff00), syntax);
    }

    #[test]
    fn merge_highlights_splits_overlapping_runs() {
        let (red, blue, bg) = (0xff0000, 0x0000ff, 0xffff00);
        let syntax = vec![colored(0..4, red), colored(6..10, blue)];
        let out = merge_highlights(syntax, std::slice::from_ref(&(2..8)), bg);
        let bg = Some(rgb(bg).into());
        let expect = |r: Range<usize>, c: Option<u32>, hit: bool| {
            (
                r,
                HighlightStyle {
                    color: c.map(|c| rgb(c).into()),
                    background_color: if hit { bg } else { None },
                    ..Default::default()
                },
            )
        };
        assert_eq!(
            out,
            vec![
                expect(0..2, Some(red), false),
                expect(2..4, Some(red), true),
                expect(4..6, None, true),
                expect(6..8, Some(blue), true),
                expect(8..10, Some(blue), false),
            ]
        );
    }
}
