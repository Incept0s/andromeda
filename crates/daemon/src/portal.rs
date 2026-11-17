// SPDX-License-Identifier: GPL-3.0-or-later
//! XDG desktop portal requests: interactive screenshot (for OCR) and the
//! color picker. GNOME shows its own UI for both; the result comes back
//! asynchronously as a `Response` signal on a per-request object, so we
//! subscribe to that signal before making the call.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use futures_util::StreamExt;
use zbus::message::Type;
use zbus::{MatchRule, MessageStream, OwnedVariant};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Calls `interface.method("", options)` and waits for the user to finish.
/// Returns the result dictionary, or an error if cancelled or failed.
async fn request(
    conn: &zbus::Connection,
    interface: &str,
    method: &str,
    mut options: HashMap<&str, zbus::Variant<'_>>,
) -> Result<HashMap<String, OwnedVariant>, String> {
    let token = format!("andromeda{}", COUNTER.fetch_add(1, Ordering::Relaxed));
    // The request path is predictable: unique name without ':' and with
    // '.' replaced by '_', then the token.
    let sender = conn.unique_name().ok_or("no bus name")?.trim_start_matches(':').replace('.', "_");
    let req_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.portal.Request")
        .member("Response")
        .path(req_path.as_str())
        .build()
        .map_err(|e| e.to_string())?;
    let mut stream = MessageStream::for_match_rule(rule, conn, std::num::NonZeroUsize::new(1)).await.map_err(|e| e.to_string())?;

    options.insert("handle_token", zbus::Variant::from(token.as_str()));
    conn.call_method(Some(DEST), PATH, Some(interface), method, &("", options))
        .await
        .map_err(|e| e.to_string())?;

    let msg = stream.next().await.ok_or("no response")?.map_err(|e| e.to_string())?;
    let (code, results): (u32, HashMap<String, OwnedVariant>) = msg.body().deserialize().map_err(|e| e.to_string())?;
    match code {
        0 => Ok(results),
        1 => Err("Cancelled".into()),
        _ => Err("The screenshot could not be taken".into()),
    }
}

/// Lets the user pick an area (GNOME's screenshot UI) and returns the file.
pub async fn screenshot(conn: &zbus::Connection) -> Result<std::path::PathBuf, String> {
    let opts = HashMap::from([("interactive", zbus::Variant::from(true))]);
    let r = request(conn, "org.freedesktop.portal.Screenshot", "Screenshot", opts).await?;
    let uri: String = r.get("uri").and_then(|v| v.try_clone().ok()?.try_into().ok()).ok_or("no image returned")?;
    let path = uri.strip_prefix("file://").ok_or("not a local file")?;
    Ok(percent_decode(path).into())
}

/// Lets the user click any pixel; returns "#rrggbb".
pub async fn pick_color(conn: &zbus::Connection) -> Result<String, String> {
    let r = request(conn, "org.freedesktop.portal.Screenshot", "PickColor", HashMap::new()).await?;
    let (red, green, blue): (f64, f64, f64) = r
        .get("color")
        .and_then(|v| v.try_clone().ok()?.try_into().ok())
        .ok_or("no color returned")?;
    let c = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Ok(format!("#{:02x}{:02x}{:02x}", c(red), c(green), c(blue)))
}

/// File URIs escape spaces and other bytes as %XX.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_file_uris() {
        assert_eq!(super::percent_decode("/home/me/Screenshot%20from%202026.png"), "/home/me/Screenshot from 2026.png");
        assert_eq!(super::percent_decode("/a%zz"), "/a%zz");
    }
}
