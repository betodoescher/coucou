//! File edits → a small diff the island can show: the file, `+N −M`, and the
//! changed lines. Computed here, before the payload's strings are cut to
//! `MAX_FIELD_LEN`, so the counts are those of the whole edit.

use serde_json::{json, Map, Value};

/// Lines kept for the island's diff card.
const MAX_LINES: usize = 80;
/// Longest line kept; the card is a few hundred pixels wide.
const MAX_LINE_LEN: usize = 160;
/// Above this many cells the middle of an edit is counted, not aligned.
const MAX_CELLS: usize = 250_000;

/// Inputs that can carry a whole file. Once the diff is computed they are
/// dead weight on the wire.
const BULKY: &[&str] = &["old_string", "new_string", "content", "file_text", "old_str", "new_str", "edits"];

#[derive(Default)]
struct Diff {
    added: usize,
    removed: usize,
    lines: Vec<String>,
}

impl Diff {
    fn push(&mut self, line: String) {
        if self.lines.len() < MAX_LINES {
            self.lines.push(cut(line));
        }
    }

    /// One replacement: `old` becomes `new`. Successive edits are separated.
    fn replace(&mut self, old: &str, new: &str) {
        if !self.lines.is_empty() {
            self.push("…".into());
        }
        let a: Vec<&str> = if old.is_empty() { vec![] } else { old.lines().collect() };
        let b: Vec<&str> = if new.is_empty() { vec![] } else { new.lines().collect() };
        let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
        let (a, b) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
        if a.len().saturating_mul(b.len()) > MAX_CELLS {
            self.removed += a.len();
            self.added += b.len();
            a.iter().for_each(|l| self.push(format!("-{l}")));
            b.iter().for_each(|l| self.push(format!("+{l}")));
            return;
        }
        for op in align(a, b) {
            match op {
                Op::Keep(l) => self.push(format!(" {l}")),
                Op::Del(l) => {
                    self.removed += 1;
                    self.push(format!("-{l}"));
                }
                Op::Add(l) => {
                    self.added += 1;
                    self.push(format!("+{l}"));
                }
            }
        }
    }

    /// Claude Code's own hunks (`tool_response.structuredPatch`).
    fn patch(&mut self, hunks: &[Value]) {
        for hunk in hunks {
            if !self.lines.is_empty() {
                self.push("…".into());
            }
            for line in hunk.get("lines").and_then(Value::as_array).into_iter().flatten() {
                let Some(l) = line.as_str() else { continue };
                if l.starts_with('+') {
                    self.added += 1;
                } else if l.starts_with('-') {
                    self.removed += 1;
                }
                self.push(l.to_string());
            }
        }
    }
}

enum Op<'a> {
    Keep(&'a str),
    Del(&'a str),
    Add(&'a str),
}

/// Longest common subsequence of lines: deletions before additions.
fn align<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] = if a[i] == b[j] { lcs[at(i + 1, j + 1)] + 1 } else { lcs[at(i + 1, j)].max(lcs[at(i, j + 1)]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::with_capacity(n + m));
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            out.push(Op::Keep(a[i]));
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[at(i + 1, j)] >= lcs[at(i, j + 1)]) {
            out.push(Op::Del(a[i]));
            i += 1;
        } else {
            out.push(Op::Add(b[j]));
            j += 1;
        }
    }
    out
}

