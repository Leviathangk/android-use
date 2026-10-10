//! Plain-language rendering for agents. Everything here is a pure function
//! from device data to short, bounded text with no wire syntax in it.
use crate::api::{normalized, Action, AndroidAction, BrowserAction, Code, Error, Node, Receipt, Result, Scene};
use serde_json::Value;
use std::collections::HashMap;

/// Byte budget for a normal `screen` read and for the screen summary that
/// follows an action. Roughly 300 tokens: enough for a typical app screen
/// so agents rarely need a second `screen full` round trip.
pub(crate) const SCREEN_BYTES: usize = 1200;
pub(crate) const FULL_SCREEN_BYTES: usize = 4000;
pub(crate) const NOTIFICATION_ACCESS_OFF: &str =
    "Notification access is off for Android Use. Ask the user to allow it on the device (Settings → Notifications → Device & app notifications → Android Use), then retry.";

/// One conceptual row compiled by the helper's semantic compiler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticRow {
    pub label: String,
    pub value: String,
    pub kind: String,
    pub state: String,
    pub enabled: bool,
    pub selected: bool,
}

pub(crate) fn parse_semantic_rows(value: &Value) -> Result<Vec<SemanticRow>> {
    let rows = value.as_array().ok_or_else(|| Error::new(Code::Protocol, "semantic screen response must be an array"))?;
    if rows.len() > 256 {
        return Err(Error::new(Code::Bounds, "semantic screen response exceeds 256 rows"));
    }
    rows.iter()
        .map(|row| {
            let row = row.as_array().filter(|row| row.len() == 6).ok_or_else(|| Error::new(Code::Protocol, "semantic screen row must contain six values"))?;
            let text = |index: usize| match row[index].as_str() {
                Some(value) if value.len() <= 512 => Ok(value.to_owned()),
                Some(_) => Err(Error::new(Code::Bounds, "semantic screen text is too long")),
                None => Err(Error::new(Code::Protocol, "semantic screen text must be a string")),
            };
            let flag = |index: usize| row[index].as_bool().ok_or_else(|| Error::new(Code::Protocol, "semantic screen flag must be boolean"));
            Ok(SemanticRow { label: text(0)?, value: text(1)?, kind: text(2)?, state: text(3)?, enabled: flag(4)?, selected: flag(5)? })
        })
        .collect()
}

pub(crate) fn display(value: &str) -> String {
    let out: String = value.chars().filter(|&c| !matches!(c, '{' | '}' | '[' | ']' | '\0') && (!c.is_control() || matches!(c, '\n' | '\r' | '\t'))).collect();
    out.trim().to_owned()
}

pub(crate) fn display_n(value: &str, max_chars: usize) -> String {
    let mut out = display(value);
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
    }
    out
}

pub(crate) fn bounded_output(value: String, max_bytes: usize) -> String {
    bounded_output_with_tail(value, max_bytes, " More text was omitted.")
}

pub(crate) fn bounded_output_with_tail(value: String, max_bytes: usize, tail: &str) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut out = String::new();
    for ch in value.chars() {
        if out.len() + ch.len_utf8() + tail.len() > max_bytes {
            break;
        }
        out.push(ch);
    }
    out.push_str(tail);
    out
}

fn capitalize(value: String) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else { return value };
    first.to_uppercase().collect::<String>() + chars.as_str()
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", display_n(value, 64))
}

pub(crate) fn format_status(value: &Value) -> String {
    let connected = value.get("ok").and_then(Value::as_u64) == Some(1);
    let control = value.get("cap").and_then(Value::as_u64).unwrap_or(0) != 0;
    match (connected, control) {
        (true, true) => "Ready. Android Use is connected and can read and control the screen.".into(),
        (true, false) => "Not ready: the Android Use helper is running, but its Accessibility service is off. Ask the user to turn on Android Use under Settings → Accessibility on the device, then retry.".into(),
        _ => "Android Use is not ready. Ask the user to run au doctor on the computer, then retry.".into(),
    }
}

// ---- Android screen ------------------------------------------------------

