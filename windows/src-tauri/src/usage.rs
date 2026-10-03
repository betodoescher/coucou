// Today's AI usage, read from the agents' own local logs. No network.
// The formats are undocumented: anything that doesn't parse counts as zero.
//
// Claude Code: ~/.claude/projects/*/*.jsonl, `message.usage` on assistant lines,
// repeated per streamed chunk, so deduplicated by message id + request id.
// Kiro: ~/.kiro/sessions/cli/*.json (`metering_usage` per turn) and
// ~/.kiro/sessions/*/sess_*/messages.jsonl (`usage_summary` lines).

use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub claude_tokens: u64,
    pub kiro_credits: f64,
}

/// Usage since `since_ms` (the local midnight, from the page).
pub fn since(since_ms: i64) -> Usage {
    let home = crate::platform::home_dir();
    let cutoff = UNIX_EPOCH + Duration::from_millis(since_ms.max(0) as u64);
    let mut usage = Usage::default();

    let mut seen = HashSet::new();
    for file in recent(&home.join(".claude").join("projects"), 2, cutoff, |p| ext(p, "jsonl")) {
        for line in read(&file).lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                usage.claude_tokens += claude_tokens(&v, since_ms, &mut seen);
            }
        }
    }

    let kiro = home.join(".kiro").join("sessions");
    for file in recent(&kiro.join("cli"), 1, cutoff, |p| ext(p, "json")) {
        if let Ok(v) = serde_json::from_str::<Value>(&read(&file)) {
            usage.kiro_credits += kiro_cli_credits(&v, since_ms);
        }
    }
    for file in recent(&kiro, 3, cutoff, |p| p.file_name().is_some_and(|n| n == "messages.jsonl")) {
        for line in read(&file).lines().filter(|l| l.contains("usage_summary")) {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                usage.kiro_credits += kiro_summary_credits(&v, since_ms);
            }
        }
    }
    usage
}

fn ext(p: &Path, want: &str) -> bool {
    p.extension().is_some_and(|e| e == want)
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// Files under `dir` (at most `depth` levels down) modified since `cutoff`.
fn recent(dir: &Path, depth: u32, cutoff: SystemTime, keep: impl Fn(&Path) -> bool + Copy) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            if depth > 1 {
                out.extend(recent(&path, depth - 1, cutoff, keep));
            }
        } else if keep(&path) && meta.modified().is_ok_and(|m| m >= cutoff) {
            out.push(path);
        }
    }
    out
}

fn claude_tokens(v: &Value, since_ms: i64, seen: &mut HashSet<String>) -> u64 {
    let Some(usage) = v.pointer("/message/usage") else { return 0 };
    if !v.get("timestamp").and_then(Value::as_str).and_then(rfc3339_ms).is_some_and(|t| t >= since_ms) {
        return 0;
    }
    let id = v.pointer("/message/id").and_then(Value::as_str).unwrap_or("");
    let request = v.get("requestId").and_then(Value::as_str).unwrap_or("");
    if !id.is_empty() && !seen.insert(format!("{id}:{request}")) {
        return 0;
    }
    ["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
        .iter()
        .filter_map(|k| usage.get(k).and_then(Value::as_u64))
        .sum()
}

fn kiro_cli_credits(v: &Value, since_ms: i64) -> f64 {
    let Some(turns) = v.pointer("/session_state/conversation_metadata/user_turn_metadatas").and_then(Value::as_array)
    else {
        return 0.0;
    };
    turns
        .iter()
        .filter(|t| t.get("end_timestamp").and_then(Value::as_str).and_then(rfc3339_ms).is_some_and(|t| t >= since_ms))
        .flat_map(|t| t.get("metering_usage").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter(|m| m.get("unit").and_then(Value::as_str) == Some("credit"))
        .filter_map(|m| m.get("value").and_then(Value::as_f64))
        .sum()
}

fn kiro_summary_credits(v: &Value, since_ms: i64) -> f64 {
    if !v.get("timestamp").and_then(Value::as_str).and_then(rfc3339_ms).is_some_and(|t| t >= since_ms) {
        return 0.0;
    }
    v.pointer("/payload/promptTurnSummaries")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|s| s.get("unit").and_then(Value::as_str) == Some("credit"))
                .filter_map(|s| s.get("usage").and_then(Value::as_f64))
                .sum()
        })
        .unwrap_or(0.0)
}

/// "2026-10-03T13:28:40.019976657Z" or "...+02:00" → Unix milliseconds.
fn rfc3339_ms(s: &str) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let rest = s.get(19..)?;
    let (frac, zone) = match rest.strip_prefix('.') {
        Some(r) => {
            let digits = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
            (&r[..digits], &r[digits..])
        }
        None => ("", rest),
    };
    let ms = format!("{frac:0<3}").get(..3)?.parse::<i64>().ok()?;
    let offset_min = match zone {
        "Z" | "z" => 0,
        z if z.len() == 6 => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            sign * (z.get(1..3)?.parse::<i64>().ok()? * 60 + z.get(4..6)?.parse::<i64>().ok()?)
        }
        _ => return None,
    };
    // Days from civil (Howard Hinnant).
    let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3_600 + mi * 60 + sec - offset_min * 60) * 1_000) + ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_timestamps() {
        assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_ms("2026-10-03T13:56:47.875Z"), Some(1_791_035_807_875));
        assert_eq!(rfc3339_ms("2026-10-03T13:28:40.019976657Z"), Some(1_791_034_120_019));
        assert_eq!(rfc3339_ms("2026-10-03T10:56:47.875-03:00"), Some(1_791_035_807_875));
        assert_eq!(rfc3339_ms("garbage"), None);
    }

    #[test]
    fn counts_today_only_and_once() {
        let since = rfc3339_ms("2026-10-03T03:00:00Z").unwrap();
        let line = |ts: &str, id: &str| json!({
            "timestamp": ts, "requestId": "r1",
            "message": { "id": id, "usage": {
                "input_tokens": 10, "output_tokens": 20,
                "cache_creation_input_tokens": 30, "cache_read_input_tokens": 40
            } }
        });
        let mut seen = HashSet::new();
        assert_eq!(claude_tokens(&line("2026-10-03T12:00:00Z", "m1"), since, &mut seen), 100);
        assert_eq!(claude_tokens(&line("2026-10-03T12:00:01Z", "m1"), since, &mut seen), 0);
        assert_eq!(claude_tokens(&line("2026-10-02T12:00:00Z", "m2"), since, &mut seen), 0);

        let cli = json!({ "session_state": { "conversation_metadata": { "user_turn_metadatas": [
            { "end_timestamp": "2026-10-03T13:28:40.019976657Z", "metering_usage": [{ "value": 0.35, "unit": "credit" }] },
            { "end_timestamp": "2026-10-02T13:28:40Z", "metering_usage": [{ "value": 9.0, "unit": "credit" }] },
            { "end_timestamp": null, "metering_usage": [{ "value": 9.0, "unit": "credit" }] }
        ] } } });
        assert_eq!(kiro_cli_credits(&cli, since), 0.35);

        let summary = json!({ "timestamp": "2026-10-03T13:56:47.875Z", "payload": {
            "type": "usage_summary", "promptTurnSummaries": [{ "unit": "credit", "usage": 0.12 }]
        } });
        assert_eq!(kiro_summary_credits(&summary, since), 0.12);
    }
}
