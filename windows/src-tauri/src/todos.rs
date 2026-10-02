// The user's to-do list, in local_dir()/todos.json.
//
// The pages own the logic (quick add, sorting, filters); Rust validates the
// whole document and writes it atomically, then tells every window. Named
// "todos" because "tasks" already means agent sessions across the app.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const MAX_ITEMS: usize = 5000;
const MAX_LISTS: usize = 50;
const MAX_TITLE: usize = 500;
const MAX_NAME: usize = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoList {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub done: bool,
    pub done_at: Option<f64>,
    /// "YYYY-MM-DD", a local day.
    pub due: Option<String>,
    /// 0 none … 3 high.
    pub priority: u8,
    pub list_id: Option<String>,
    pub created_at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoDoc {
    pub version: u32,
    pub lists: Vec<TodoList>,
    pub items: Vec<TodoItem>,
}

impl Default for TodoDoc {
    fn default() -> Self {
        Self { version: 1, lists: Vec::new(), items: Vec::new() }
    }
}

/// Both windows save; one write at a time so they never share the temp file.
static WRITE: Mutex<()> = Mutex::new(());

fn path() -> PathBuf {
    crate::settings::local_dir().join("todos.json")
}

pub fn load() -> TodoDoc {
    load_from(&path())
}

pub fn save(doc: &TodoDoc) -> Result<(), String> {
    validate(doc)?;
    let dir = crate::settings::local_dir();
    crate::platform::ensure_private_dir(&dir).map_err(|e| e.to_string())?;
    save_to(&path(), doc)
}

/// A missing file is an empty list. An unreadable one is set aside, never
/// overwritten: the next save would otherwise replace the user's tasks with nothing.
fn load_from(path: &Path) -> TodoDoc {
    let Ok(bytes) = std::fs::read(path) else { return TodoDoc::default() };
    match serde_json::from_slice::<TodoDoc>(&bytes) {
        Ok(doc) => doc,
        Err(err) => {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let broken = path.with_extension(format!("json.broken-{secs}"));
            eprintln!("[coucou] todos.json unreadable ({err}), moved to {}", broken.display());
            let _ = std::fs::rename(path, &broken);
            TodoDoc::default()
        }
    }
}

/// Written beside the target and renamed over it: a crash or a full disk
/// leaves the previous file whole.
fn save_to(path: &Path, doc: &TodoDoc) -> Result<(), String> {
    let _guard = WRITE.lock().unwrap();
    let json = serde_json::to_vec_pretty(doc).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("json.coucou-{}", std::process::id()));
    let result = std::fs::write(&temp, json).and_then(|_| std::fs::rename(&temp, path));
    if let Err(err) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Could not save your tasks: {err}"));
    }
    Ok(())
}

fn validate(doc: &TodoDoc) -> Result<(), String> {
    if doc.items.len() > MAX_ITEMS {
        return Err(format!("Too many tasks (max {MAX_ITEMS})."));
    }
    if doc.lists.len() > MAX_LISTS {
        return Err(format!("Too many lists (max {MAX_LISTS})."));
    }
    if doc.lists.iter().any(|l| l.name.trim().is_empty() || l.name.chars().count() > MAX_NAME) {
        return Err(format!("A list name must be 1 to {MAX_NAME} characters."));
    }
    for item in &doc.items {
        if item.title.trim().is_empty() || item.title.chars().count() > MAX_TITLE {
            return Err(format!("A task must be 1 to {MAX_TITLE} characters."));
        }
        if item.priority > 3 {
            return Err("Priority goes from 0 to 3.".into());
        }
        if item.due.as_deref().is_some_and(|d| !is_day(d)) {
            return Err("A due date must look like 2026-10-05.".into());
        }
    }
    Ok(())
}

fn is_day(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str) -> TodoItem {
        TodoItem {
            id: "a".into(),
            title: title.into(),
            done: false,
            done_at: None,
            due: Some("2026-10-05".into()),
            priority: 2,
            list_id: None,
            created_at: 1.0,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coucou-todos-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saves_and_loads_the_same_document() {
        let dir = temp_dir("roundtrip");
        let file = dir.join("todos.json");
        let doc = TodoDoc {
            version: 1,
            lists: vec![TodoList { id: "w".into(), name: "Work".into(), color: "#3b82f6".into() }],
            items: vec![item("Send the report")],
        };
        save_to(&file, &doc).unwrap();
        assert_eq!(load_from(&file), doc);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file left behind");
        assert_eq!(load_from(&dir.join("missing.json")), TodoDoc::default());
    }

    #[test]
    fn an_unreadable_file_is_set_aside_not_lost() {
        let dir = temp_dir("broken");
        let file = dir.join("todos.json");
        std::fs::write(&file, b"{ not json").unwrap();
        assert_eq!(load_from(&file), TodoDoc::default());
        assert!(!file.exists());
        let kept: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read(kept[0].path()).unwrap(), b"{ not json");
    }

    #[test]
    fn refuses_documents_out_of_bounds() {
        let ok = TodoDoc { items: vec![item("ok")], ..TodoDoc::default() };
        assert!(validate(&ok).is_ok());

        let too_many = TodoDoc { items: vec![item("x"); MAX_ITEMS + 1], ..TodoDoc::default() };
        assert!(validate(&too_many).is_err());

        let long = TodoDoc { items: vec![item(&"x".repeat(MAX_TITLE + 1))], ..TodoDoc::default() };
        assert!(validate(&long).is_err());

        let mut bad_day = item("x");
        bad_day.due = Some("05/10/2026".into());
        assert!(validate(&TodoDoc { items: vec![bad_day], ..TodoDoc::default() }).is_err());

        let mut bad_priority = item("x");
        bad_priority.priority = 4;
        assert!(validate(&TodoDoc { items: vec![bad_priority], ..TodoDoc::default() }).is_err());
    }
}