/// Renders compiled rows. Duplicate labels are annotated with the
/// `number N` an agent must add to target them, in the same reading order
/// the resolver uses.
pub(crate) fn format_semantic_rows(rows: &[SemanticRow], full: bool, focus: &str) -> String {
    if rows.is_empty() {
        return if focus.is_empty() {
            "The screen has no readable controls. Read screen again after the view changes, or capture screen to see it.".into()
        } else {
            format!("Nothing on the screen matches {}. Read screen for the current view.", quoted(focus))
        };
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in rows.iter().filter(|row| row.kind != "heading") {
        *counts.entry(normalized(&row.label)).or_default() += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut lines = Vec::new();
    for row in rows.iter().take(if full { 256 } else { 64 }) {
        let line = semantic_line(row);
        if line.is_empty() {
            continue;
        }
        let key = normalized(&row.label);
        if row.kind != "heading" && counts.get(&key).copied().unwrap_or(0) > 1 {
            let number = seen.entry(key).or_default();
            *number += 1;
            let tag = format!(" (number {number})");
            lines.push(if line.contains(" — ") { line.replacen(" — ", &format!("{tag} — "), 1) } else { line + &tag });
        } else {
            lines.push(line);
        }
    }
    let omitted = rows.len().saturating_sub(lines.len());
    let mut text = lines.join("\n");
    if omitted > 0 {
        text.push_str(&format!("\n{omitted} more screen items were omitted."));
    }
    bounded_output_with_tail(
        text,
        if full { FULL_SCREEN_BYTES } else { SCREEN_BYTES },
        if focus.is_empty() && full {
            "\nSome controls were cut to fit; use find \"TEXT\" to locate one."
        } else if focus.is_empty() {
            "\nRead screen full for the complete control list."
        } else {
            "\nRead screen for a broader match."
        },
    )
}

pub(crate) fn semantic_delta(previous: &[SemanticRow], current: &[SemanticRow]) -> Vec<SemanticRow> {
    current.iter().filter(|row| !previous.contains(row)).cloned().collect()
}

pub(crate) fn format_semantic_delta(rows: &[SemanticRow]) -> String {
    if rows.is_empty() {
        return "No semantic screen changes.".into();
    }
    let lines = rows.iter().map(semantic_line).filter(|line| !line.is_empty()).collect::<Vec<_>>();
    bounded_output_with_tail(lines.join("\n"), SCREEN_BYTES, "\nRead screen for the complete current state.")
}

/// `Label — value state kind`, e.g. `Wi-Fi — on switch` or `Name — empty text field`.
pub(crate) fn semantic_line(row: &SemanticRow) -> String {
    let label = display_n(&row.label, 96);
    if label.is_empty() || row.kind == "heading" {
        return label;
    }
    let value = display_n(&row.value, 128);
    let state = display_n(&row.state, 32);
    let mut parts: Vec<&str> = Vec::new();
    match row.kind.as_str() {
        "switch" => parts.push(if matches!(state.as_str(), "checked" | "on") { "on" } else { "off" }),
        "checkbox" => parts.push(if state == "checked" { "checked" } else { "unchecked" }),
        "radio" if row.selected || state == "checked" => parts.push("selected"),
        "text field" => parts.push(if value.is_empty() { "empty" } else { value.as_str() }),
        "tab" if row.selected => parts.push("selected"),
        "radio" | "tab" => {}
        _ if !value.is_empty() => parts.push(value.as_str()),
        "slider" | "scroll area" | "button" | "link" => {}
        _ if !state.is_empty() => parts.push(state.as_str()),
        _ => {}
    }
    if !row.enabled {
        parts.push("disabled");
    }
    let generic = !matches!(row.kind.as_str(), "switch" | "checkbox" | "radio" | "text field" | "slider" | "scroll area" | "tab" | "button" | "link");
    if generic && row.selected {
        parts.push("selected");
    }
    match row.kind.as_str() {
        "radio" => parts.push("option"),
        kind if !generic => parts.push(kind),
        _ => {}
    }
    if parts.is_empty() {
        label
    } else {
        format!("{label} — {}", parts.join(" "))
    }
}

fn role_name(role: u8) -> &'static str {
    match role as char {
        'b' => "button",
        'i' => "text field",
        'c' => "checkbox",
        's' => "scroll area",
        'm' => "control",
        't' => "text",
        'a' => "link",
        _ => "item",
    }
}

fn display_node_label(value: &str) -> String {
    match value.split_once(":id/") {
        Some((_, tail)) => display_n(&tail.replace(['_', '-'], " "), 96),
        None => display_n(value, 96),
    }
}

fn actionable_node(node: &Node) -> bool {
    !node.label.contains(":id/") || node.role as char != 't' || node.flags & 1 != 0
}

/// Raw-scene rendering for helpers without the semantic compiler.
pub(crate) fn format_scene(scene: &Scene, full: bool) -> String {
    let useful: Vec<_> = scene.nodes.iter().filter(|node| !node.label.is_empty() && actionable_node(node)).collect();
    if useful.is_empty() {
        return "The screen has no readable controls. Read screen again after the view changes, or capture screen to see it.".into();
    }
    let limit = if full { 256 } else { 40 };
    let mut lines = Vec::new();
    for node in useful.iter().take(limit) {
        let state = match node.role as char {
            'c' if node.flags & 4 != 0 => "checked",
            'c' => "unchecked",
            _ if node.flags & 2 == 0 => "disabled",
            _ => "",
        };
        let detail = [state, role_name(node.role)].iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join(" ");
        lines.push(format!("{} — {detail}", display_node_label(&node.label)));
    }
    if useful.len() > limit {
        lines.push(format!("{} more controls were omitted.", useful.len() - limit));
    }
    bounded_output_with_tail(lines.join("\n"), if full { FULL_SCREEN_BYTES } else { SCREEN_BYTES }, "\nRead screen full for the complete control list.")
}

pub(crate) fn format_scene_focus(scene: &Scene, full: bool, matching: Option<&str>) -> String {
    let Some(matching) = matching.filter(|value| !value.trim().is_empty()) else { return format_scene(scene, full) };
    let needle = normalized(matching);
    let nodes: Vec<_> = scene.nodes.iter().filter(|node| crate::resolve::semantic_labels(&node.label).iter().any(|label| label.contains(&needle))).cloned().collect();
    if nodes.is_empty() {
        return format!("Nothing on the screen matches {}. Read screen for the current view.", quoted(matching));
    }
    format_scene(&Scene { nodes: nodes.into_boxed_slice(), ..scene.clone() }, full)
}

// ---- Chrome ---------------------------------------------------------------

pub(crate) fn format_browser_tabs(value: &Value) -> String {
    // Only pages are tabs: service workers and frames cannot be selected or closed by title.
    let pages: Vec<&Value> =
        value.get("tabs").and_then(Value::as_array).map(|tabs| tabs.iter().filter(|tab| tab.get("type").and_then(Value::as_str) == Some("page")).collect()).unwrap_or_default();
    if pages.is_empty() {
        return "Chrome has no open tabs. Use page open \"URL\" to open one.".into();
    }
    let names: Vec<_> = pages.iter().take(12).map(|tab| display_n(tab.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("untitled"), 80)).collect();
    let more = if pages.len() > names.len() { format!(", and {} more", pages.len() - names.len()) } else { String::new() };
    let truncated = if value.get("truncated").and_then(Value::as_bool) == Some(true) { " More tabs may be open." } else { "" };
    bounded_output(format!("Chrome has {} tabs: {}{more}.{truncated}", pages.len(), names.join("; ")), 640)
}

pub(crate) fn format_browser_page(value: &Value) -> String {
    let title = display_n(value.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("untitled"), 96);
    let host = value.get("url").and_then(Value::as_str).and_then(|url| url.split("://").nth(1)).and_then(|rest| rest.split('/').next()).unwrap_or("");
    let rows = value.get("n").and_then(Value::as_array).cloned().unwrap_or_default();
    let controls: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let row = row.as_array()?;
            let label = row.get(1).and_then(Value::as_str).filter(|label| !label.trim().is_empty())?;
            let role = row.get(2).and_then(Value::as_str).and_then(|s| s.as_bytes().first().copied()).unwrap_or(b'm');
            Some(format!("{} {}", display_n(label, 72), role_name(role)))
        })
        .take(30)
        .collect();
    let page = if host.is_empty() { format!("The current page is {title}.") } else { format!("The current page is {title} ({}).", display_n(host, 64)) };
    if controls.is_empty() {
        format!("{page} It has no readable controls; read page text for its content.")
    } else {
        bounded_output_with_tail(format!("{page} Controls: {}.", controls.join("; ")), SCREEN_BYTES, " Read page text for the page content.")
    }
}

