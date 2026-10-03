// Integration cards shown in the overview's left card — DOM ports of
// IntegrationCardView and friends from IslandViewContent.swift.
//
// Cal.com is the one simplification: macOS shows a three-level calendar
// (month → day → booking); here it is the list of upcoming bookings.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { HOME_ID, State, isAgentPill, type AgentTask } from "../core/state";
import { Bridge } from "../core/bridge";
import { Todos } from "../core/todoStore";
import { dayOf, upNext, whenLabel } from "../core/todos";

/** Same shape as the Swift `timeAgo` computed properties. */
export function timeAgo(value: unknown): string {
  const date = typeof value === "number" ? new Date(value) : new Date(String(value));
  const diff = (Date.now() - date.getTime()) / 1000;
  if (!Number.isFinite(diff)) return "";
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  return `${Math.floor(diff / 86400)}d`;
}

function header(color: string, name: string, kind: string, extra?: Node): HTMLElement {
  const row = h("div", { class: "int-head" }, dot(color, 7), h("b", { text: name }), h("span", { text: kind }));
  if (extra) row.append(extra);
  return row;
}

/** Highlighted first row + plain rows, the layout every list card shares. */
function listRow(accent: string, first: boolean, ...children: Node[]): HTMLElement {
  const row = h("div", { class: first ? "int-row first" : "int-row" }, dot(accent, 5), ...children);
  if (first) row.style.background = `${accent}14`;
  return row;
}

function get(id: string): Record<string, unknown> {
  return (State.integrations[id]?.data ?? {}) as Record<string, unknown>;
}

function arr(id: string, key: string): Record<string, unknown>[] {
  const v = get(id)[key];
  return Array.isArray(v) ? (v as Record<string, unknown>[]) : [];
}

// ── Not configured / idle ─────────────────────────────────────────────────────

const OPEN_URLS: Record<string, string> = {
  integration_resend: "https://resend.com/emails",
  integration_vercel: "https://vercel.com/dashboard",
  integration_github: "https://github.com",
  integration_stripe: "https://dashboard.stripe.com/payments",
  integration_notion: "https://notion.so",
  integration_calcom: "https://app.cal.com/bookings",
};

function idleCard(task: AgentTask, openSettings: () => void): HTMLElement {
  const info = State.integrations[task.id];
  const configured = info?.configured ?? false;
  const error = info?.error ?? null;
  const label = error ?? (configured ? "Connected · loading…" : "Key not configured");
  const statusColor = error || !configured ? "#F4505E" : "#22C55E";

  const actions = h("div", { class: "int-actions" });
  if (task.id === "integration_n8n") {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: "Open n8n",
        onclick: () => void Bridge.openN8n(),
      }),
    );
  } else if (OPEN_URLS[task.id]) {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: `Open ${task.name}`,
        onclick: () => void Bridge.openUrl(OPEN_URLS[task.id]),
      }),
    );
  }
  if (configured) {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: "Refresh",
        onclick: () => void Bridge.refreshIntegration(task.id),
      }),
    );
  } else {
    actions.append(
      h("button", { class: "link-btn", style: "color:#8e939c", text: "Settings…", onclick: openSettings }),
    );
  }

  return h(
    "div",
    { class: "int-card" },
    header(task.color, task.name, "Integration"),
    h("div", { class: "int-status" }, dot(statusColor, 5), h("span", { text: label })),
    actions,
  );
}

// ── Vercel ────────────────────────────────────────────────────────────────────

function vercelCard(onDetail: () => void): HTMLElement {
  const deployments = arr("integration_vercel", "deployments");
  const rows = h("div", { class: "int-rows" });
  deployments.slice(0, 3).forEach((d, i) => {
    const accent = d.state === "READY" ? "#22C55E" : "#F4505E";
    const name = h("span", { class: "int-name", text: String(d.projectName ?? "") });
    const ago = h("span", { class: "int-ago", text: timeAgo(d.createdAt) });
    if (i === 0) {
      const more = h(
        "button",
        { class: "int-more", title: "Details", onclick: onDetail },
        svg(ICONS.ellipsis, 8),
      );
      rows.append(listRow(accent, true, name, ago, more));
    } else {
      rows.append(listRow(accent, false, name, ago));
    }
  });
  return h("div", { class: "int-card" }, header("#7C5CFF", "Vercel", "Deployments"), rows);
}

