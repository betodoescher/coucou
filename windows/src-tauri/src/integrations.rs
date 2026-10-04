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

/// What changes minute to minute, in one round trip: the user's open PRs with
/// their head commit's CI, the default branch CI of their most recently pushed
/// repositories, and the PRs waiting for their review.
const GITHUB_PULSE_QUERY: &str = r#"query {
  viewer {
    pullRequests(states: OPEN, first: 20, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number title url isDraft reviewDecision updatedAt
        repository { name nameWithOwner }
        commits(last: 1) { nodes { commit { oid statusCheckRollup { state } } } }
      }
    }
    repositories(first: 10, ownerAffiliations: [OWNER], orderBy: {field: PUSHED_AT, direction: DESC}) {
      nodes {
        name nameWithOwner url isArchived
        defaultBranchRef { name target { ... on Commit { oid statusCheckRollup { state } } } }
      }
    }
  }
  reviewRequested: search(query: "is:pr is:open review-requested:@me archived:false", type: ISSUE, first: 20) {
    nodes { ... on PullRequest { number title url isDraft createdAt repository { name nameWithOwner } } }
  }
}"#;

const GITHUB_ACTIVITY_QUERY: &str = r#"query {
  viewer { contributionsCollection { contributionCalendar {
    totalContributions
    weeks { contributionDays { date contributionCount contributionLevel weekday } }
  } } }
}"#;

/// The contribution grid only moves a few times a day.
const GITHUB_ACTIVITY_EVERY: Duration = Duration::from_secs(1800);
const GITHUB_ACTIVITY_WEEKS: usize = 23;

/// The previous pulse, to tell a CI that just turned red or green from one that
/// has been that colour for a while.
static GITHUB_PULSE: Mutex<Option<Value>> = Mutex::new(None);
static GITHUB_ACTIVITY: Mutex<Option<(std::time::Instant, Value)>> = Mutex::new(None);

async fn github_graphql(http: &reqwest::Client, token: &str, query: &str) -> Option<Value> {
    let r = http
        .post("https://api.github.com/graphql")
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "Coucou")
        .json(&json!({ "query": query }))
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v: Value = r.json().await.ok()?;
    v.get("data").filter(|d| d.is_object()).cloned()
}

fn ci_state(rollup: &Value) -> &'static str {
    match rollup.get("state").and_then(Value::as_str) {
        Some("PENDING" | "EXPECTED") => "pending",
        Some("SUCCESS") => "success",
        Some("ERROR" | "FAILURE") => "failure",
        _ => "unknown",
    }
}

fn review_state(decision: &Value) -> &'static str {
    match decision.as_str() {
        Some("APPROVED") => "approved",
        Some("CHANGES_REQUESTED") => "changes",
        Some("REVIEW_REQUIRED") => "pending",
        _ => "",
    }
}

/// The pulse as the card's rows. A missing piece is an empty list, never a failed poll.
fn github_pulse(data: &Value) -> Value {
    let nodes = |v: &Value| v.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    let viewer = &data["viewer"];

    let mine: Vec<Value> = nodes(&viewer["pullRequests"])
        .iter()
        .filter_map(|p| {
            let full = p["repository"]["nameWithOwner"].as_str()?;
            let number = p["number"].as_i64()?;
            let commit = &p["commits"]["nodes"][0]["commit"];
            Some(json!({
                "id": format!("{full}#{number}"),
                "title": p["title"].as_str()?,
                "repo": p["repository"]["name"].as_str().unwrap_or(full),
                "number": number,
                "url": p["url"].as_str()?,
                "draft": p["isDraft"].as_bool().unwrap_or(false),
                "review": review_state(&p["reviewDecision"]),
                "ci": ci_state(&commit["statusCheckRollup"]),
                "sha": commit["oid"].as_str().unwrap_or(""),
                "createdAt": p["updatedAt"].as_str().unwrap_or(""),
            }))
        })
        .collect();

    let main_ci: Vec<Value> = nodes(&viewer["repositories"])
        .iter()
        .filter(|r| !r["isArchived"].as_bool().unwrap_or(false))
        .filter_map(|r| {
            let branch = r["defaultBranchRef"].as_object()?;
            let target = &branch.get("target")?;
            Some(json!({
                "id": r["nameWithOwner"].as_str()?,
                "repo": r["name"].as_str()?,
                "url": r["url"].as_str()?,
                "branch": branch.get("name").and_then(Value::as_str).unwrap_or("main"),
                "ci": ci_state(&target["statusCheckRollup"]),
                "sha": target["oid"].as_str().unwrap_or(""),
            }))
        })
        .collect();

    let reviews: Vec<Value> = nodes(&data["reviewRequested"])
        .iter()
        .filter_map(|p| {
            let full = p["repository"]["nameWithOwner"].as_str()?;
            let number = p["number"].as_i64()?;
            Some(json!({
                "id": format!("{full}#{number}"),
                "title": p["title"].as_str()?,
                "repo": p["repository"]["name"].as_str().unwrap_or(full),
                "number": number,
                "url": p["url"].as_str()?,
                "draft": p["isDraft"].as_bool().unwrap_or(false),
                "createdAt": p["createdAt"].as_str().unwrap_or(""),
            }))
        })
        .collect();

    json!({ "mine": mine, "mainCi": main_ci, "reviews": reviews })
}

