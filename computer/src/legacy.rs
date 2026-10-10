//! Deprecated structured JSON forms (`au act JSON`, JSONL envelopes, and MCP
//! calls without a `command` string). New agent calls use the plain command
//! grammar in `command.rs`; this module only keeps the old wire contract
//! parseable and bounded.
use crate::api::{
    safe_setting, safe_url, BrowserOp, BrowserPlan, BrowserPredicate, BrowserRead, Capture, Code, Direction, Error, Key, Match, Op, Plan, Predicate, Range, Read, Result, VisualOp,
    VisualPlan, VisualRead, MAX_MUTATIONS, MAX_OPS, MAX_PREDICATE, MAX_TEXT,
};
use serde_json::Value;

impl Plan {
    pub fn parse(v: Value) -> Result<Self> {
        let o = v.as_object().ok_or_else(|| Error::new(Code::Args, "act arguments must be an object"))?;
        let id = string(o.get("id"), "id", 64)?;
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(Error::new(Code::Args, "id may contain only letters, digits, dash, and underscore"));
        }
        let id = id.into_boxed_str();
        let generation = uint(o.get("g"), "g")?;
        let deadline_ms = o.get("deadline_ms").map(|v| uint(Some(v), "deadline_ms")).transpose()?.unwrap_or(8000);
        if !(1..=30_000).contains(&deadline_ms) {
            return Err(Error::new(Code::Bounds, "deadline_ms must be 1..30000"));
        }
        let max_mutations = o.get("max_mutations").map(|v| uint(Some(v), "max_mutations")).transpose()?.unwrap_or(MAX_MUTATIONS as u64);
        if max_mutations > MAX_MUTATIONS as u64 {
            return Err(Error::new(Code::Bounds, "max_mutations exceeds 16"));
        }
        let rows = o.get("p").and_then(Value::as_array).ok_or_else(|| Error::new(Code::Args, "p must be an operation array"))?;
        if rows.is_empty() || rows.len() > MAX_OPS {
            return Err(Error::new(Code::Bounds, "plan must contain 1..32 operations"));
        }
        let ops: Vec<Op> = rows.iter().map(parse_op).collect::<Result<_>>()?;
        if ops.iter().filter(|o| o.mutates()).count() > max_mutations as usize {
            return Err(Error::new(Code::Bounds, "plan exceeds its mutation budget"));
        }
        Ok(Self { id, generation, deadline_ms: deadline_ms as u32, max_mutations: max_mutations as u8, ops: ops.into_boxed_slice() })
    }
}

impl BrowserPlan {
    pub fn parse(v: Value) -> Result<Self> {
        let o = v.as_object().ok_or_else(|| Error::new(Code::Args, "browser act arguments must be an object"))?;
        if o.get("target").and_then(Value::as_str) != Some("browser") {
            return Err(Error::new(Code::Args, "browser plan target must be browser"));
        }
        let id = string(o.get("id"), "id", 64)?;
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(Error::new(Code::Args, "id may contain only letters, digits, dash, and underscore"));
        }
        let generation = uint(o.get("g"), "g")?;
        let deadline_ms = o.get("deadline_ms").map(|v| uint(Some(v), "deadline_ms")).transpose()?.unwrap_or(8000);
        if !(1..=30_000).contains(&deadline_ms) {
            return Err(Error::new(Code::Bounds, "deadline_ms must be 1..30000"));
        }
        let max_mutations = o.get("max_mutations").map(|v| uint(Some(v), "max_mutations")).transpose()?.unwrap_or(MAX_MUTATIONS as u64);
        if max_mutations > MAX_MUTATIONS as u64 {
            return Err(Error::new(Code::Bounds, "max_mutations exceeds 16"));
        }
        let rows = o.get("p").and_then(Value::as_array).ok_or_else(|| Error::new(Code::Args, "p must be an operation array"))?;
        if rows.is_empty() || rows.len() > MAX_OPS {
            return Err(Error::new(Code::Bounds, "plan must contain 1..32 operations"));
        }
        let ops: Vec<BrowserOp> = rows.iter().map(parse_browser_op).collect::<Result<_>>()?;
        if ops.iter().filter(|op| op.mutates()).count() > max_mutations as usize {
            return Err(Error::new(Code::Bounds, "plan exceeds its mutation budget"));
        }
        Ok(Self { id: id.into_boxed_str(), generation, deadline_ms: deadline_ms as u32, max_mutations: max_mutations as u8, ops: ops.into_boxed_slice() })
    }
}