fn cut(mut s: String) -> String {
    if s.len() > MAX_LINE_LEN {
        let mut end = MAX_LINE_LEN;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('…');
    }
    s
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The diff of one file edit, or None when the event is not one.
/// Claude Code: PostToolUse of Edit / MultiEdit / Write. Cursor: afterFileEdit.
/// Kiro: PostToolUse of fs_write.
fn compute(agent: &str, event: &str, map: &Map<String, Value>) -> Option<(String, Diff)> {
    let tool = map.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let input = map.get("tool_input").cloned().unwrap_or(Value::Null);
    let mut d = Diff::default();
    match (agent, event) {
        ("cursor", "afterFileEdit") => {
            let path = map.get("file_path").and_then(Value::as_str)?.to_string();
            for e in map.get("edits")?.as_array()? {
                d.replace(text(e, "old_string"), text(e, "new_string"));
            }
            Some((path, d))
        }
        ("kiro", "PostToolUse") if tool == "fs_write" || tool == "write" => {
            let path = text(&input, "path").to_string();
            match text(&input, "command") {
                "create" => d.replace("", text(&input, "file_text")),
                "str_replace" => d.replace(text(&input, "old_str"), text(&input, "new_str")),
                "insert" | "append" => d.replace("", text(&input, "new_str")),
                _ => return None,
            }
            (!path.is_empty()).then_some((path, d))
        }
        ("", "PostToolUse") if matches!(tool, "Edit" | "MultiEdit" | "Write") => {
            let path = text(&input, "file_path").to_string();
            let hunks = map.get("tool_response").and_then(|r| r.get("structuredPatch")).and_then(Value::as_array);
            match hunks {
                Some(h) if !h.is_empty() => d.patch(h),
                _ => match tool {
                    "Write" => d.replace("", text(&input, "content")),
                    "Edit" => d.replace(text(&input, "old_string"), text(&input, "new_string")),
                    _ => {
                        for e in input.get("edits")?.as_array()? {
                            d.replace(text(e, "old_string"), text(e, "new_string"));
                        }
                    }
                },
            }
            (!path.is_empty()).then_some((path, d))
        }
        _ => None,
    }
}

/// Adds `coucou_diff` to a file-edit event and drops the edit's full text.
/// `event` is the name as the agent sent it.
pub fn attach(agent: &str, event: &str, map: &mut Map<String, Value>) {
    let Some((path, d)) = compute(agent, event, map) else { return };
    map.insert(
        "coucou_diff".into(),
        json!({ "path": path, "added": d.added, "removed": d.removed, "lines": d.lines }),
    );
    for key in BULKY {
        map.remove(*key);
        if let Some(input) = map.get_mut("tool_input").and_then(Value::as_object_mut) {
            input.remove(*key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff_of(agent: &str, event: &str, v: Value) -> Value {
        let mut map = v.as_object().unwrap().clone();
        attach(agent, event, &mut map);
        map.get("coucou_diff").cloned().unwrap_or(Value::Null)
    }

    #[test]
    fn claude_edit_counts_only_the_changed_lines() {
        let d = diff_of("", "PostToolUse", json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": "/p/a.rs", "old_string": "a\nb\nc", "new_string": "a\nB\nc\nd" }
        }));
        assert_eq!(d["path"], "/p/a.rs");
        assert_eq!((d["added"].as_u64(), d["removed"].as_u64()), (Some(2), Some(1)));
        assert_eq!(d["lines"], json!(["-b", "+B", " c", "+d"]));
    }

    #[test]
    fn claude_structured_patch_wins_and_bulky_text_goes() {
        let mut map = json!({
            "tool_name": "Write",
            "tool_input": { "file_path": "/p/b.ts", "content": "x".repeat(10_000) },
            "tool_response": { "structuredPatch": [{ "lines": [" keep", "-old", "+new", "+more"] }] }
        }).as_object().unwrap().clone();
        attach("", "PostToolUse", &mut map);
        assert_eq!(map["coucou_diff"]["added"], 2);
        assert_eq!(map["coucou_diff"]["removed"], 1);
        assert!(map["tool_input"].get("content").is_none());
        assert_eq!(map["tool_input"]["file_path"], "/p/b.ts");
    }

    #[test]
    fn cursor_and_kiro_edits() {
        let c = diff_of("cursor", "afterFileEdit", json!({
            "file_path": "/p/c.py",
            "edits": [{ "old_string": "x = 1", "new_string": "x = 2" }, { "old_string": "", "new_string": "y = 3" }]
        }));
        assert_eq!((c["added"].as_u64(), c["removed"].as_u64()), (Some(2), Some(1)));
        assert_eq!(c["lines"], json!(["-x = 1", "+x = 2", "…", "+y = 3"]));

        let k = diff_of("kiro", "PostToolUse", json!({
            "tool_name": "fs_write",
            "tool_input": { "command": "create", "path": "/p/new.md", "file_text": "one\ntwo" }
        }));
        assert_eq!((k["added"].as_u64(), k["removed"].as_u64()), (Some(2), Some(0)));
        // Reads and shell commands are not edits.
        assert!(diff_of("kiro", "PostToolUse", json!({ "tool_name": "fs_read", "tool_input": {} })).is_null());
        assert!(diff_of("", "PostToolUse", json!({ "tool_name": "Bash", "tool_input": {} })).is_null());
    }

    #[test]
    fn huge_edits_are_counted_and_capped() {
        let old: String = (0..1000).map(|i| format!("a{i}\n")).collect();
        let new: String = (0..1000).map(|i| format!("b{i}\n")).collect();
        let d = diff_of("", "PostToolUse", json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": "/p/big", "old_string": old, "new_string": new }
        }));
        assert_eq!((d["added"].as_u64(), d["removed"].as_u64()), (Some(1000), Some(1000)));
        assert_eq!(d["lines"].as_array().unwrap().len(), MAX_LINES);
    }
}