/// What changed between two pulses, most urgent first: a CI that just failed
/// (on a PR or a default branch), a new review request, then a PR whose CI just
/// went green. Nothing on the first pulse, which only fills the card.
fn github_pulse_event(old: Option<&Value>, new: &Value) -> Option<IntegrationEvent> {
    let old = old?;
    let list = |v: &Value, k: &str| v[k].as_array().cloned().unwrap_or_default();
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    let find = |l: &[Value], id: &Value| l.iter().find(|v| &v["id"] == id).cloned();

    let (old_mine, old_main, old_reviews) = (list(old, "mine"), list(old, "mainCi"), list(old, "reviews"));
    let mut failed = None;
    let mut passed = None;
    for pr in list(new, "mine") {
        let ci = s(&pr, "ci");
        let before = find(&old_mine, &pr["id"]);
        let (fail, pass) = match before {
            Some(b) if b["sha"] == pr["sha"] => {
                let was = s(&b, "ci");
                (ci == "failure" && was != "failure", ci == "success" && was == "pending")
            }
            // A new commit or a PR we had not seen: only a settled CI says anything.
            _ => (ci == "failure", ci == "success"),
        };
        if fail && failed.is_none() {
            failed = Some(pr);
        } else if pass && passed.is_none() {
            passed = Some(pr);
        }
    }
    let main_failed = list(new, "mainCi").into_iter().find(|r| {
        r["ci"] == "failure"
            && find(&old_main, &r["id"]).is_none_or(|b| b["sha"] != r["sha"] || b["ci"] != "failure")
    });
    let review = list(new, "reviews").into_iter().find(|p| find(&old_reviews, &p["id"]).is_none());

    let pr_label = |p: &Value| format!("{}#{}", s(p, "repo"), p["number"]);
    if let Some(p) = failed {
        return Some(IntegrationEvent {
            success: false,
            label: format!("CI failed · {}", pr_label(&p)),
            detail: Some(s(&p, "title")),
            alert: true,
        });
    }
    if let Some(r) = main_failed {
        return Some(IntegrationEvent {
            success: false,
            label: format!("CI failed · {}", s(&r, "repo")),
            detail: Some(format!("{} branch", s(&r, "branch"))),
            alert: true,
        });
    }
    if let Some(p) = review {
        return Some(IntegrationEvent {
            success: true,
            label: format!("Review requested · {}", s(&p, "repo")),
            detail: Some(s(&p, "title")),
            alert: true,
        });
    }
    passed.map(|p| IntegrationEvent {
        success: true,
        label: format!("CI passed · {}", pr_label(&p)),
        detail: Some(s(&p, "title")),
        alert: false,
    })
}

fn contribution_level(level: &Value) -> i64 {
    match level.as_str() {
        Some("FIRST_QUARTILE") => 1,
        Some("SECOND_QUARTILE") => 2,
        Some("THIRD_QUARTILE") => 3,
        Some("FOURTH_QUARTILE") => 4,
        _ => 0,
    }
}

/// The year's total and the last weeks of the contribution calendar, oldest day first.
fn github_activity(data: &Value) -> Option<Value> {
    let calendar = &data["viewer"]["contributionsCollection"]["contributionCalendar"];
    let weeks = calendar["weeks"].as_array()?;
    let recent = &weeks[weeks.len().saturating_sub(GITHUB_ACTIVITY_WEEKS)..];
    let days: Vec<Value> = recent
        .iter()
        .flat_map(|w| w["contributionDays"].as_array().cloned().unwrap_or_default())
        .filter_map(|d| {
            Some(json!({
                "date": d["date"].as_str()?,
                "count": d["contributionCount"].as_i64().unwrap_or(0),
                "level": contribution_level(&d["contributionLevel"]),
                "weekday": d["weekday"].as_i64().unwrap_or(0),
            }))
        })
        .collect();
    Some(json!({ "total": calendar["totalContributions"].as_i64().unwrap_or(0), "days": days }))
}

