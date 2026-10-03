// Integration pollers — the Rust side of StripePoller / GithubPoller /
// VercelPoller / N8nPoller / ResendPoller / NotionPoller / CalcomPoller.
//
// Same endpoints, same first-run delays and intervals as the Swift pollers. Each
// one emits an `integration` event; the island owns the badge, the sound and the
// 60 s auto-clear, exactly as the Swift handlers do.
//
// Nothing is polled until its key exists in the Credential Manager, and no
// request goes anywhere the user has not configured.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::island::WINDOW_LABEL;
use crate::log;
use crate::secrets;

const TIMEOUT: Duration = Duration::from_secs(10);

/// What the island receives. `event` is only set when something actually changed,
/// which is what drives the pill badge and the sound.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationUpdate {
    pub id: &'static str,
    pub data: Value,
    pub error: Option<String>,
    pub event: Option<IntegrationEvent>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationEvent {
    pub success: bool,
    pub label: String,
    pub detail: Option<String>,
    /// Open the island on it rather than only badging the pill.
    pub alert: bool,
}

fn emit(app: &AppHandle, update: IntegrationUpdate) {
    let _ = app.emit_to(WINDOW_LABEL, "integration", update);
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// Set from the tray's Pause item. While it is on, nothing reaches the network:
/// pausing Coucou has to mean pausing Coucou, not just hiding the island.
pub static PAUSED: AtomicBool = AtomicBool::new(false);

pub fn set_paused(on: bool) {
    PAUSED.store(on, Ordering::Relaxed);
}

/// Spawns every poller with the macOS delays and intervals.
pub fn start(app: AppHandle) {
    spawn(app.clone(), "integration_n8n", 3, 15, poll_n8n);
    spawn(app.clone(), "integration_vercel", 5, 30, poll_vercel);
    spawn(app.clone(), "integration_stripe", 6, 30, poll_stripe);
    spawn(app.clone(), "integration_resend", 6, 60, poll_resend);
    spawn(app.clone(), "integration_github", 7, 120, poll_github);
    spawn(app.clone(), "integration_calcom", 8, 300, poll_calcom);
    spawn(app.clone(), "integration_notion", 9, 300, poll_notion);
    spawn(app, WEATHER_ID, 4, 1800, poll_weather);
}

/// True when the user has this integration switched on in settings.
fn enabled(app: &AppHandle, id: &str) -> bool {
    if id == WEATHER_ID {
        return !weather_city(app).is_empty();
    }
    app.try_state::<crate::Shared>()
        .map(|shared| {
            let settings = shared.settings.lock().unwrap();
            settings.active_integrations.iter().any(|x| x == id)
        })
        .unwrap_or(false)
}

fn weather_city(app: &AppHandle) -> String {
    app.try_state::<crate::Shared>()
        .map(|shared| shared.settings.lock().unwrap().weather_city.trim().to_string())
        .unwrap_or_default()
}

fn spawn<F, Fut>(app: AppHandle, id: &'static str, delay_secs: u64, every_secs: u64, poll: F)
where
    F: Fn(AppHandle) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
        let mut ticker = tokio::time::interval(Duration::from_secs(every_secs));
        loop {
            ticker.tick().await;
            // The ticker keeps its cadence; we just decline to do the work. An
            // integration the user switched off, or a paused app, must make no
            // network calls at all — CLAUDE.md allows talking only to services
            // the user configured, and a disabled one is not configured.
            if PAUSED.load(Ordering::Relaxed) || !enabled(&app, id) {
                continue;
            }
            poll(app.clone()).await;
        }
    });
}

/// One-shot refresh from the Refresh buttons in the island.
pub async fn poll_once(app: AppHandle, id: &str) {
    match id {
        "integration_stripe" => poll_stripe(app).await,
        "integration_github" => poll_github(app).await,
        "integration_vercel" => poll_vercel(app).await,
        "integration_n8n" => poll_n8n(app).await,
        "integration_resend" => poll_resend(app).await,
        "integration_notion" => poll_notion(app).await,
        "integration_calcom" => poll_calcom(app).await,
        WEATHER_ID => poll_weather(app).await,
        _ => {}
    }
}