pub(crate) fn format_page_text(value: &Value, matching: bool) -> String {
    let text = display(value.get("text").and_then(Value::as_str).unwrap_or(""));
    match (text.is_empty(), matching) {
        (true, true) => return "No matching page text was found. Read page text for a broader view.".into(),
        (true, false) => return "The page has no readable text yet. Read the page again after it loads.".into(),
        _ => {}
    }
    let limit = if matching { 900 } else { 2400 };
    let excerpt: String = text.chars().take(limit).collect();
    let omitted = text.chars().count() > limit;
    format!("Page text: {excerpt}{}", if omitted { "\nMore text was omitted; use page text matching \"TEXT\" to jump to a part of the page." } else { "" })
}

// ---- Device state -------------------------------------------------------

pub(crate) fn format_capabilities(value: &Value) -> String {
    let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let hardware = |present: bool, granted: bool| match (present, granted) {
        (false, _) => "not available",
        (true, true) => "ready",
        (true, false) => "needs permission",
    };
    let parts = [
        format!("screenshots {}", if flag("screen_capture") { "ready" } else { "not available" }),
        format!("camera {}", hardware(flag("camera"), flag("camera_permission"))),
        format!("microphone {}", hardware(flag("microphone"), flag("microphone_permission"))),
        format!("location {}", if flag("location_permission") { "ready" } else { "needs permission" }),
        format!("screen recording {}", if flag("screen_record") { "ready" } else { "needs approval" }),
        format!("notifications {}", if flag("notifications") { "ready" } else { "need notification access" }),
    ];
    let mut text = format!("Capabilities: {}.", parts.join("; "));
    if parts.iter().any(|part| part.contains("need")) {
        text.push_str(" The user grants permissions and approvals in the Android Use app on the device.");
    }
    text
}