/// The cached grid, refetched once it is older than half an hour.
async fn github_activity_cached(http: &reqwest::Client, token: &str) -> Value {
    let cached = GITHUB_ACTIVITY.lock().unwrap().clone();
    if let Some((at, ref v)) = cached {
        if at.elapsed() < GITHUB_ACTIVITY_EVERY {
            return v.clone();
        }
    }
    match github_graphql(http, token, GITHUB_ACTIVITY_QUERY).await.as_ref().and_then(github_activity) {
        Some(v) => {
            *GITHUB_ACTIVITY.lock().unwrap() = Some((std::time::Instant::now(), v.clone()));
            v
        }
        None => cached.map(|(_, v)| v).unwrap_or(Value::Null),
    }
}

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

/// The pulse (the user's PRs and their CI, default branch CI, review requests),
/// open PRs in their organisations and the contribution grid. A CI that turns
/// red or a new review request opens the island; a CI that turns green or a new
/// PR only badges.
async fn poll_github_activity(http: &reqwest::Client, token: &str, login: &str) -> (Value, Option<IntegrationEvent>) {
    let api = "https://api.github.com";
    let orgs: Vec<String> = github_get(http, token, &format!("{api}/user/orgs?per_page=50"))
        .await
        .and_then(|v| v.as_array().cloned())
        .map(|l| l.iter().filter_map(|o| o.get("login")?.as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    let pulse = github_graphql(http, token, GITHUB_PULSE_QUERY).await.map(|d| github_pulse(&d));
    let pulse_event = pulse.as_ref().and_then(|new| {
        let mut previous = GITHUB_PULSE.lock().unwrap();
        let event = github_pulse_event(previous.as_ref(), new);
        *previous = Some(new.clone());
        event
    });
    let pulse = pulse.unwrap_or_else(|| json!({ "mine": [], "mainCi": [], "reviews": [] }));

    let search = |q: String| format!("{api}/search/issues?q={}&sort=created&order=desc&per_page=5", q.replace(' ', "+"));
    let scope = if orgs.is_empty() {
        format!("user:{login}")
    } else {
        orgs.iter().map(|o| format!("org:{o}")).collect::<Vec<_>>().join(" ")
    };
    let (prs, open_prs) =
        github_prs(github_get(http, token, &search(format!("is:pr is:open archived:false {scope}"))).await);

    let activity = github_activity_cached(http, token).await;

    // Checked on every poll so the first one only fills the card.
    let pr_id = prs
        .iter()
        .filter_map(|v| v.get("id")?.as_str()?.parse::<i64>().ok())
        .max()
        .map(|n| n.to_string())
        .unwrap_or_default();
    let new_pr = !pr_id.is_empty() && is_new("github_pr", &pr_id);

    let event = pulse_event.or_else(|| {
        let p = prs.iter().find(|v| v["id"] == pr_id.as_str()).filter(|_| new_pr)?;
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Some(IntegrationEvent {
            success: true,
            label: format!("New PR · {}", s("repo")),
            detail: Some(s("title")),
            alert: false,
        })
    });

    (
        json!({
            "mine": pulse["mine"],
            "mainCi": pulse["mainCi"],
            "reviews": pulse["reviews"],
            "prs": prs,
            "openPrs": open_prs,
            "activity": activity,
        }),
        event,
    )
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

    fn pr(number: i64, sha: &str, state: &str) -> Value {
        json!({
            "number": number, "title": format!("PR {number}"), "url": "https://github.com/me/app/pull/1",
            "isDraft": false, "reviewDecision": "REVIEW_REQUIRED", "updatedAt": "2026-10-03T10:00:00Z",
            "repository": { "name": "app", "nameWithOwner": "me/app" },
            "commits": { "nodes": [{ "commit": { "oid": sha, "statusCheckRollup": { "state": state } } }] }
        })
    }

    fn pulse(prs: Vec<Value>, main: &str, main_sha: &str, reviews: Vec<i64>) -> Value {
        github_pulse(&json!({
            "viewer": {
                "pullRequests": { "nodes": prs },
                "repositories": { "nodes": [
                    { "name": "app", "nameWithOwner": "me/app", "url": "https://github.com/me/app", "isArchived": false,
                      "defaultBranchRef": { "name": "main", "target": { "oid": main_sha, "statusCheckRollup": { "state": main } } } },
                    { "name": "old", "nameWithOwner": "me/old", "url": "https://github.com/me/old", "isArchived": true,
                      "defaultBranchRef": { "name": "main", "target": { "oid": "x", "statusCheckRollup": { "state": "FAILURE" } } } },
                    { "name": "empty", "nameWithOwner": "me/empty", "url": "https://github.com/me/empty", "isArchived": false,
                      "defaultBranchRef": null }
                ] }
            },
            "reviewRequested": { "nodes": reviews.iter().map(|n| json!({
                "number": n, "title": "Look", "url": "https://github.com/team/api/pull/9", "isDraft": false,
                "createdAt": "2026-10-03T09:00:00Z", "repository": { "name": "api", "nameWithOwner": "team/api" }
            })).collect::<Vec<_>>() }
        }))
    }

    #[test]
    fn github_pulse_reads_prs_ci_and_reviews() {
        let p = pulse(vec![pr(7, "a", "PENDING"), pr(8, "b", "ERROR")], "SUCCESS", "m", vec![9]);
        assert_eq!(p["mine"][0]["id"], "me/app#7");
        assert_eq!(p["mine"][0]["ci"], "pending");
        assert_eq!(p["mine"][0]["review"], "pending");
        assert_eq!(p["mine"][1]["ci"], "failure");
        assert_eq!(p["mainCi"].as_array().unwrap().len(), 1, "archived and empty repos are skipped");
        assert_eq!(p["mainCi"][0]["ci"], "success");
        assert_eq!(p["reviews"][0]["id"], "team/api#9");
        assert_eq!(github_pulse(&json!({}))["mine"], json!([]));
        assert_eq!(ci_state(&Value::Null), "unknown");
    }

    #[test]
    fn github_pulse_events_fire_on_transitions_only() {
        let base = pulse(vec![pr(7, "a", "PENDING")], "SUCCESS", "m", vec![]);
        assert!(github_pulse_event(None, &base).is_none(), "the first pulse only fills the card");
        assert!(github_pulse_event(Some(&base), &base).is_none());

        let green = pulse(vec![pr(7, "a", "SUCCESS")], "SUCCESS", "m", vec![]);
        let e = github_pulse_event(Some(&base), &green).unwrap();
        assert!(e.success && !e.alert);
        assert_eq!(e.label, "CI passed · app#7");
        assert!(github_pulse_event(Some(&green), &green).is_none());

        let red = pulse(vec![pr(7, "a", "FAILURE")], "SUCCESS", "m", vec![]);
        let e = github_pulse_event(Some(&base), &red).unwrap();
        assert!(!e.success && e.alert);
        assert_eq!(e.label, "CI failed · app#7");

        let new_commit = pulse(vec![pr(7, "b", "PENDING")], "SUCCESS", "m", vec![]);
        assert!(github_pulse_event(Some(&red), &new_commit).is_none(), "a new commit waits for its CI");

        let main_red = pulse(vec![pr(7, "a", "PENDING")], "FAILURE", "m2", vec![]);
        assert_eq!(github_pulse_event(Some(&base), &main_red).unwrap().detail.unwrap(), "main branch");
        assert!(github_pulse_event(Some(&main_red), &main_red).is_none());

        let review = pulse(vec![pr(7, "a", "SUCCESS")], "SUCCESS", "m", vec![9]);
        let e = github_pulse_event(Some(&base), &review).unwrap();
        assert_eq!(e.label, "Review requested · api", "a review outranks a green CI");
    }

    #[test]
    fn github_activity_keeps_the_last_weeks() {
        let weeks: Vec<Value> = (0..30)
            .map(|w| json!({ "contributionDays": (0..7).map(|d| json!({
                "date": format!("w{w}d{d}"), "contributionCount": d, "weekday": d,
                "contributionLevel": if d == 0 { "NONE" } else { "THIRD_QUARTILE" }
            })).collect::<Vec<_>>() }))
            .collect();
        let data = json!({ "viewer": { "contributionsCollection": { "contributionCalendar": {
            "totalContributions": 1234, "weeks": weeks
        } } } });
        let a = github_activity(&data).unwrap();
        assert_eq!(a["total"], 1234);
        let days = a["days"].as_array().unwrap();
        assert_eq!(days.len(), GITHUB_ACTIVITY_WEEKS * 7);
        assert_eq!(days[0]["date"], "w7d0");
        assert_eq!(days[1]["level"], 3);
        assert_eq!(days[0]["level"], 0);
        assert!(github_activity(&json!({})).is_none());
    }
}