impl VisualPlan {
    pub fn parse(v: Value) -> Result<Self> {
        let o = v.as_object().ok_or_else(|| Error::new(Code::Args, "visual act arguments must be an object"))?;
        if o.get("target").and_then(Value::as_str) != Some("visual") {
            return Err(Error::new(Code::Args, "visual plan target must be visual"));
        }
        let id = string(o.get("id"), "id", 64)?;
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(Error::new(Code::Args, "id may contain only letters, digits, dash, and underscore"));
        }
        let generation = uint(o.get("g"), "g")?;
        let rows = o.get("p").and_then(Value::as_array).ok_or_else(|| Error::new(Code::Args, "p must be an operation array"))?;
        if rows.len() != 1 {
            return Err(Error::new(Code::Bounds, "visual plan must contain one operation"));
        }
        Ok(Self { id: id.into_boxed_str(), generation, op: parse_visual_op(&rows[0])? })
    }
}

fn parse_visual_op(v: &Value) -> Result<VisualOp> {
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "visual operation must be an array"))?;
    if a.len() != 6 || a.first().and_then(Value::as_str) != Some("crop") {
        return Err(Error::new(Code::Unsupported, "visual operation must be crop"));
    }
    let artifact = string(Some(&a[1]), "artifact", 64)?;
    let n = |v: &Value, name: &str| -> Result<u32> { u32::try_from(uint(Some(v), name)?).map_err(|_| Error::new(Code::Bounds, format!("{name} exceeds u32"))) };
    Ok(VisualOp::Crop { artifact: artifact.into_boxed_str(), x: n(&a[2], "x")?, y: n(&a[3], "y")?, w: n(&a[4], "w")?, h: n(&a[5], "h")? })
}

fn parse_browser_op(v: &Value) -> Result<BrowserOp> {
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "browser operation must be an array"))?;
    let name = a.first().and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, "browser operation name must be a string"))?;
    let exact = |n| if a.len() == n { Ok(()) } else { Err(Error::new(Code::Args, format!("{name} expects {} values", n - 1))) };
    match name {
        "navigate" | "new" => {
            exact(2)?;
            let url = string(Some(&a[1]), "url", 2048)?;
            if !(url.starts_with("https://") || url.starts_with("http://")) || url.bytes().any(|b| b.is_ascii_control() || b == b'"') {
                return Err(Error::new(Code::Args, "url must be an http(s) URL without control characters"));
            }
            Ok(if name == "new" { BrowserOp::New(url.into_boxed_str()) } else { BrowserOp::Navigate(url.into_boxed_str()) })
        }
        "back" => exact(1).map(|_| BrowserOp::Back),
        "forward" => exact(1).map(|_| BrowserOp::Forward),
        "reload" => exact(1).map(|_| BrowserOp::Reload),
        "screenshot" => exact(1).map(|_| BrowserOp::Screenshot),
        "click" => {
            exact(2)?;
            Ok(BrowserOp::Click(ref_id(&a[1])?))
        }
        "focus" => {
            exact(2)?;
            Ok(BrowserOp::Focus(ref_id(&a[1])?))
        }
        "text" => {
            exact(3)?;
            Ok(BrowserOp::Text(ref_id(&a[1])?, bounded_text(&a[2])?))
        }
        "key" => {
            exact(2)?;
            let key = string(Some(&a[1]), "key", 32)?;
            if !matches!(key.as_str(), "Enter" | "Tab" | "Escape" | "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Backspace") {
                return Err(Error::new(Code::Args, "unsupported browser key"));
            }
            Ok(BrowserOp::Key(key.into_boxed_str()))
        }
        "scroll" => {
            exact(2)?;
            let px = a[1].as_i64().ok_or_else(|| Error::new(Code::Args, "scroll must be a signed integer"))?;
            let px = i32::try_from(px).map_err(|_| Error::new(Code::Bounds, "scroll exceeds i32"))?;
            if !(-20_000..=20_000).contains(&px) {
                return Err(Error::new(Code::Bounds, "scroll must be within +/-20000 pixels"));
            }
            Ok(BrowserOp::Scroll(px))
        }
        "wait" => {
            exact(3)?;
            let p = a[1].as_array().ok_or_else(|| Error::new(Code::Args, "browser wait predicate must be an array"))?;
            if p.len() != 2 {
                return Err(Error::new(Code::Args, "browser wait predicate expects one value"));
            }
            let value = predicate_text(&p[1])?;
            let predicate = match p[0].as_str() {
                Some("css") => BrowserPredicate::Css(value),
                Some("text") => BrowserPredicate::Text(value),
                _ => return Err(Error::new(Code::Unsupported, "browser wait predicate must be css or text")),
            };
            let ms = uint(Some(&a[2]), "timeout")?;
            if ms > 30_000 {
                return Err(Error::new(Code::Bounds, "wait timeout exceeds 30000"));
            }
            Ok(BrowserOp::Wait(predicate, ms as u16))
        }
        "select" | "close" => {
            exact(2)?;
            let id = string(Some(&a[1]), "tab id", 128)?;
            if !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')) {
                return Err(Error::new(Code::Args, "tab id contains unsupported characters"));
            }
            Ok(if name == "select" { BrowserOp::Select(id.into_boxed_str()) } else { BrowserOp::Close(id.into_boxed_str()) })
        }
        _ => Err(Error::new(Code::Unsupported, format!("unknown browser operation {name}"))),
    }
}