function vercelDetail(onBack: () => void): HTMLElement {
  const d = arr("integration_vercel", "deployments")[0] ?? {};
  const success = d.state === "READY";
  const accent = success ? "#22C55E" : "#F4505E";
  const status = success ? "Ready" : d.state === "CANCELED" ? "Canceled" : "Error";
  const body = h("div", { class: "int-detail-body" });
  if (d.commitMessage) body.append(h("div", { class: "int-commit", text: String(d.commitMessage) }));
  const meta = h("div", { class: "int-meta" });
  if (d.branch) meta.append(h("span", { text: String(d.branch) }));
  meta.append(h("span", { text: `${timeAgo(d.createdAt)} ago` }));
  body.append(meta);
  if (d.url) {
    body.append(
      h("button", {
        class: "int-link",
        text: String(d.url),
        onclick: () => void Bridge.openUrl(`https://${d.url}`),
      }),
    );
  }
  return h(
    "div",
    { class: "int-card detail" },
    h(
      "div",
      { class: "int-detail-head" },
      h("button", { class: "int-back", onclick: onBack }, svg(ICONS.chevronLeft, 10, { stroke: 2.4 })),
      dot(accent, 6),
      h("b", { text: String(d.projectName ?? "Deployment") }),
      h("span", { class: "int-badge", style: `color:${accent};background:${accent}24`, text: status }),
    ),
    body,
  );
}

// ── Resend ────────────────────────────────────────────────────────────────────

function resendCard(): HTMLElement {
  const emails = arr("integration_resend", "emails");
  const total = get("integration_resend").total;
  const extra =
    total != null
      ? h("span", { class: "int-total" }, h("i", { class: "pulse" }), h("span", { text: String(total) }))
      : undefined;
  const rows = h("div", { class: "int-rows" });
  emails.slice(0, 3).forEach((e, i) => {
    const delivered = e.lastEvent === "delivered";
    const accent = delivered ? "#22C55E" : "#F4505E";
    const to = Array.isArray(e.to) ? String(e.to[0] ?? "?") : "?";
    const short = to.split("@")[0];
    const cells: Node[] = [
      h("span", { class: "int-name", text: short }),
      h("span", { class: "int-ago", text: timeAgo(e.createdAt) }),
    ];
    if (i === 0 && e.subject) cells.push(h("span", { class: "int-sub", text: String(e.subject) }));
    rows.append(listRow(accent, i === 0, ...cells));
  });
  return h("div", { class: "int-card" }, header("#22C55E", "Resend", "Emails", extra), rows);
}

// ── GitHub ────────────────────────────────────────────────────────────────────

function statRow(icon: string, color: string, label: string, value: string): HTMLElement {
  return h(
    "div",
    { class: "int-stat" },
    h("i", { class: "int-stat-icon", style: `color:${color}` }, svg(icon, 10)),
    h("span", { class: "int-stat-label", text: label }),
    h("span", { class: "int-stat-value", text: value }),
  );
}

function githubCard(): HTMLElement {
  const d = get("integration_github");
  const stars = Number(d.totalStars ?? 0);
  const repos = Number(d.totalRepos ?? 0);
  const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));

  // Failed CI first, then PRs waiting for your review, then the org's open PRs.
  const items: [string, string, string, unknown][] = [
    ...arr("integration_github", "failures").map((f): [string, string, string, unknown] =>
      ["#F4505E", `${f.repo} · ${f.workflow}`, String(f.url ?? ""), f.createdAt]),
    ...arr("integration_github", "reviews").map((p): [string, string, string, unknown] =>
      ["#F5A524", `${p.repo} · ${p.title}`, String(p.url ?? ""), p.createdAt]),
    ...arr("integration_github", "prs")
      .filter((p) => !arr("integration_github", "reviews").some((r) => r.id === p.id))
      .map((p): [string, string, string, unknown] =>
        ["#22C55E", `${p.repo} · ${p.title}`, String(p.url ?? ""), p.createdAt]),
  ];
  if (items.length > 0) {
    const rows = h("div", { class: "int-rows" });
    items.slice(0, 3).forEach(([accent, text, url, at], i) => {
      const row = listRow(
        accent,
        i === 0,
        h("span", { class: "int-name", text }),
        h("span", { class: "int-ago", text: timeAgo(at) }),
      );
      if (url) {
        row.style.cursor = "pointer";
        row.addEventListener("click", () => void Bridge.openUrl(url));
      }
      rows.append(row);
    });
    const open = Number(d.openPrs ?? 0);
    return h("div", { class: "int-card" }, header("#F4505E", "GitHub", open ? `${open} open PRs` : "Pull requests & CI"), rows);
  }

  return h(
    "div",
    { class: "int-card" },
    header("#F4505E", "GitHub", "Overview"),
    h(
      "div",
      { class: "int-stats" },
      statRow(ICONS.star, "#F5A524", "Total stars", fmt(stars)),
      statRow(ICONS.stack, "#6B7079", "Repositories", String(repos)),
    ),
  );
}