/// Remembers the newest id per integration so an event fires once, not on every poll.
struct Seen(Mutex<std::collections::HashMap<&'static str, String>>);

static SEEN: std::sync::LazyLock<Seen> =
    std::sync::LazyLock::new(|| Seen(Mutex::new(std::collections::HashMap::new())));

/// Returns true the first time a given id is seen (and false on the very first
/// load, which only fills the card).
fn is_new(key: &'static str, id: &str) -> bool {
    let mut map = SEEN.0.lock().unwrap();
    match map.insert(key, id.to_string()) {
        Some(previous) => previous != id,
        None => false, // first poll: populate silently, like the Swift pollers
    }
}

fn status_error(code: u16, unauthorised_hint: &str) -> String {
    match code {
        401 => "Invalid API key (401)".into(),
        403 => unauthorised_hint.into(),
        _ => format!("API error {code}"),
    }
}

// ── Stripe ────────────────────────────────────────────────────────────────────

async fn poll_stripe(app: AppHandle) {
    let Some(key) = secrets::get("stripe-api-key") else { return };
    let auth = format!("Basic {}", crate::claude::base64_for(format!("{key}:").as_bytes()));
    let http = client();

    let balance = http
        .get("https://api.stripe.com/v1/balance")
        .header("Authorization", &auth)
        .send()
        .await;

    let (amount, currency) = match balance {
        Ok(r) if r.status().is_success() => {
            let json: Value = r.json().await.unwrap_or(json!({}));
            let mut buckets: Vec<Value> = Vec::new();
            for k in ["available", "pending"] {
                if let Some(arr) = json.get(k).and_then(Value::as_array) {
                    buckets.extend(arr.iter().cloned());
                }
            }
            let currency = buckets
                .first()
                .and_then(|b| b.get("currency"))
                .and_then(Value::as_str)
                .unwrap_or("eur")
                .to_string();
            let amount: i64 = buckets
                .iter()
                .filter_map(|b| b.get("amount").and_then(Value::as_i64))
                .sum();
            (amount, currency)
        }
        Ok(r) => {
            let code = r.status().as_u16();
            emit(&app, IntegrationUpdate {
                id: "integration_stripe",
                data: json!({}),
                error: Some(status_error(code, "Use a secret key (sk_live_… not pk_live_…)")),
                event: None,
            });
            return;
        }
        Err(e) => {
            emit(&app, IntegrationUpdate {
                id: "integration_stripe",
                data: json!({}),
                error: Some(format!("No connection: {e}")),
                event: None,
            });
            return;
        }
    };

    let charges = http
        .get("https://api.stripe.com/v1/charges?limit=3")
        .header("Authorization", &auth)
        .send()
        .await;
    let Ok(response) = charges else { return };
    if !response.status().is_success() {
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let payments: Vec<Value> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|c| {
                    let description = c
                        .get("description")
                        .and_then(Value::as_str)
                        .or_else(|| {
                            c.get("billing_details")
                                .and_then(|b| b.get("name"))
                                .and_then(Value::as_str)
                        })
                        .map(str::to_string);
                    Some(json!({
                        "id": c.get("id")?.as_str()?,
                        "amount": c.get("amount")?.as_i64()?,
                        "currency": c.get("currency")?.as_str()?,
                        "description": description,
                        "createdAt": c.get("created").and_then(Value::as_i64).unwrap_or(0) * 1000,
                        "status": c.get("status").and_then(Value::as_str).unwrap_or("succeeded"),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();

    let newest = payments
        .first()
        .and_then(|p| p.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let event = if !newest.is_empty() && is_new("stripe", &newest) {
        let label = payments[0]
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                let cents = payments[0].get("amount").and_then(Value::as_i64).unwrap_or(0);
                format!("{:.2}", cents as f64 / 100.0)
            });
        Some(IntegrationEvent { success: true, label, detail: None, alert: false })
    } else {
        None
    };

    emit(&app, IntegrationUpdate {
        id: "integration_stripe",
        data: json!({ "balance": amount, "currency": currency, "payments": payments }),
        error: None,
        event,
    });
}

// ── GitHub ────────────────────────────────────────────────────────────────────

/// How many recently pushed repositories get their latest Actions run checked.
const GITHUB_CI_REPOS: usize = 6;

async fn github_get(http: &reqwest::Client, token: &str, url: &str) -> Option<Value> {
    let r = http
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Coucou")
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.json().await.ok()
}

/// Search results as the card's rows: newest first.
fn github_prs(search: Option<Value>) -> (Vec<Value>, i64) {
    let Some(v) = search else { return (Vec::new(), 0) };
    let total = v.get("total_count").and_then(Value::as_i64).unwrap_or(0);
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|i| {
                    let repo = i.get("repository_url")?.as_str()?.rsplit('/').next()?.to_string();
                    Some(json!({
                        "id": i.get("id")?.as_i64()?.to_string(),
                        "title": i.get("title")?.as_str()?,
                        "repo": repo,
                        "number": i.get("number")?.as_i64()?,
                        "url": i.get("html_url")?.as_str()?,
                        "createdAt": i.get("created_at")?.as_str()?,
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    (items, total)
}

/// Open PRs waiting for the user's review, open PRs in their organisations,
/// and the latest Actions run of their most recently pushed repositories.
/// A failed run or a new review request opens the island; a new PR only badges.
async fn poll_github_activity(http: &reqwest::Client, token: &str, login: &str) -> (Value, Option<IntegrationEvent>) {
    let api = "https://api.github.com";
    let orgs: Vec<String> = github_get(http, token, &format!("{api}/user/orgs?per_page=50"))
        .await
        .and_then(|v| v.as_array().cloned())
        .map(|l| l.iter().filter_map(|o| o.get("login")?.as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    let search = |q: String| format!("{api}/search/issues?q={}&sort=created&order=desc&per_page=5", q.replace(' ', "+"));
    let (reviews, _) = github_prs(
        github_get(http, token, &search("is:pr is:open archived:false review-requested:@me".into())).await,
    );
    let scope = if orgs.is_empty() {
        format!("user:{login}")
    } else {
        orgs.iter().map(|o| format!("org:{o}")).collect::<Vec<_>>().join(" ")
    };
    let (prs, open_prs) =
        github_prs(github_get(http, token, &search(format!("is:pr is:open archived:false {scope}"))).await);

    let mut failures: Vec<Value> = Vec::new();
    let repos = github_get(
        http,
        token,
        &format!("{api}/user/repos?sort=pushed&per_page={GITHUB_CI_REPOS}&affiliation=owner,organization_member"),
    )
    .await
    .and_then(|v| v.as_array().cloned())
    .unwrap_or_default();
    for repo in repos {
        let Some(full) = repo.get("full_name").and_then(Value::as_str) else { continue };
        let runs = github_get(http, token, &format!("{api}/repos/{full}/actions/runs?per_page=1")).await;
        let Some(run) = runs.as_ref().and_then(|v| v.get("workflow_runs")).and_then(|v| v.get(0)) else {
            continue;
        };
        if run.get("conclusion").and_then(Value::as_str) == Some("failure") {
            failures.push(json!({
                "id": run.get("id").and_then(Value::as_i64).unwrap_or(0).to_string(),
                "repo": repo.get("name").and_then(Value::as_str).unwrap_or(full),
                "workflow": run.get("name").and_then(Value::as_str).unwrap_or("CI"),
                "branch": run.get("head_branch").and_then(Value::as_str).unwrap_or(""),
                "url": run.get("html_url").and_then(Value::as_str).unwrap_or(""),
                "createdAt": run.get("updated_at").and_then(Value::as_str).unwrap_or(""),
            }));
        }
    }

    // Every key is checked on every poll so the first one only fills the card.
    let newest = |list: &[Value]| {
        list.iter()
            .filter_map(|v| v.get("id")?.as_str()?.parse::<i64>().ok())
            .max()
            .map(|n| n.to_string())
            .unwrap_or_default()
    };
    let (run_id, review_id, pr_id) = (newest(&failures), newest(&reviews), newest(&prs));
    let new_run = !run_id.is_empty() && is_new("github_run", &run_id);
    let new_review = !review_id.is_empty() && is_new("github_review", &review_id);
    let new_pr = !pr_id.is_empty() && is_new("github_pr", &pr_id);

    let find = |list: &[Value], id: &str| list.iter().find(|v| v["id"] == id).cloned().unwrap_or(json!({}));
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let event = if new_run {
        let r = find(&failures, &run_id);
        Some(IntegrationEvent {
            success: false,
            label: format!("CI failed · {}", s(&r, "repo")),
            detail: Some(format!("{} · {}", s(&r, "workflow"), s(&r, "branch"))),
            alert: true,
        })
    } else if new_review {
        let p = find(&reviews, &review_id);
        Some(IntegrationEvent {
            success: true,
            label: format!("Review requested · {}", s(&p, "repo")),
            detail: Some(s(&p, "title")),
            alert: true,
        })
    } else if new_pr {
        let p = find(&prs, &pr_id);
        Some(IntegrationEvent {
            success: true,
            label: format!("New PR · {}", s(&p, "repo")),
            detail: Some(s(&p, "title")),
            alert: false,
        })
    } else {
        None
    };

    (json!({ "reviews": reviews, "prs": prs, "openPrs": open_prs, "failures": failures }), event)
}

async fn poll_github(app: AppHandle) {
    let Some(token) = secrets::get("github-token") else { return };
    let http = client();

    let user = http
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Coucou")
        .send()
        .await;
    let Ok(response) = user else { return };
    if !response.status().is_success() {
        emit(&app, IntegrationUpdate {
            id: "integration_github",
            data: json!({}),
            error: Some(status_error(response.status().as_u16(), "Token lacks the needed scope")),
            event: None,
        });
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let public = json.get("public_repos").and_then(Value::as_i64).unwrap_or(0);
    let private = json
        .get("owned_private_repos")
        .or_else(|| json.get("total_private_repos"))
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let repos = http
        .get("https://api.github.com/user/repos?per_page=100&affiliation=owner&sort=pushed")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Coucou")
        .send()
        .await;
    let stars: i64 = match repos {
        Ok(r) if r.status().is_success() => r
            .json::<Value>()
            .await
            .ok()
            .and_then(|v| v.as_array().cloned())
            .map(|list| {
                list.iter()
                    .filter_map(|r| r.get("stargazers_count").and_then(Value::as_i64))
                    .sum()
            })
            .unwrap_or(0),
        _ => 0,
    };

    let login = json.get("login").and_then(Value::as_str).unwrap_or("").to_string();
    let (mut data, event) = poll_github_activity(&http, &token, &login).await;
    data["totalRepos"] = json!(public + private);
    data["totalStars"] = json!(stars);
    emit(&app, IntegrationUpdate { id: "integration_github", data, error: None, event });
}

// ── Vercel ────────────────────────────────────────────────────────────────────

async fn poll_vercel(app: AppHandle) {
    let Some(token) = secrets::get("vercel-token") else { return };
    let response = client()
        .get("https://api.vercel.com/v6/deployments?limit=5")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await;
    let Ok(response) = response else { return };
    if !response.status().is_success() {
        emit(&app, IntegrationUpdate {
            id: "integration_vercel",
            data: json!({}),
            error: Some(status_error(response.status().as_u16(), "Token lacks access")),
            event: None,
        });
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let terminal = ["READY", "ERROR", "CANCELED"];
    let deployments: Vec<Value> = json
        .get("deployments")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|d| {
                    let state = d.get("state")?.as_str()?;
                    if !terminal.contains(&state) {
                        return None;
                    }
                    let meta = d.get("meta");
                    let pick = |keys: [&str; 3]| {
                        meta.and_then(|m| keys.iter().find_map(|k| m.get(*k).and_then(Value::as_str)))
                            .map(str::to_string)
                    };
                    Some(json!({
                        "id": d.get("uid")?.as_str()?,
                        "projectName": d.get("name")?.as_str()?,
                        "url": d.get("url").and_then(Value::as_str).unwrap_or(""),
                        "state": state,
                        "createdAt": d.get("createdAt").and_then(Value::as_f64).unwrap_or(0.0),
                        "commitMessage": pick(["githubCommitMessage", "gitlabCommitMessage", "bitbucketCommitMessage"]),
                        "branch": pick(["githubCommitRef", "gitlabCommitRef", "bitbucketBranch"]),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();

    let event = deployments.first().and_then(|latest| {
        let id = latest.get("id")?.as_str()?;
        if !is_new("vercel", id) {
            return None;
        }
        let success = latest.get("state")?.as_str()? == "READY";
        Some(IntegrationEvent {
            success,
            label: latest.get("projectName")?.as_str()?.to_string(),
            detail: None,
            alert: false,
        })
    });

    emit(&app, IntegrationUpdate {
        id: "integration_vercel",
        data: json!({ "deployments": deployments }),
        error: None,
        event,
    });
}

// ── Resend ────────────────────────────────────────────────────────────────────

async fn poll_resend(app: AppHandle) {
    let Some(key) = secrets::get("resend-api-key") else { return };
    let response = client()
        .get("https://api.resend.com/emails?limit=100")
        .header("Authorization", format!("Bearer {key}"))
        .header("Accept", "application/json")
        .send()
        .await;
    let Ok(response) = response else { return };
    if !response.status().is_success() {
        emit(&app, IntegrationUpdate {
            id: "integration_resend",
            data: json!({}),
            error: Some(status_error(response.status().as_u16(), "Key lacks access")),
            event: None,
        });
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let total = json
        .get("total")
        .or_else(|| json.get("count"))
        .and_then(Value::as_i64);
    let emails: Vec<Value> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(5)
                .filter_map(|e| {
                    let to = match e.get("to") {
                        Some(Value::Array(a)) => a.clone(),
                        Some(Value::String(s)) => vec![Value::String(s.clone())],
                        _ => vec![],
                    };
                    Some(json!({
                        "id": e.get("id")?.as_str()?,
                        "to": to,
                        "subject": e.get("subject").and_then(Value::as_str).unwrap_or(""),
                        "createdAt": e.get("created_at").and_then(Value::as_str).unwrap_or(""),
                        "lastEvent": e.get("last_event").and_then(Value::as_str).unwrap_or(""),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();

    emit(&app, IntegrationUpdate {
        id: "integration_resend",
        data: json!({ "emails": emails, "total": total }),
        error: None,
        event: None,
    });
}

// ── Notion ────────────────────────────────────────────────────────────────────

async fn poll_notion(app: AppHandle) {
    let Some(token) = secrets::get("notion-api-key") else { return };
    let response = client()
        .post("https://api.notion.com/v1/search")
        .header("Authorization", format!("Bearer {token}"))
        .header("Notion-Version", "2022-06-28")
        .header("Content-Type", "application/json")
        .json(&json!({
            "sort": { "direction": "descending", "timestamp": "last_edited_time" },
            "page_size": 3
        }))
        .send()
        .await;
    let Ok(response) = response else { return };
    if !response.status().is_success() {
        emit(&app, IntegrationUpdate {
            id: "integration_notion",
            data: json!({}),
            error: Some(status_error(response.status().as_u16(), "Integration lacks access")),
            event: None,
        });
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let pages: Vec<Value> = json
        .get("results")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(parse_notion_page).collect())
        .unwrap_or_default();

    emit(&app, IntegrationUpdate {
        id: "integration_notion",
        data: json!({ "pages": pages }),
        error: None,
        event: None,
    });
}

fn parse_notion_page(obj: &Value) -> Option<Value> {
    let id = obj.get("id")?.as_str()?;
    let is_database = obj.get("object").and_then(Value::as_str) == Some("database");

    let mut title = "Untitled".to_string();
    if is_database {
        if let Some(text) = obj
            .get("title")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|t| t.get("plain_text"))
            .and_then(Value::as_str)
        {
            if !text.is_empty() {
                title = text.to_string();
            }
        }
    } else if let Some(props) = obj.get("properties").and_then(Value::as_object) {
        for prop in props.values() {
            if prop.get("type").and_then(Value::as_str) != Some("title") {
                continue;
            }
            if let Some(text) = prop
                .get("title")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|t| t.get("plain_text"))
                .and_then(Value::as_str)
            {
                if !text.is_empty() {
                    title = text.to_string();
                    break;
                }
            }
        }
    }

    let emoji = obj
        .get("icon")
        .filter(|i| i.get("type").and_then(Value::as_str) == Some("emoji"))
        .and_then(|i| i.get("emoji"))
        .and_then(Value::as_str);

    Some(json!({
        "id": id,
        "title": title,
        "emoji": emoji,
        "lastEditedAt": obj.get("last_edited_time").and_then(Value::as_str)?,
        "url": obj.get("url").and_then(Value::as_str).unwrap_or("https://notion.so"),
    }))
}

// ── Cal.com ───────────────────────────────────────────────────────────────────

async fn poll_calcom(app: AppHandle) {
    let Some(key) = secrets::get("calcom-api-key") else { return };
    let response = client()
        .get("https://api.cal.com/v2/bookings?status=upcoming")
        .header("Authorization", format!("Bearer {key}"))
        .header("cal-api-version", "2024-08-13")
        .send()
        .await;
    let Ok(response) = response else { return };
    if !response.status().is_success() {
        emit(&app, IntegrationUpdate {
            id: "integration_calcom",
            data: json!({}),
            error: Some(status_error(response.status().as_u16(), "Key lacks access")),
            event: None,
        });
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let bookings: Vec<Value> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|b| {
                    let start = b
                        .get("start")
                        .or_else(|| b.get("startTime"))
                        .and_then(Value::as_str)?;
                    let attendee = b.get("attendees").and_then(Value::as_array).and_then(|a| a.first());
                    let notes = b
                        .get("responses")
                        .and_then(|r| r.get("notes"))
                        .and_then(|n| n.get("value"))
                        .and_then(Value::as_str)
                        .or_else(|| b.get("description").and_then(Value::as_str))
                        .filter(|s| !s.is_empty());
                    Some(json!({
                        "id": b.get("id").map(|v| v.to_string()).unwrap_or_default(),
                        "title": b.get("title").and_then(Value::as_str).unwrap_or("Meeting"),
                        "start": start,
                        "status": b.get("status").and_then(Value::as_str).unwrap_or("accepted"),
                        "attendeeName": attendee.and_then(|a| a.get("name")).and_then(Value::as_str),
                        "attendeeEmail": attendee.and_then(|a| a.get("email")).and_then(Value::as_str),
                        "attendeeNotes": notes,
                    }))
                })
                .collect()
        })
        .unwrap_or_default();

    emit(&app, IntegrationUpdate {
        id: "integration_calcom",
        data: json!({ "bookings": bookings }),
        error: None,
        event: None,
    });
}

// ── Weather (Open-Meteo, no key) ──────────────────────────────────────────────

const WEATHER_ID: &str = "weather";

/// The city last geocoded: (what the user typed, display name, lat, lon).
static GEO: Mutex<Option<(String, String, f64, f64)>> = Mutex::new(None);

async fn poll_weather(app: AppHandle) {
    let city = weather_city(&app);
    if city.is_empty() || PAUSED.load(Ordering::Relaxed) {
        return;
    }
    let http = client();
    let cached = GEO.lock().unwrap().clone().filter(|g| g.0 == city);
    let (name, lat, lon) = match cached {
        Some((_, name, lat, lon)) => (name, lat, lon),
        None => {
            let mut place = None;
            for (query, qualifier) in geo_queries(&city) {
                let found = http
                    .get("https://geocoding-api.open-meteo.com/v1/search")
                    .query(&[("name", query.as_str()), ("count", "10"), ("format", "json")])
                    .send()
                    .await;
                let Ok(found) = found else { return };
                let json: Value = found.json().await.unwrap_or(json!({}));
                place = pick_place(&json, qualifier.as_deref());
                if place.is_some() {
                    break;
                }
            }
            let Some((name, lat, lon)) = place else {
                emit(&app, IntegrationUpdate {
                    id: WEATHER_ID,
                    data: json!({}),
                    error: Some(format!("City not found: {city}")),
                    event: None,
                });
                return;
            };
            *GEO.lock().unwrap() = Some((city.clone(), name.clone(), lat, lon));
            (name, lat, lon)
        }
    };

    let response = http
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&[
            ("latitude", lat.to_string()),
            ("longitude", lon.to_string()),
            ("current", "temperature_2m,weather_code,is_day".into()),
            ("daily", "temperature_2m_max,temperature_2m_min,precipitation_probability_max".into()),
            ("timezone", "auto".into()),
            ("forecast_days", "1".into()),
        ])
        .send()
        .await;
    let Ok(response) = response else { return };
    if !response.status().is_success() {
        emit(&app, IntegrationUpdate {
            id: WEATHER_ID,
            data: json!({}),
            error: Some(status_error(response.status().as_u16(), "Weather unavailable")),
            event: None,
        });
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    emit(&app, IntegrationUpdate { id: WEATHER_ID, data: weather_data(&name, &json), error: None, event: None });
}

/// Open-Meteo only matches English country names, and a state only after a
/// comma: "São Paulo, Brasil" and "Florianópolis SC" find nothing. So: the text
/// as typed, then the city alone with the rest kept to pick among the results.
fn geo_queries(city: &str) -> Vec<(String, Option<String>)> {
    let mut out = vec![(city.to_string(), None)];
    if let Some((name, rest)) = city.split_once(',') {
        out.push((name.trim().to_string(), Some(rest.trim().to_string())));
    } else if let Some((name, last)) = city.rsplit_once(' ') {
        out.push((name.trim().to_string(), Some(last.trim().to_string())));
    }
    out
}

/// The first result, or the first whose state or country matches `qualifier`
/// ("SC" = Santa Catarina by initials, "Brasil", "BR", "Santa Catarina").
fn pick_place(json: &Value, qualifier: Option<&str>) -> Option<(String, f64, f64)> {
    let results = json.get("results")?.as_array()?;
    let matches = |place: &Value| {
        let Some(q) = qualifier.map(str::to_lowercase).filter(|q| !q.is_empty()) else { return false };
        let field = |k: &str| place.get(k).and_then(Value::as_str).unwrap_or("").to_lowercase();
        let admin1 = field("admin1");
        let initials: String = admin1.split_whitespace().filter_map(|w| w.chars().next()).collect();
        [admin1.clone(), initials, field("country"), field("country_code")].contains(&q)
    };
    let place = results.iter().find(|p| matches(p)).or_else(|| results.first())?;
    Some((
        place.get("name")?.as_str()?.to_string(),
        place.get("latitude")?.as_f64()?,
        place.get("longitude")?.as_f64()?,
    ))
}

fn weather_data(city: &str, json: &Value) -> Value {
    let current = json.get("current").cloned().unwrap_or(json!({}));
    let daily = |key: &str| json.get("daily").and_then(|d| d.get(key)).and_then(|v| v.get(0)).cloned();
    json!({
        "city": city,
        "temp": current.get("temperature_2m"),
        "code": current.get("weather_code"),
        "isDay": current.get("is_day").and_then(Value::as_i64).map(|d| d == 1),
        "max": daily("temperature_2m_max"),
        "min": daily("temperature_2m_min"),
        "rain": daily("precipitation_probability_max"),
    })
}

// ── n8n ───────────────────────────────────────────────────────────────────────

async fn poll_n8n(app: AppHandle) {
    let (Some(key), Some(raw_base)) = (secrets::get("n8n-api-key"), secrets::get("n8n-url")) else {
        return;
    };
    let base = raw_base.trim_end_matches('/').to_string();
    let http = client();

    // Same two shapes as the Swift poller: the public API first, then /rest.
    let list_urls = [
        format!("{base}/api/v1/executions?limit=1&includeData=false"),
        format!("{base}/rest/executions?limit=1&includeData=false"),
    ];

    let mut items: Option<Vec<Value>> = None;
    for url in &list_urls {
        let Ok(response) = http.get(url).header("X-N8N-API-KEY", &key).header("Accept", "application/json").send().await
        else {
            continue;
        };
        if !response.status().is_success() {
            // Only the status: a self-hosted base URL can carry credentials.
            log::line(format!("n8n list HTTP {}", response.status()));
            continue;
        }
        let Ok(json) = response.json::<Value>().await else { continue };
        items = match &json {
            Value::Object(o) => o.get("data").and_then(Value::as_array).cloned(),
            Value::Array(a) => Some(a.clone()),
            _ => None,
        };
        if items.is_some() {
            break;
        }
    }

    let Some(first) = items.and_then(|list| list.into_iter().next()) else { return };
    let id = match first.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return,
    };

    let status = first.get("status").and_then(Value::as_str).unwrap_or("");
    if !["success", "error", "crashed", "canceled", "failed"].contains(&status) {
        return;
    }
    if !is_new("n8n", &id) {
        return;
    }
    let success = status == "success";

    let detail_urls = [
        format!("{base}/api/v1/executions/{id}?includeData=true"),
        format!("{base}/api/v1/executions/{id}"),
        format!("{base}/rest/executions/{id}?includeData=true"),
        format!("{base}/rest/executions/{id}"),
    ];
    let mut name = "Workflow".to_string();
    let mut detail = None;
    for url in &detail_urls {
        let Ok(response) = http.get(url).header("X-N8N-API-KEY", &key).header("Accept", "application/json").send().await
        else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(json) = response.json::<Value>().await else { continue };
        name = json
            .get("workflowData")
            .and_then(|w| w.get("name"))
            .and_then(Value::as_str)
            .or_else(|| json.get("name").and_then(Value::as_str))
            .unwrap_or("Workflow")
            .to_string();
        detail = n8n_detail(&json, success);
        break;
    }

    log::line(format!("n8n execution {id} {status} · {name}"));
    emit(&app, IntegrationUpdate {
        id: "integration_n8n",
        data: json!({ "workflow": name, "status": status }),
        error: None,
        event: Some(IntegrationEvent { success, label: name, detail, alert: false }),
    });
}

fn n8n_detail(json: &Value, success: bool) -> Option<String> {
    let result = json.get("data")?.get("resultData")?;
    if !success {
        if let Some(error) = result.get("error") {
            let message = error.get("message").and_then(Value::as_str).unwrap_or("");
            if let Some(node) = error.get("node").and_then(|n| n.get("name")).and_then(Value::as_str) {
                if !node.is_empty() {
                    return Some(format!("{node}\n{message}"));
                }
            }
            return Some(message.to_string());
        }
        let runs = result.get("runData")?.as_object()?;
        for (node, value) in runs {
            if let Some(message) = value
                .as_array()
                .and_then(|a| a.first())
                .and_then(|r| r.get("error"))
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
            {
                return Some(format!("{node}\n{message}"));
            }
        }
        return None;
    }

    let last_node = result.get("lastNodeExecuted")?.as_str()?;
    let items = result
        .get("runData")?
        .get(last_node)?
        .as_array()?
        .first()?
        .get("data")?
        .get("main")?
        .as_array()?
        .first()?
        .as_array()?;
    let count = items.len();
    let header = format!("→ {last_node} · {count} item{}", if count == 1 { "" } else { "s" });

    let fields = items
        .first()
        .and_then(|i| i.get("json"))
        .and_then(Value::as_object)
        .map(|obj| {
            obj.iter()
                .take(4)
                .map(|(k, v)| format!("{k}: {}", fmt_value(v)))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.is_empty());

    Some(match fields {
        Some(f) => format!("{header}\n{f}"),
        None => header,
    })
}

fn fmt_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.chars().take(50).collect(),
        Value::Array(a) => format!("[{}]", a.len()),
        Value::Object(_) => "{…}".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_reads_open_meteo_shapes() {
        let geo = json!({ "results": [
            { "name": "Florianópolis", "admin1": "Acre", "country_code": "BR", "latitude": -9.0, "longitude": -70.0 },
            { "name": "Florianópolis", "admin1": "Santa Catarina", "country_code": "BR", "latitude": -27.6, "longitude": -48.5 },
        ] });
        assert_eq!(pick_place(&geo, Some("SC")).unwrap().1, -27.6);
        assert_eq!(pick_place(&geo, Some("santa catarina")).unwrap().1, -27.6);
        assert_eq!(pick_place(&geo, None).unwrap().1, -9.0);
        assert_eq!(pick_place(&geo, Some("Brasil")).unwrap().1, -9.0);
        assert_eq!(pick_place(&json!({}), None), None);

        assert_eq!(geo_queries("Florianópolis SC")[1], ("Florianópolis".into(), Some("SC".into())));
        assert_eq!(geo_queries("São Paulo, Brasil")[1], ("São Paulo".into(), Some("Brasil".into())));
        assert_eq!(geo_queries("Lisboa").len(), 1);

        let forecast = json!({
            "current": { "temperature_2m": 24.3, "weather_code": 2, "is_day": 1 },
            "daily": {
                "temperature_2m_max": [28.1], "temperature_2m_min": [18.0],
                "precipitation_probability_max": [30]
            }
        });
        let data = weather_data("São Paulo", &forecast);
        assert_eq!(data["temp"], json!(24.3));
        assert_eq!(data["code"], json!(2));
        assert_eq!(data["isDay"], json!(true));
        assert_eq!(data["max"], json!(28.1));
        assert_eq!(data["rain"], json!(30));
    }
}