fn parse_op(v: &Value) -> Result<Op> {
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "operation must be an array"))?;
    let name = a.first().and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, "operation name must be a string"))?;
    let exact = |n| if a.len() == n { Ok(()) } else { Err(Error::new(Code::Args, format!("{name} expects {} values", n - 1))) };
    match name {
        "tap" => {
            exact(2)?;
            Ok(Op::Tap(ref_id(&a[1])?))
        }
        "long" => {
            exact(2)?;
            Ok(Op::Long(ref_id(&a[1])?))
        }
        "tap_point" => {
            exact(3)?;
            Ok(Op::PointTap { x: u16v(&a[1], "x")?, y: u16v(&a[2], "y")? })
        }
        "swipe" => {
            exact(6)?;
            let duration_ms = u16v(&a[5], "duration")?;
            if duration_ms > 30_000 {
                return Err(Error::new(Code::Bounds, "swipe duration exceeds 30000"));
            }
            Ok(Op::Swipe { x1: u16v(&a[1], "x1")?, y1: u16v(&a[2], "y1")?, x2: u16v(&a[3], "x2")?, y2: u16v(&a[4], "y2")?, duration_ms })
        }
        "text" => {
            exact(3)?;
            Ok(Op::Text(ref_id(&a[1])?, bounded_text(&a[2])?))
        }
        "scroll" => {
            exact(3)?;
            Ok(Op::Scroll(ref_id(&a[1])?, direction(&a[2])?))
        }
        "key" => {
            exact(2)?;
            Ok(Op::Key(key(&a[1])?))
        }
        "gesture" => {
            exact(2)?;
            Ok(Op::Gesture(points(&a[1])?))
        }
        "wait" => {
            exact(3)?;
            let t = uint(Some(&a[2]), "timeout")?;
            if t > 30_000 {
                return Err(Error::new(Code::Bounds, "wait timeout exceeds 30000"));
            }
            Ok(Op::Wait(predicate(&a[1])?, t as u16))
        }
        "assert" => {
            exact(2)?;
            Ok(Op::Assert(predicate(&a[1])?))
        }
        "launch" => {
            exact(2)?;
            Ok(Op::Launch(string(Some(&a[1]), "package", 255)?.into_boxed_str()))
        }
        "setting" => {
            exact(2)?;
            let name = string(Some(&a[1]), "setting", 128)?;
            if !safe_setting(&name) {
                return Err(Error::new(Code::Unsupported, "setting is not allowlisted"));
            }
            Ok(Op::Setting(name.into_boxed_str()))
        }
        "link" => {
            exact(2)?;
            let url = string(Some(&a[1]), "url", 2048)?;
            if !safe_url(&url) {
                return Err(Error::new(Code::Args, "url must be an allowlisted http(s) link"));
            }
            Ok(Op::Link(url.into_boxed_str()))
        }
        "capture" => {
            exact(2)?;
            if a[1].as_str() != Some("screen") {
                return Err(Error::new(Code::Unsupported, "only screen capture is available in core"));
            }
            Ok(Op::Capture(Capture::Screen))
        }
        "camera" => {
            if a.len() != 2 && a.len() != 4 {
                return Err(Error::new(Code::Args, "camera expects facing or facing,width,height"));
            }
            let facing = string(Some(&a[1]), "camera", 8)?;
            if !matches!(facing.as_str(), "rear" | "front" | "") {
                return Err(Error::new(Code::Args, "camera must be rear or front"));
            }
            let (width, height) = if a.len() == 4 {
                let width = uint(Some(&a[2]), "camera width")?;
                let height = uint(Some(&a[3]), "camera height")?;
                if !(160..=4096).contains(&width) || !(160..=4096).contains(&height) {
                    return Err(Error::new(Code::Bounds, "camera dimensions must be 160..4096"));
                }
                (Some(width as u16), Some(height as u16))
            } else {
                (None, None)
            };
            Ok(Op::Capture(Capture::Camera { facing: facing.into_boxed_str(), width, height }))
        }
        "microphone" => {
            exact(2)?;
            let seconds = uint(Some(&a[1]), "seconds")?;
            if !(1..=30).contains(&seconds) {
                return Err(Error::new(Code::Bounds, "microphone duration must be 1..30 seconds"));
            }
            Ok(Op::Capture(Capture::Microphone(seconds as u16)))
        }
        "screen_record" => {
            exact(2)?;
            let seconds = uint(Some(&a[1]), "seconds")?;
            if !(1..=30).contains(&seconds) {
                return Err(Error::new(Code::Bounds, "screen recording duration must be 1..30 seconds"));
            }
            Ok(Op::Capture(Capture::ScreenRecord(seconds as u16)))
        }
        "notification_open" | "notification_dismiss" | "notification_action" => {
            exact(2)?;
            let id = string(Some(&a[1]), "notification id", 256)?;
            Ok(match name {
                "notification_open" => Op::NotificationOpen(id.into_boxed_str()),
                "notification_dismiss" => Op::NotificationDismiss(id.into_boxed_str()),
                _ => Op::NotificationAction(id.into_boxed_str()),
            })
        }
        _ => Err(Error::new(Code::Unsupported, format!("unknown operation {name}"))),
    }
}

