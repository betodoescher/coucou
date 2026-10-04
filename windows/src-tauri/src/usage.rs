// AI usage for the home card.
//
// Claude Code: today's tokens, read from ~/.claude/projects/*/*.jsonl. The
// format is undocumented: anything that doesn't parse counts as zero.
// `message.usage` is repeated per streamed chunk, so it is deduplicated by
// message id + request id.
// Claude plan limits (5 hours, week): the last ones Claude Code handed its
// status line, which coucou-hook relays.
// Cursor and Kiro: the plan usage their own `/usage` command shows.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub claude_tokens: u64,
    pub claude_plan: Option<ClaudePlan>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanWindow {
    /// 0–100.
    pub used_pct: f64,
    /// Unix seconds.
    pub resets_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePlan {
    pub five_hour: Option<PlanWindow>,
    pub seven_day: Option<PlanWindow>,
    /// Unix milliseconds.
    pub updated_at: i64,
}

/// `rate_limits` from Claude Code's status line input. Pro and Max only;
/// anything malformed is ignored rather than shown wrong.
pub fn parse_claude_plan(limits: &Value, now_ms: i64) -> Option<ClaudePlan> {
    let window = |key: &str| -> Option<PlanWindow> {
        let w = limits.get(key)?;
        let pct = w.get("used_percentage")?.as_f64()?;
        let resets_at = w.get("resets_at")?.as_f64()? as i64;
        // Beyond 400 days it is milliseconds, or nonsense.
        if !(0.0..=200.0).contains(&pct) || resets_at <= 0 || resets_at > now_ms / 1000 + 400 * 86_400 {
            return None;
        }
        Some(PlanWindow { used_pct: pct.min(100.0), resets_at })
    };
    let (five_hour, seven_day) = (window("five_hour"), window("seven_day"));
    (five_hour.is_some() || seven_day.is_some()).then_some(ClaudePlan { five_hour, seven_day, updated_at: now_ms })
}

fn plan_path() -> PathBuf {
    crate::settings::local_dir().join("claude-plan.json")
}

static LAST_PLAN: Mutex<Option<ClaudePlan>> = Mutex::new(None);

/// Keeps the latest limits. The status line runs on every message, so the
/// file is only rewritten when a number moved or a minute went by.
/// Returns whether anything new is worth telling the page.
pub fn save_claude_plan(plan: &ClaudePlan) -> bool {
    let mut last = LAST_PLAN.lock().unwrap();
    let fresh = last.as_ref().is_none_or(|l| {
        l.five_hour != plan.five_hour || l.seven_day != plan.seven_day || plan.updated_at - l.updated_at >= 60_000
    });
    if !fresh {
        return false;
    }
    if let Ok(text) = serde_json::to_string(plan) {
        let _ = std::fs::write(plan_path(), text);
    }
    *last = Some(plan.clone());
    true
}

fn load_claude_plan() -> Option<ClaudePlan> {
    if let Some(plan) = LAST_PLAN.lock().unwrap().clone() {
        return Some(plan);
    }
    serde_json::from_slice(&std::fs::read(plan_path()).ok()?).ok()
}

#[derive(Serialize, Default)]
pub struct Plans {
    pub cursor: Option<crate::cursor_chat::CursorPlan>,
    pub kiro: Option<crate::kiro_chat::KiroPlan>,
}

/// Usage since `since_ms` (the local midnight, from the page).
pub fn since(since_ms: i64) -> Usage {
    let home = crate::platform::home_dir();
    let cutoff = UNIX_EPOCH + Duration::from_millis(since_ms.max(0) as u64);
    let mut usage = Usage { claude_plan: load_claude_plan(), ..Usage::default() };

    let mut seen = HashSet::new();
    for file in recent(&home.join(".claude").join("projects"), 2, cutoff) {
        for line in std::fs::read_to_string(&file).unwrap_or_default().lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                usage.claude_tokens += claude_tokens(&v, since_ms, &mut seen);
            }
        }
    }
    usage
}

/// `.jsonl` files under `dir` (at most `depth` levels down) modified since `cutoff`.
fn recent(dir: &Path, depth: u32, cutoff: SystemTime) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            if depth > 1 {
                out.extend(recent(&path, depth - 1, cutoff));
            }
        } else if path.extension().is_some_and(|e| e == "jsonl") && meta.modified().is_ok_and(|m| m >= cutoff) {
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
    fn reads_the_plan_limits() {
        let now = 1_791_035_807_875;
        let limits = json!({
            "five_hour": { "used_percentage": 42.5, "resets_at": 1_791_040_000 },
            "seven_day": { "used_percentage": 130, "resets_at": 1_791_400_000 }
        });
        let plan = parse_claude_plan(&limits, now).unwrap();
        assert_eq!(plan.five_hour, Some(PlanWindow { used_pct: 42.5, resets_at: 1_791_040_000 }));
        assert_eq!(plan.seven_day.unwrap().used_pct, 100.0);
        assert_eq!(plan.updated_at, now);

        // Milliseconds instead of seconds, or a silly percentage, are dropped.
        let bad = json!({
            "five_hour": { "used_percentage": 10, "resets_at": 1_791_040_000_000_i64 },
            "seven_day": { "used_percentage": 250, "resets_at": 1_791_400_000 }
        });
        assert!(parse_claude_plan(&bad, now).is_none());
        assert!(parse_claude_plan(&json!({}), now).is_none());
        let half = parse_claude_plan(&json!({ "seven_day": { "used_percentage": 3, "resets_at": 1_791_400_000 } }), now).unwrap();
        assert!(half.five_hour.is_none());
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
    }
}