pub(crate) fn format_location(value: &Value) -> String {
    if value.get("permission").and_then(Value::as_bool) == Some(false) {
        return "Location permission is not granted to Android Use. Ask the user to allow location for Android Use on the device, then retry.".into();
    }
    let (Some(lat), Some(lon)) = (value.get("lat").and_then(Value::as_f64), value.get("lon").and_then(Value::as_f64)) else {
        return "No location fix is available right now. Location may be turned off on the device.".into();
    };
    let mut text = format!("Location: {lat:.5}, {lon:.5}");
    if let Some(accuracy) = value.get("acc").and_then(Value::as_f64).filter(|accuracy| *accuracy >= 0.0) {
        text.push_str(&format!(", accurate to about {} m", accuracy.round() as u64));
    }
    if value.get("current").and_then(Value::as_bool) == Some(true) {
        text.push_str(", current fix.");
    } else {
        let minutes = value.get("age_ms").and_then(Value::as_u64).unwrap_or(0) / 60_000;
        text.push_str(&match minutes {
            0 => ", last known fix from under a minute ago.".to_owned(),
            1..=119 => format!(", last known fix from {minutes} min ago."),
            _ => format!(", last known fix from {} h ago.", minutes / 60),
        });
    }
    text
}

pub(crate) fn format_notifications(value: &Value) -> String {
    if value.get("enabled").and_then(Value::as_bool) != Some(true) {
        return NOTIFICATION_ACCESS_OFF.into();
    }
    let items = crate::resolve::notification_items(value);
    if items.is_empty() {
        return if value.get("active").and_then(Value::as_bool) == Some(false) {
            "Notification access is on, but the listener has not connected yet. Retry in a few seconds.".into()
        } else {
            "There are no notifications.".into()
        };
    }
    // Listed and targeted by the same label (title, or text when untitled), so a number shown
    // here selects the same item in `dismiss notification` and the other notification commands.
    let label = |item: &Value| display_n(&crate::resolve::notification_label(item), 120);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for item in &items {
        *counts.entry(normalized(&label(item))).or_default() += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut lines = vec![format!("{} notifications, newest first:", items.len())];
    for item in items.iter().take(16) {
        let (title, text) = (display_n(item.get("title").and_then(Value::as_str).unwrap_or(""), 120), display_n(item.get("text").and_then(Value::as_str).unwrap_or(""), 120));
        let shown = label(item);
        let key = normalized(&shown);
        let number = if counts.get(&key).copied().unwrap_or(0) > 1 {
            let n = seen.entry(key).or_default();
            *n += 1;
            format!(" (number {n})")
        } else {
            String::new()
        };
        let shown = if shown.is_empty() { "(no title or text)".to_owned() } else { shown };
        let detail = if title.is_empty() || text.is_empty() { String::new() } else { format!(" — {text}") };
        lines.push(format!("- {shown}{number}{detail}"));
    }
    bounded_output(lines.join("\n"), SCREEN_BYTES)
}

pub(crate) fn format_image_hash(alias: &str, value: &Value) -> String {
    let hash = display_n(value.get("hash").and_then(Value::as_str).unwrap_or("unavailable"), 96);
    format!("Image {} has hash {hash}.", quoted(alias))
}

pub(crate) fn format_image_difference(left: &str, right: &str, value: &Value) -> String {
    let verdict = if value.get("changed").and_then(Value::as_u64) == Some(1) { "are different" } else { "match" };
    format!("Images {} and {} {verdict}.", quoted(left), quoted(right))
}

// ---- Action results ----------------------------------------------------

/// Imperative description of one step, used to name what failed.
pub(crate) fn describe_action(action: &Action) -> String {
    use AndroidAction as A;
    use BrowserAction as B;
    match action {
        Action::Android(action) => match action {
            A::Tap(t) => format!("tap {}", quoted(&t.label)),
            A::Toggle(t) => format!("toggle {}", quoted(&t.label)),
            A::Hold(t) => format!("hold {}", quoted(&t.label)),
            A::Type { target, .. } => format!("type into {}", quoted(&target.label)),
            A::Scroll { direction, target } => format!("scroll {} in {}", format!("{direction:?}").to_lowercase(), quoted(&target.label)),
            A::ScrollScreen(direction) => format!("scroll {}", format!("{direction:?}").to_lowercase()),
            A::Key(key) => format!("press {}", format!("{key:?}").to_lowercase()),
            A::WaitTarget { target, .. } => format!("wait for {}", quoted(&target.label)),
            A::WaitText { text, .. } => format!("wait for text {}", quoted(text)),
            A::WaitScreenChange { .. } => "wait for the screen to change".into(),
            A::VerifyExists(t) => format!("verify {} exists", quoted(&t.label)),
            A::VerifyGone(t) => format!("verify {} is gone", quoted(&t.label)),
            A::VerifyText(text) => format!("verify text {} exists", quoted(text)),
            A::OpenApp(name) => format!("open app {}", quoted(name)),
            A::OpenSetting(name) => format!("open setting {}", quoted(name)),
            A::OpenLink(url) => format!("open link {}", quoted(url)),
            A::CaptureScreen => "capture the screen".into(),
            A::Camera { .. } => "take a photo".into(),
            A::Microphone(_) => "record the microphone".into(),
            A::ScreenRecord(_) => "record the screen".into(),
            A::NotificationOpen(t) => format!("open notification {}", quoted(&t.label)),
            A::NotificationDismiss(t) => format!("dismiss notification {}", quoted(&t.label)),
            A::NotificationAction(t) => format!("run notification action {}", quoted(&t.label)),
            A::PointTap { x, y } => format!("tap point {x} {y}"),
            A::Swipe { .. } => "swipe".into(),
        },
        Action::Browser(action) => match action {
            B::Open(url) => format!("page open {}", quoted(url)),
            B::Click(t) => format!("page click {}", quoted(&t.label)),
            B::Focus(t) => format!("page focus {}", quoted(&t.label)),
            B::Type { target, .. } => format!("page type into {}", quoted(&target.label)),
            B::Key(key) => format!("page press {}", quoted(key)),
            B::Scroll(px) => format!("page scroll {px}"),
            B::WaitText { text, .. } => format!("page wait for text {}", quoted(text)),
            B::WaitCss { selector, .. } => format!("page wait for css {}", quoted(selector)),
            B::Back => "page back".into(),
            B::Forward => "page forward".into(),
            B::Reload => "page reload".into(),
            B::Screenshot => "page screenshot".into(),
            B::SelectTab(t) => format!("select tab {}", quoted(&t.label)),
            B::CloseTab(t) => format!("close tab {}", quoted(&t.label)),
            B::NewTab(url) => format!("new tab {}", quoted(url)),
        },
        Action::Visual(_) => "crop the image".into(),
    }
}

/// Past-tense description of one completed step.
fn done_phrase(action: &Action) -> String {
    use AndroidAction as A;
    use BrowserAction as B;
    let short = |value: &str| display_n(value, 64);
    match action {
        Action::Android(A::Tap(t)) => format!("tapped {}", short(&t.label)),
        Action::Android(A::Toggle(t)) => format!("toggled {}", short(&t.label)),
        Action::Android(A::Hold(t)) => format!("held {}", short(&t.label)),
        Action::Android(A::Type { text, target }) | Action::Browser(B::Type { text, target }) => format!("typed {} in {}", short(text), short(&target.label)),
        Action::Android(A::VerifyExists(t)) => format!("verified {} exists", short(&t.label)),
        Action::Android(A::VerifyGone(t)) => format!("verified {} is gone", short(&t.label)),
        Action::Android(A::VerifyText(text)) => format!("verified that {} appeared", short(text)),
        Action::Android(A::Scroll { direction, .. } | A::ScrollScreen(direction)) => format!("scrolled {}", format!("{direction:?}").to_lowercase()),
        Action::Android(A::Key(key)) => format!("pressed {}", format!("{key:?}").to_lowercase()),
        Action::Android(A::OpenApp(name)) => format!("opened {}", short(name)),
        Action::Android(A::OpenSetting(name)) => format!("opened {} settings", short(name)),
        Action::Android(A::OpenLink(_)) => "opened the link".into(),
        Action::Android(A::CaptureScreen) => "captured the screen".into(),
        Action::Android(A::Camera { .. }) => "took a photo".into(),
        Action::Android(A::Microphone(_)) => "recorded audio".into(),
        Action::Android(A::ScreenRecord(_)) => "recorded the screen".into(),
        Action::Android(A::NotificationOpen(t)) => format!("opened the {} notification", short(&t.label)),
        Action::Android(A::NotificationDismiss(t)) => format!("dismissed the {} notification", short(&t.label)),
        Action::Android(A::NotificationAction(t)) => format!("ran the {} notification action", short(&t.label)),
        Action::Android(A::PointTap { x, y }) => format!("tapped point {x} {y}"),
        Action::Android(A::Swipe { .. }) => "swiped".into(),
        Action::Android(A::WaitTarget { .. } | A::WaitText { .. } | A::WaitScreenChange { .. }) | Action::Browser(B::WaitText { .. } | B::WaitCss { .. }) => {
            "met the wait condition".into()
        }
        Action::Browser(B::Open(url)) => format!("opened {}", short(url)),
        Action::Browser(B::Click(t)) => format!("clicked {}", short(&t.label)),
        Action::Browser(B::Focus(t)) => format!("focused {}", short(&t.label)),
        Action::Browser(B::Key(key)) => format!("pressed {}", short(key)),
        Action::Browser(B::Scroll(_)) => "scrolled the page".into(),
        Action::Browser(B::Back) => "went back".into(),
        Action::Browser(B::Forward) => "went forward".into(),
        Action::Browser(B::Reload) => "reloaded the page".into(),
        Action::Browser(B::Screenshot) => "captured the page".into(),
        Action::Browser(B::SelectTab(t)) => format!("switched to the {} tab", short(&t.label)),
        Action::Browser(B::CloseTab(t)) => format!("closed the {} tab", short(&t.label)),
        Action::Browser(B::NewTab(_)) => "opened a new tab".into(),
        Action::Visual(_) => "cropped the image".into(),
    }
}

pub(crate) fn success_text(actions: &[Action]) -> String {
    const SHOWN: usize = 6;
    let mut parts: Vec<String> = actions.iter().take(SHOWN).map(done_phrase).collect();
    if actions.len() > SHOWN {
        parts.push(format!("{} more steps", actions.len() - SHOWN));
    }
    let text = match parts.len() {
        0 => return "Done.".into(),
        1 => format!("Done. {}.", capitalize(parts.remove(0))),
        2 => format!("Done. {} and {}.", capitalize(parts.remove(0)), parts.remove(0)),
        _ => {
            let last = parts.pop().unwrap_or_default();
            format!("Done. {}, and {last}.", capitalize(parts.join(", ")))
        }
    };
    bounded_output(text, 320)
}

fn wait_condition(action: &Action) -> Option<(u16, String)> {
    match action {
        Action::Android(AndroidAction::WaitTarget { target, seconds }) => Some((*seconds, format!("{} to appear", quoted(&target.label)))),
        Action::Android(AndroidAction::WaitText { text, seconds }) => Some((*seconds, format!("text {} to appear", quoted(text)))),
        Action::Android(AndroidAction::WaitScreenChange { seconds }) => Some((*seconds, "the screen to change".into())),
        Action::Browser(BrowserAction::WaitText { text, seconds }) => Some((*seconds, format!("page text {} to appear", quoted(text)))),
        Action::Browser(BrowserAction::WaitCss { selector, seconds }) => Some((*seconds, format!("an element matching {} to appear", quoted(selector)))),
        _ => None,
    }
}

/// Explains a failed receipt: what failed, whether anything changed, and the
/// single safe next move.
pub(crate) fn failure_text(receipt: &Receipt, actions: &[Action]) -> String {
    let step = receipt.at.map(usize::from).filter(|index| *index < actions.len()).map(|index| (index, &actions[index]));
    let what = step.map_or_else(|| "run the command".to_owned(), |(_, action)| describe_action(action));
    let which = step.map_or_else(|| "a later step".to_owned(), |(index, action)| format!("step {} of {} ({})", index + 1, actions.len(), describe_action(action)));
    let text = if receipt.m > 0 && receipt.e.as_deref() == Some("unknown") {
        format!("Partly done, and uncertain: at least {} change(s) were made, and {which} may or may not have finished. Read the screen before doing anything else, and do not repeat the whole command.", receipt.m)
    } else if receipt.m > 0 || receipt.e.as_deref() == Some("partial") {
        format!("Partly done: {} change(s) were made before {which} failed. Read the screen before doing anything else, and do not repeat the whole command.", receipt.m)
    } else {
        match receipt.e.as_deref() {
            Some("stale") => "The screen changed before anything ran, so nothing was done. Retry the same command.".into(),
            Some("unknown") => "The result is uncertain: the command may or may not have run. Read the screen before doing anything else, and do not repeat it blindly.".into(),
            Some("permission") => {
                format!("Android Use is missing the Android permission needed to {what}. Ask the user to grant it in the Android Use app on the device, then retry.")
            }
            Some("timeout") => format!("Timed out at {which}; nothing changed. Retry if it is still needed."),
            Some("unsupported") => format!("Cannot {what} on this device or Android version; nothing changed."),
            Some("bounds") => format!("Cannot {what}: it exceeds a safety limit. Nothing changed."),
            Some("ambiguous") => "The target is ambiguous; nothing changed. Read the screen and add number N after the label.".into(),
            _ => match step {
                Some((_, action)) if wait_condition(action).is_some() => {
                    let (seconds, condition) = wait_condition(action).unwrap_or_default();
                    format!("Waited {seconds} seconds for {condition}, but it did not happen. Nothing changed.")
                }
                Some((_, Action::Android(AndroidAction::VerifyExists(_) | AndroidAction::VerifyGone(_) | AndroidAction::VerifyText(_)))) => {
                    format!("Check failed: {what} is not true on the current screen. Nothing changed.")
                }
                Some(_) => format!("Could not {what} on the current screen; nothing changed. Read the screen and choose another target."),
                None => "The command failed before it changed anything. Read the screen, then retry if it is still needed.".into(),
            },
        }
    };
    bounded_output(text, 480)
}

pub(crate) fn receipt_code(receipt: &Receipt) -> Code {
    if receipt.m > 0 {
        return Code::Partial;
    }
    match receipt.e.as_deref() {
        Some("stale") => Code::Stale,
        Some("unknown") => Code::Unknown,
        Some("timeout") => Code::Timeout,
        Some("permission") => Code::Permission,
        Some("ambiguous") => Code::Ambiguous,
        Some("unsupported") => Code::Unsupported,
        Some("bounds") => Code::Bounds,
        Some("args") => Code::Args,
        _ => Code::Helper,
    }
}

/// Turns any error into one short recovery instruction for an agent.
/// Internal diagnostics are summarized rather than echoed.
pub(crate) fn plain_error_model(error: &Error) -> String {
    let message = display(&error.message);
    if error.plain {
        return bounded_output(message, 720);
    }
    match error.code {
        Code::Args | Code::Bounds | Code::Unsupported if message.contains("Use ") => bounded_output(message, 320),
        Code::Args | Code::Bounds | Code::Unsupported => bounded_output(format!("The command needs a correction: {message}."), 320),
        Code::Ambiguous | Code::Missing | Code::Partial => bounded_output(message, 480),
        Code::Permission => "Android Use's Accessibility service is off or a required Android permission is missing. Ask the user to turn on Android Use under Settings → Accessibility on the device (and grant any permission the task needs), then retry.".into(),
        Code::Artifact => bounded_output(format!("{message}. Capture the screen before using an image alias."), 320),
        Code::Stale => "The screen changed before anything ran, so nothing was done. Retry the same command.".into(),
        Code::Unknown => "The result is uncertain: the command may or may not have run. Read the screen before doing anything else, and do not repeat it blindly.".into(),
        Code::Identity => "The enrolled Android device is not connected. Ask the user to connect and unlock it and approve USB debugging if Android asks, then retry. A different device needs au setup first.".into(),
        Code::Device if message.contains("adb was not found") => "Android platform tools (adb) were not found on this computer. Ask the user to install Android SDK Platform-Tools or set AU_ADB, then run au doctor.".into(),
        Code::Device => bounded_output(format!("The computer cannot reach the Android device ({message}). Ask the user to connect and unlock it with a data cable and approve USB debugging; au doctor explains what is missing."), 480),
        Code::Timeout => "The device took too long to respond. Make sure it is unlocked and awake, then retry.".into(),
        Code::Helper | Code::Auth | Code::Sequence | Code::Protocol | Code::Io => {
            "The Android Use helper on the device did not respond correctly. Retry once; if it fails again, ask the user to run au doctor, and au repair if the helper is broken.".into()
        }
    }
}

pub(crate) fn mime_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") {
        "audio/wav"
    } else if bytes.get(4..8) == Some(b"ftyp") {
        "video/mp4"
    } else {
        "application/octet-stream"
    }
}