fn predicate(v: &Value) -> Result<Predicate> {
    if v.to_string().len() > MAX_PREDICATE {
        return Err(Error::new(Code::Bounds, "predicate exceeds 1024 bytes"));
    }
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "predicate must be an array"))?;
    let n = a.first().and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, "predicate name must be a string"))?;
    if a.len() != 2 {
        return Err(Error::new(Code::Args, "predicate expects one operand"));
    }
    match n {
        "exists" => Ok(Predicate::Exists(match_arg(&a[1])?)),
        "missing" => Ok(Predicate::Missing(match_arg(&a[1])?)),
        "text" => Ok(Predicate::Text(predicate_text(&a[1])?)),
        "generation_after" => Ok(Predicate::GenerationAfter(uint(Some(&a[1]), "generation")?)),
        _ => Err(Error::new(Code::Unsupported, format!("unknown predicate {n}"))),
    }
}

fn match_arg(v: &Value) -> Result<Match> {
    if let Some(r) = v.as_u64() {
        return u16::try_from(r).map(Match::Ref).map_err(|_| Error::new(Code::Bounds, "ref exceeds u16"));
    }
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "match must be a ref or [label,text]"))?;
    if a.len() != 2 || a[0].as_str() != Some("label") {
        return Err(Error::new(Code::Args, "match must be a ref or [label,text]"));
    }
    Ok(Match::Label(predicate_text(&a[1])?))
}

fn ref_id(v: &Value) -> Result<u16> {
    u16::try_from(v.as_u64().ok_or_else(|| Error::new(Code::Args, "ref must be an integer"))?).map_err(|_| Error::new(Code::Bounds, "ref exceeds u16"))
}
fn bounded_text(v: &Value) -> Result<Box<str>> {
    let s = v.as_str().ok_or_else(|| Error::new(Code::Args, "text must be a string"))?;
    if s.len() > MAX_TEXT {
        return Err(Error::new(Code::Bounds, "text exceeds 8192 bytes"));
    }
    Ok(s.into())
}
fn predicate_text(v: &Value) -> Result<Box<str>> {
    let s = v.as_str().ok_or_else(|| Error::new(Code::Args, "predicate text must be a string"))?;
    if s.len() > MAX_PREDICATE {
        return Err(Error::new(Code::Bounds, "predicate text exceeds 1024 bytes"));
    }
    Ok(s.into())
}
fn string(v: Option<&Value>, name: &str, max: usize) -> Result<String> {
    let s = v.and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, format!("{name} must be a string")))?;
    if s.is_empty() || s.len() > max {
        return Err(Error::new(Code::Bounds, format!("{name} length is invalid")));
    }
    Ok(s.into())
}
fn uint(v: Option<&Value>, name: &str) -> Result<u64> {
    v.and_then(Value::as_u64).ok_or_else(|| Error::new(Code::Args, format!("{name} must be an unsigned integer")))
}
fn u16v(v: &Value, name: &str) -> Result<u16> {
    u16::try_from(uint(Some(v), name)?).map_err(|_| Error::new(Code::Bounds, format!("{name} exceeds 65535")))
}