// ── Stripe ────────────────────────────────────────────────────────────────────

function stripeCard(): HTMLElement {
  const d = get("integration_stripe");
  const balance = (Number(d.balance ?? 0) / 100).toFixed(2);
  const currency = String(d.currency ?? "eur").toUpperCase();
  const rows = h("div", { class: "int-rows tight" });
  for (const p of arr("integration_stripe", "payments")) {
    const success = p.status === "succeeded";
    const accent = success ? "#22C55E" : "#F4505E";
    rows.append(
      h(
        "div",
        { class: "int-row" },
        dot(accent, 5),
        h("span", { class: "int-name", text: String(p.description ?? "Payment") }),
        h("span", {
          class: "int-amount",
          style: "color:#22c55e",
          text: `+${(Number(p.amount ?? 0) / 100).toFixed(2)}`,
        }),
        h("span", { class: "int-ago", text: timeAgo(p.createdAt) }),
      ),
    );
  }
  return h(
    "div",
    { class: "int-card" },
    header("#0570DE", "Stripe", "Payments"),
    h("div", { class: "int-balance" }, h("span", { text: balance }), h("i", { text: currency })),
    rows,
  );
}

// ── Notion ────────────────────────────────────────────────────────────────────

function notionCard(): HTMLElement {
  const rows = h("div", { class: "int-rows tight" });
  for (const p of arr("integration_notion", "pages").slice(0, 3)) {
    rows.append(
      h(
        "button",
        {
          class: "int-page",
          onclick: () => {
            if (typeof p.url === "string") void Bridge.openUrl(p.url);
          },
        },
        p.emoji
          ? h("span", { class: "int-emoji", text: String(p.emoji) })
          : h("i", { class: "int-emoji" }, svg(ICONS.doc, 9)),
        h("span", { class: "int-name", text: String(p.title ?? "Untitled") }),
        h("span", { class: "int-ago", text: timeAgo(p.lastEditedAt) }),
      ),
    );
  }
  return h("div", { class: "int-card" }, header("#E8E8E8", "Notion", "Recent"), rows);
}

// ── Cal.com ───────────────────────────────────────────────────────────────────

function calcomCard(): HTMLElement {
  const bookings = arr("integration_calcom", "bookings")
    .slice()
    .sort((a, b) => new Date(String(a.start)).getTime() - new Date(String(b.start)).getTime());
  const rows = h("div", { class: "int-rows tight" });
  if (bookings.length === 0) {
    rows.append(h("div", { class: "int-empty", text: "No calls scheduled" }));
  }
  for (const b of bookings.slice(0, 3)) {
    const when = new Date(String(b.start));
    const day = when.toLocaleDateString(undefined, { day: "2-digit", month: "2-digit" });
    const time = when.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
    rows.append(
      h(
        "div",
        { class: "int-row" },
        dot("#C9956A", 4),
        h("span", { class: "int-time", text: `${day} ${time}` }),
        h("span", { class: "int-name", text: String(b.title ?? "Meeting") }),
      ),
    );
  }
  return h("div", { class: "int-card" }, header("#C9956A", "Cal.com", "Schedule"), rows);
}

// ── n8n ───────────────────────────────────────────────────────────────────────