pub(crate) fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "audio/wav" => "wav",
        "video/mp4" => "mp4",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Target;
    use serde_json::json;

    fn row(label: &str, kind: &str, value: &str, state: &str) -> SemanticRow {
        SemanticRow { label: label.into(), value: value.into(), kind: kind.into(), state: state.into(), enabled: true, selected: false }
    }

    #[test]
    fn model_text_is_plain_and_bounded() {
        let scene = Scene {
            observation: "1".into(),
            generation: 1,
            package: "fixture".into(),
            nodes: vec![
                Node { id: 0, label: "Save".into(), role: b'b', flags: 3 },
                Node { id: 1, label: "Name".into(), role: b'i', flags: 2 },
                Node { id: 2, label: "com.android:id/internal_text".into(), role: b't', flags: 2 },
                Node { id: 3, label: "com.android:id/submit_button".into(), role: b'b', flags: 3 },
            ]
            .into_boxed_slice(),
        };
        let text = format_scene(&scene, false);
        assert!(!text.contains(['{', '}', '[', ']']));
        assert!(!text.contains("com.android"));
        assert!(text.contains("submit button"));
        assert!(text.len() <= SCREEN_BYTES);

        let actions = vec![
            Action::Android(AndroidAction::Type { text: "Sample text".into(), target: Target { label: "Name".into(), ordinal: None } }),
            Action::Android(AndroidAction::Tap(Target { label: "Save".into(), ordinal: None })),
            Action::Android(AndroidAction::VerifyText("Submitted".into())),
        ];
        assert_eq!(success_text(&actions), "Done. Typed Sample text in Name, tapped Save, and verified that Submitted appeared.");
    }

    #[test]
    fn duplicate_rows_show_the_number_an_agent_must_use() {
        let rows = [row("Settings", "heading", "", ""), row("Save", "button", "", ""), row("Wi-Fi", "switch", "", "checked"), row("Save", "button", "", "")];
        let text = format_semantic_rows(&rows, false, "");
        assert_eq!(text, "Settings\nSave (number 1) — button\nWi-Fi — on switch\nSave (number 2) — button");
        assert_eq!(semantic_line(&row("Name", "text field", "", "")), "Name — empty text field");
    }

    #[test]
    fn device_state_reads_report_values_not_field_names() {
        let location = format_location(&json!({"permission":true,"available":true,"current":true,"lat":47.606_21,"lon":-122.332_07,"acc":12.4,"age_ms":0}));
        assert_eq!(location, "Location: 47.60621, -122.33207, accurate to about 12 m, current fix.");
        assert!(format_location(&json!({"permission":false,"available":false})).contains("not granted"));
        let notifications = format_notifications(&json!({"enabled":true,"active":true,"items":[
            {"id":"a","title":"Messages","text":"Lunch?"},{"id":"b","title":"Messages","text":"Running late"},{"id":"c","title":"Calendar","text":""}
        ]}));
        assert_eq!(notifications, "3 notifications, newest first:\n- Calendar\n- Messages (number 1) — Running late\n- Messages (number 2) — Lunch?");
        assert_eq!(format_notifications(&json!({"enabled":false,"items":[]})), NOTIFICATION_ACCESS_OFF);
        let capabilities = format_capabilities(&json!({"screen_capture":true,"camera":true,"camera_permission":false,"microphone":true,"microphone_permission":true}));
        assert!(capabilities.contains("camera needs permission") && capabilities.contains("microphone ready"));
    }

    #[test]
    fn failures_name_the_step_and_whether_anything_changed() {
        let actions = vec![Action::Android(AndroidAction::Tap(Target { label: "Save".into(), ordinal: None })), Action::Android(AndroidAction::VerifyText("Saved".into()))];
        let failed = |m: u8, at: u8, e: &str| Receipt { id: "x".into(), ok: 0, g: 1, m, at: Some(at), e: Some(e.into()), partial: None, next: None, artifact: None };
        assert!(failure_text(&failed(1, 1, "partial"), &actions).contains("step 2 of 2 (verify text \"Saved\" exists)"));
        assert_eq!(receipt_code(&failed(1, 1, "partial")), Code::Partial);
        assert!(failure_text(&failed(1, 1, "unknown"), &actions).contains("step 2 of 2 (verify text \"Saved\" exists) may or may not have finished"));
        let verify = [Action::Android(AndroidAction::VerifyText("Saved".into()))];
        assert!(failure_text(&failed(0, 0, "helper"), &verify).starts_with("Check failed"));
        let wait = [Action::Android(AndroidAction::WaitText { text: "Done".into(), seconds: 5 })];
        assert!(failure_text(&failed(0, 0, "helper"), &wait).starts_with("Waited 5 seconds for text \"Done\" to appear"));
    }

    #[test]
    fn tabs_list_only_pages_and_say_when_there_are_more() {
        let tabs = json!({"tabs":[
            {"id":"p1","type":"page","title":"Docs"},
            {"id":"w1","type":"service_worker","title":"Worker"},
            {"id":"p2","type":"page","title":""}
        ], "truncated": false});
        assert_eq!(format_browser_tabs(&tabs), "Chrome has 2 tabs: Docs; untitled.");
        let many = json!({"tabs": (0..14).map(|i| json!({"id": format!("p{i}"), "type": "page", "title": format!("T{i}")})).collect::<Vec<_>>(), "truncated": true});
        assert!(format_browser_tabs(&many).contains("Chrome has 14 tabs: T0; T1; T2; T3; T4; T5; T6; T7; T8; T9; T10; T11, and 2 more. More tabs may be open."));
    }

    #[test]
    fn model_errors_hide_wire_syntax_and_give_a_next_step() {
        let error = Error::new(Code::Ambiguous, "Save matches two controls. Use tap \"Save\" number 1 or number 2.");
        let text = plain_error_model(&error);
        assert!(!text.contains(['{', '}', '[', ']']));
        assert!(plain_error_model(&Error::new(Code::Identity, "no connected transport matches")).contains("not connected"));
        assert!(plain_error_model(&Error::new(Code::Device, "adb was not found; set AU_ADB")).contains("Platform-Tools"));
        assert_eq!(plain_error_model(&Error::plain(Code::Helper, "Check failed.")), "Check failed.");
        assert_eq!(mime_type(b"\0\0\0\x18ftypmp42"), "video/mp4");
    }
}