fn direction(v: &Value) -> Result<Direction> {
    match v.as_str() {
        Some("up") => Ok(Direction::Up),
        Some("down") => Ok(Direction::Down),
        Some("left") => Ok(Direction::Left),
        Some("right") => Ok(Direction::Right),
        _ => Err(Error::new(Code::Args, "direction must be up, down, left, or right")),
    }
}

fn key(v: &Value) -> Result<Key> {
    match v.as_str() {
        Some("back") => Ok(Key::Back),
        Some("home") => Ok(Key::Home),
        Some("recents") => Ok(Key::Recents),
        Some("notifications") => Ok(Key::Notifications),
        Some("enter") => Ok(Key::Enter),
        _ => Err(Error::new(Code::Args, "unsupported key")),
    }
}
fn points(v: &Value) -> Result<Box<[[u16; 3]]>> {
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "gesture points must be an array"))?;
    if a.len() < 2 || a.len() > 16 {
        return Err(Error::new(Code::Bounds, "gesture needs 2..16 points"));
    }
    let mut out = Vec::with_capacity(a.len());
    for p in a {
        let p = p.as_array().ok_or_else(|| Error::new(Code::Args, "gesture point must be [x,y,ms]"))?;
        if p.len() != 3 {
            return Err(Error::new(Code::Args, "gesture point must be [x,y,ms]"));
        }
        out.push([
            u16::try_from(uint(Some(&p[0]), "x")?).map_err(|_| Error::new(Code::Bounds, "x exceeds u16"))?,
            u16::try_from(uint(Some(&p[1]), "y")?).map_err(|_| Error::new(Code::Bounds, "y exceeds u16"))?,
            u16::try_from({
                let ms = uint(Some(&p[2]), "ms")?;
                if ms > 30_000 {
                    return Err(Error::new(Code::Bounds, "gesture duration exceeds 30000"));
                }
                ms
            })
            .map_err(|_| Error::new(Code::Bounds, "ms exceeds u16"))?,
        ]);
    }
    Ok(out.into_boxed_slice())
}

pub fn parse_read(v: Value) -> Result<Read> {
    let o = v.as_object().ok_or_else(|| Error::new(Code::Args, "read arguments must be an object"))?;
    match o
        .get("q")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new(Code::Args, "q must be status, observe, artifact, browser, capabilities, location, notifications, or visual"))?
    {
        "status" => Ok(Read::Status),
        "observe" => {
            let base = o.get("base").map(|v| string(Some(v), "base", 64).map(String::into_boxed_str)).transpose()?;
            let detail = o.get("detail").map(|v| uint(Some(v), "detail")).transpose()?.unwrap_or(0);
            if detail > 1 {
                return Err(Error::new(Code::Bounds, "detail must be 0 or 1"));
            }
            Ok(Read::Observe { base, detail: detail as u8 })
        }
        "artifact" => {
            let id = string(o.get("id"), "id", 64)?.into_boxed_str();
            let range = o.get("range").map(|v| serde_json::from_value::<Range>(v.clone()).map_err(Error::from)).transpose()?;
            Ok(Read::Artifact { id, range })
        }
        "browser" => {
            let op = match o.get("op").and_then(Value::as_str).unwrap_or("observe") {
                "tabs" => BrowserRead::Tabs,
                "observe" => BrowserRead::Observe,
                "text" => BrowserRead::Text,
                _ => return Err(Error::new(Code::Args, "browser op must be tabs, observe, or text")),
            };
            Ok(Read::Browser { op })
        }
        "capabilities" => Ok(Read::Capabilities),
        "location" => Ok(Read::Location),
        "notifications" => Ok(Read::Notifications),
        "visual" => {
            let op = o.get("op").and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, "visual read requires op"))?;
            match op {
                "hash" => Ok(Read::Visual(VisualRead::Hash(string(o.get("id"), "id", 64)?.into_boxed_str()))),
                "diff" => Ok(Read::Visual(VisualRead::Diff(string(o.get("a"), "a", 64)?.into_boxed_str(), string(o.get("b"), "b", 64)?.into_boxed_str()))),
                _ => Err(Error::new(Code::Unsupported, "visual read must be hash or diff")),
            }
        }
        _ => Err(Error::new(Code::Args, "q must be status, observe, artifact, browser, capabilities, location, notifications, or visual")),
    }
}