function n8nCard(task: AgentTask, onDetail: () => void, openSettings: () => void): HTMLElement {
  const hasActivity = task.steps.length > 0 && (task.state === "finished" || task.state === "error");
  if (!hasActivity) return idleCard(task, openSettings);
  const success = task.state === "finished";
  const accent = success ? "#22C55E" : "#F4505E";
  return h(
    "div",
    { class: "int-card" },
    header("#F29B38", "n8n", "Workflow"),
    h(
      "div",
      { class: "int-actions" },
      h(
        "button",
        {
          class: "int-pill",
          style: `background:${accent}1a;border-color:${accent}38`,
          onclick: onDetail,
        },
        dot(accent, 5),
        h("span", { class: "int-name", text: task.steps[0] ?? "Workflow" }),
        svg(ICONS.ellipsis, 8),
      ),
    ),
  );
}

function n8nDetail(task: AgentTask, onBack: () => void): HTMLElement {
  const success = task.state === "finished";
  const accent = success ? "#22C55E" : "#F4505E";
  const detail = task.steps[1];
  return h(
    "div",
    { class: "int-card detail" },
    h(
      "div",
      { class: "int-detail-head" },
      h("button", { class: "int-back", onclick: onBack }, svg(ICONS.chevronLeft, 10, { stroke: 2.4 })),
      dot(accent, 6),
      h("b", { text: task.steps[0] ?? "Workflow" }),
      h("span", {
        class: "int-badge",
        style: `color:${accent};background:${accent}24`,
        text: success ? "Success" : "Failed",
      }),
    ),
    detail
      ? h("pre", { class: "int-detail-text", text: detail })
      : h("div", {
          class: "int-status",
          text: success ? "Completed successfully." : "No error details available.",
        }),
  );
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

export interface IntegrationCardHooks {
  detailOpen: boolean;
  openDetail(): void;
  closeDetail(): void;
  openSettings(): void;
}

/** True when this integration has data worth showing instead of the idle card. */
export function hasIntegrationData(id: string): boolean {
  const info = State.integrations[id];
  if (!info || info.error) return false;
  switch (id) {
    case "integration_vercel":
      return arr(id, "deployments").length > 0;
    case "integration_resend":
      return arr(id, "emails").length > 0;
    case "integration_github":
      return get(id).totalRepos != null;
    case "integration_stripe":
      return info.loaded;
    case "integration_notion":
      return arr(id, "pages").length > 0;
    case "integration_calcom":
      return info.loaded;
    default:
      return false;
  }
}

// ── Home ──────────────────────────────────────────────────────────────────────

/** WMO weather code → icon and word (Open-Meteo `weather_code`). */
function sky(code: number, isDay: boolean): [string, string] {
  if (code <= 1) return [isDay ? ICONS.sun : ICONS.moon, "Clear"];
  if (code <= 3) return [ICONS.cloud, "Cloudy"];
  if (code === 45 || code === 48) return [ICONS.cloud, "Fog"];
  if ((code >= 71 && code <= 77) || code === 85 || code === 86) return [ICONS.snow, "Snow"];
  if (code >= 95) return [ICONS.storm, "Storm"];
  return [ICONS.rain, "Rain"];
}

const deg = (v: unknown) => (typeof v === "number" ? `${Math.round(v)}°` : "–");

function homeSummary() {
  const now = new Date();
  const today = dayOf(now);
  const next = upNext(Todos.doc, 1)[0] ?? null;
  const habits = Todos.doc.habits ?? [];
  const agents = State.tasks.filter((t) => isAgentPill(t.id));
  return {
    today,
    next: next ? { title: next.title, when: next.due ? whenLabel(next, now) : "" } : null,
    habitsDone: habits.filter((x) => x.days.includes(today)).length,
    habits: habits.length,
    agents: agents.length,
    waiting: agents.filter((t) => t.state === "approval" || t.pillBadge === "approval").length,
    weather: State.integrations.weather ?? null,
    usage: State.usage,
  };
}

function tokens(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`;
  return String(n);
}

/** "Claude 1.2M · Kiro 0.9 cr · Cursor 3", only the agents used today. */
function usageText(u: NonNullable<typeof State.usage>): string {
  const bits: string[] = [];
  if (u.claudeTokens) bits.push(`Claude ${tokens(u.claudeTokens)}`);
  if (u.kiroCredits >= 0.01) bits.push(`Kiro ${u.kiroCredits.toFixed(u.kiroCredits < 10 ? 2 : 1)} cr`);
  if (u.cursorRequests) bits.push(`Cursor ${u.cursorRequests}`);
  return bits.join(" · ");
}

/** Changes whenever the home card would draw differently. */
export function homeKey(): string {
  return JSON.stringify(homeSummary());
}

function homeCard(task: AgentTask, openSettings: () => void): HTMLElement {
  const s = homeSummary();
  const w = (s.weather?.data ?? {}) as Record<string, unknown>;
  const hasWeather = typeof w.temp === "number";
  const city = State.settings.weatherCity.trim();

  const now = hasWeather
    ? (() => {
        const [icon, word] = sky(Number(w.code ?? 0), w.isDay !== false);
        return h("span", { class: "home-now", title: word }, svg(icon, 13, { stroke: 1.8 }), h("span", { text: deg(w.temp) }));
      })()
    : undefined;
  const head = header(task.color, hasWeather ? String(w.city ?? city) : "Today", "", now);

  const weatherLine = hasWeather
    ? h("div", { class: "int-status" }, h("span", {
        text: `H ${deg(w.max)} · L ${deg(w.min)}${typeof w.rain === "number" ? ` · Rain ${w.rain}%` : ""}`,
      }))
    : city && s.weather?.error
      ? h("div", { class: "int-status" }, dot("#F4505E", 5), h("span", { class: "int-name", text: s.weather.error }))
      : city
        ? h("div", { class: "int-status" }, h("span", { text: "Loading weather…" }))
        : h("div", { class: "int-status" }, h("button", {
            class: "link-btn", style: "color:#8e939c;padding:0", text: "Set a city for weather…", onclick: openSettings,
          }));

  const taskLine = h("div", { class: "int-status" },
    svg(ICONS.checklist, 11, { stroke: 1.8 }),
    s.next
      ? h("span", { class: "int-name", text: s.next.when ? `${s.next.when} · ${s.next.title}` : s.next.title })
      : h("span", { text: "No open tasks" }),
  );

  const bits: string[] = [];
  if (s.habits) bits.push(`Habits ${s.habitsDone}/${s.habits}`);
  bits.push(s.agents ? `${s.agents} agent${s.agents > 1 ? "s" : ""}` : "No agents");
  if (s.waiting) bits.push(`${s.waiting} waiting`);
  const statusLine = h("div", { class: "int-status" },
    dot(s.waiting ? "#F5A524" : s.agents ? "#22C55E" : "#6b7079", 5),
    h("span", { class: "int-name", text: bits.join(" · ") }),
  );

  const used = s.usage ? usageText(s.usage) : "";
  const usageLine = used
    ? h("div", {
        class: "int-status",
        title: "AI used today: Claude tokens, Kiro credits, Cursor prompts",
      }, svg(ICONS.timer, 11), h("span", { class: "int-name", text: used }))
    : null;

  return h("div", { class: "int-card" }, head, weatherLine, taskLine, statusLine, usageLine);
}

export function renderIntegrationCard(task: AgentTask, hooks: IntegrationCardHooks): HTMLElement {
  if (task.id === HOME_ID) return homeCard(task, hooks.openSettings);
  if (task.id === "integration_n8n") {
    const hasActivity = task.steps.length > 0 && (task.state === "finished" || task.state === "error");
    return hooks.detailOpen && hasActivity
      ? n8nDetail(task, hooks.closeDetail)
      : n8nCard(task, hooks.openDetail, hooks.openSettings);
  }
  if (task.id === "integration_vercel" && hasIntegrationData(task.id)) {
    return hooks.detailOpen ? vercelDetail(hooks.closeDetail) : vercelCard(hooks.openDetail);
  }
  if (!hasIntegrationData(task.id)) return idleCard(task, hooks.openSettings);

  switch (task.id) {
    case "integration_resend":
      return resendCard();
    case "integration_github":
      return githubCard();
    case "integration_stripe":
      return stripeCard();
    case "integration_notion":
      return notionCard();
    case "integration_calcom":
      return calcomCard();
    default:
      return idleCard(task, hooks.openSettings);
  }
}

export { clear };
