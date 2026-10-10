use au::api::{parse_read, Plan};
use serde_json::Value;

fn next(state: &mut u64) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state as u8
}

#[test]
fn bounded_random_agent_inputs_never_panic() {
    let mut state = 0x9e3779b97f4a7c15;
    for case in 0..10_000 {
        let len = (next(&mut state) as usize * 4 + case % 17).min(1024);
        let mut bytes = vec![0; len];
        for byte in &mut bytes {
            *byte = next(&mut state);
        }
        let run = std::panic::catch_unwind(|| {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                let _ = Plan::parse(value.clone());
                let _ = parse_read(value);
            }
        });
        assert!(run.is_ok(), "parser panicked on case {case}");
    }
}

#[test]
fn mutated_golden_plan_never_bypasses_limits() {
    let golden: Value = serde_json::from_str(include_str!("../../protocol-golden.json")).unwrap();
    for count in 0..80 {
        let mut plan = golden["plan"].clone();
        plan["p"] = Value::Array((0..count).map(|_| serde_json::json!(["assert", ["text", "x"]])).collect());
        let result = Plan::parse(plan);
        assert_eq!(result.is_ok(), (1..=32).contains(&count));
    }
}

/// Random sequences of grammar words, quoted values, numbers, and junk must
/// never panic the command parser, and anything it accepts stays in bounds.
#[test]
fn command_grammar_fuzz_never_panics_or_exceeds_limits() {
    const WORDS: &[&str] = &[
        "tap",
        "hold",
        "toggle",
        "type",
        "in",
        "into",
        "scroll",
        "up",
        "down",
        "left",
        "press",
        "back",
        "home",
        "enter",
        "wait",
        "for",
        "text",
        "screen",
        "change",
        "to",
        "seconds",
        "verify",
        "exists",
        "is",
        "gone",
        "open",
        "app",
        "setting",
        "link",
        "capture",
        "take",
        "rear",
        "camera",
        "photo",
        "record",
        "microphone",
        "page",
        "click",
        "focus",
        "reload",
        "screenshot",
        "select",
        "close",
        "new",
        "tab",
        "notification",
        "dismiss",
        "run",
        "action",
        "point",
        "swipe",
        "from",
        "over",
        "milliseconds",
        "crop",
        "image",
        "with",
        "size",
        "by",
        "number",
        "then",
        "THEN",
        "full",
        "matching",
        "find",
        "status",
        "changes",
    ];
    const QUOTED: &[&str] = &["\"Save\"", "\"\"", "\"a\\\"b\"", "\"https://example.com\"", "\"wifi\"", "\"☃ then\"", "\"unterminated", "\"x\"y", "\"\\q\""];
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    for case in 0..20_000 {
        let mut parts = Vec::new();
        for _ in 0..(next(&mut state) % 24) {
            parts.push(match next(&mut state) % 10 {
                0..=5 => WORDS[next(&mut state) as usize % WORDS.len()].to_owned(),
                6 | 7 => QUOTED[next(&mut state) as usize % QUOTED.len()].to_owned(),
                8 => (u32::from(next(&mut state)) * 401).to_string(),
                _ => char::from_u32(u32::from(next(&mut state)) * 37 + 32).unwrap_or('?').to_string(),
            });
        }
        let command = parts.join(" ");
        let run = std::panic::catch_unwind(|| (au::api::parse_act_command(&command).map(|actions| actions.len()), au::api::parse_read_command(&command).is_ok()));
        let (actions, _) = run.unwrap_or_else(|_| panic!("parser panicked on case {case}: {command}"));
        if let Ok(count) = actions {
            assert!((1..=32).contains(&count), "case {case} accepted {count} actions: {command}");
        }
    }
}
