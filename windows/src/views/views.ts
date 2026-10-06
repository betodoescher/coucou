// Island views — DOM ports of IslandViewContent.swift. Paddings, font sizes,
// colours and wording are copied from the Swift views so both platforms read
// identically.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Ticker } from "./ticker";
import { HOME_ID, State, TODO_PILL_ID, type AgentTask, type EditInfo } from "../core/state";
import { baseName, stepPlain } from "../core/steps";
import { Bridge, type CursorStatus } from "../core/bridge";
import { washRGBA, type IslandViewName, type Wash } from "../core/layout";
import { createMiniBot, pruneMiniBots } from "../mochi/minibots";
import { buildPrompt } from "./chat";
import { buildToday } from "./today";
import { Todos } from "../core/todoStore";
import { dayOf, dueCount } from "../core/todos";
import { buildChoose, buildUpload, buildUploading } from "./upload";
import { homeKey, renderIntegrationCard, type IntegrationCardHooks } from "./integrations";
import { buildWardrobe } from "./wardrobe";
import type { OutfitChoice, Worn } from "../mochi/outfits";

export interface ViewActions {
  setView(v: IslandViewName): void;
  collapse(): void;
  setFocus(id: string): void;
  openTerminal(): void;
  /** The ↗ button: opens whatever the focused pill points at. */
  openTarget(): void;
  openUrl(url: string): void;
  decide(d: "allow" | "deny"): void;
  /** Answers per question text: a label, or labels for a multi-select. */
  answerQuestion(answers: Record<string, string | string[]>): void;
  /** Hands the pending question back to the terminal. */
  replyInTerminal(): void;
  toggleSound(): void;
  setVolume(v: number): void;
  setAutoClose(seconds: number): void;
  openSettingsWindow(): void;
  blip(): void;
  /** Keeps an outfit: saved, and Mochi is proud of it. */
  wearOutfit(choice: OutfitChoice): void;
  /** Shows an outfit on Mochi while it is hovered in the wardrobe; null ends it. */
  previewOutfit(worn: Worn | null): void;
}

export interface ViewHost {
  el: HTMLElement;
  sync(): void;
  /** Called when the view becomes active, for views with a text field. */
  focus?(): void;
  /** Called every frame while the view is on screen. */
  tick?(nowMs: number): void;
}

// ── Shared pieces ─────────────────────────────────────────────────────────────

function card(wash: Wash, ...children: (Node | string)[]): HTMLElement {
  const el = h("div", { class: wash ? "card wash" : "card" }, ...children);
  if (wash) el.style.setProperty("--wash", washRGBA(wash));
  return el;
}

function btn(
  label: string,
  kind: "primary" | "secondary",
  onClick: () => void,
  kbd?: string,
): HTMLElement {
  return h(
    "button",
    { class: `btn ${kind}`, onclick: onClick },
    h("span", { text: label }),
    kbd ? h("span", { class: "kbd", text: kbd }) : null,
  );
}

/** AgentWho — coloured dot + task name + grey label. */
function agentWho(task: AgentTask | null, label: string): HTMLElement {
  const row = h("div", { class: "who-row" });
  if (task) {
    row.append(dot(task.color, 8), h("span", { class: "n", text: task.name }));
  }
  row.append(h("span", { text: label }));
  return row;
}

function stack(padLeft: number, padRight: number, ...children: Node[]): HTMLElement {
  const el = h("div", { class: "stack" }, ...children);
  el.style.padding = `4px ${padRight}px 4px ${padLeft}px`;
  return el;
}

// ── Header ────────────────────────────────────────────────────────────────────

/** A header pill's width plus the gap after it, as in `.header-pills`. */
const PILL_W = 34;
const PILL_GAP = 5;

export function buildHeader(actions: ViewActions): ViewHost {
  const tabHome = h("button", { class: "tab", title: "Overview", onclick: () => go("overview") }, svg(ICONS.house, 13));
  const tabChat = h("button", { class: "tab", title: "Ask", onclick: () => go("prompt") }, svg(ICONS.bubble, 13));
  const todoCount = h("span", { class: "tab-count" });
  const tabTodos = h(
    "button",
    { class: "tab tab-todos", title: "Today", onclick: () => go("todos") },
    svg(ICONS.checklist, 14, { stroke: 1.8 }),
    todoCount,
  );
  const tabDrop = h("button", { class: "tab", title: "Drop", onclick: () => go("upload") }, svg(ICONS.plus, 13));

  const gearBtn = h("button", { title: "Settings", onclick: () => go("settings") }, svg(ICONS.gear, 14));
  const soundBtn = h("button", { title: "Mute", onclick: () => actions.toggleSound() }, svg(ICONS.speakerOn, 14));
  const collapseBtn = h(
    "button",
    { title: "Make smaller", onclick: () => actions.collapse() },
    svg(ICONS.chevronUp, 12, { stroke: 2.4 }),
  );

  function go(v: IslandViewName) {
    actions.blip();
    actions.setView(v);
  }

  const pillBar = h("div", { class: "header-pills" });
  const moreMenu = h("div", { class: "pill-menu" });
  let pillKey = "";
  let hidden: AgentTask[] = [];

  function closeMenu() {
    moreMenu.classList.remove("on");
  }

  function pick(task: AgentTask) {
    closeMenu();
    actions.setFocus(task.id);
    if (task.id !== TODO_PILL_ID && State.view !== "overview") actions.setView("overview");
  }

  const moreBtn = h("button", {
    class: "pill-more",
    onclick: (e: Event) => {
      e.stopPropagation();
      if (moreMenu.classList.contains("on")) return closeMenu();
      clear(moreMenu);
      for (const t of hidden) {
        moreMenu.append(h(
          "button",
          { class: "pill-menu-row", onclick: () => pick(t) },
          createMiniBot(t, 14),
          h("span", { text: t.name }),
        ));
      }
      moreMenu.classList.add("on");
    },
  });
  document.addEventListener("click", (e) => {
    if (!moreMenu.contains(e.target as Node)) closeMenu();
  });

  const el = h(
    "div",
    { id: "header" },
    h("div", { class: "tabs" }, tabHome, tabChat, tabTodos, tabDrop),
    pillBar,
    h("div", { class: "header-actions" }, gearBtn, soundBtn, collapseBtn),
    moreMenu,
  );

  function syncPills() {
    const tasks = State.otherTasks;
    const css = getComputedStyle(pillBar);
    const inner = pillBar.clientWidth - parseFloat(css.paddingLeft) - parseFloat(css.paddingRight);
    const width = inner > 0 ? inner : 6 * (PILL_W + PILL_GAP);
    const fit = Math.max(1, Math.floor((width + PILL_GAP) / (PILL_W + PILL_GAP)));
    const shown = tasks.length > fit ? tasks.slice(0, fit - 1) : tasks;
    hidden = tasks.slice(shown.length);
    const key = `${fit}#` + tasks.map((t) => `${t.id}:${t.name}:${t.pillBadge ?? ""}`).join("|");
    if (key === pillKey) return;
    pillKey = key;
    clear(pillBar);
    for (const t of shown) pillBar.append(buildPill(t, () => pick(t)));
    if (hidden.length) {
      moreBtn.textContent = `+${hidden.length}`;
      moreBtn.title = hidden.map((t) => t.name).join(", ");
      pillBar.append(moreBtn);
    } else {
      closeMenu();
    }
    pruneMiniBots();
  }

  // The island's width animates frame by frame while it opens.
  new ResizeObserver(() => syncPills()).observe(pillBar);

  return {
    el,
    sync() {
      syncPills();
      if (State.mode !== "expanded") closeMenu();
      const v = State.view;
      tabHome.classList.toggle("on", v === "overview" || v === "empty");
      tabChat.classList.toggle("on", v === "prompt");
      tabTodos.classList.toggle("on", v === "todos");
      const due = dueCount(Todos.doc, dayOf(new Date()));
      todoCount.textContent = due ? String(Math.min(due, 99)) : "";
      tabDrop.classList.toggle("on", v === "upload");
      gearBtn.classList.toggle("on", v === "settings");
      clear(gearBtn);
      gearBtn.append(svg(v === "settings" ? ICONS.gearFill : ICONS.gear, 14));
      clear(soundBtn);
      soundBtn.append(svg(State.settings.soundEnabled ? ICONS.speakerOn : ICONS.speakerOff, 14));
      el.style.opacity = v === "confused" ? "0" : "1";
    },
  };
}

// ── Overview ──────────────────────────────────────────────────────────────────

/** One edit, in place of the ticker: the file, its +N −M and the changed lines. */
function diffCard(edit: EditInfo, onBack: () => void): HTMLElement {
  const lines = h("div", { class: "diff-lines" });
  for (const l of edit.lines) {
    const cls = l.startsWith("+") ? "add" : l.startsWith("-") ? "del" : "ctx";
    lines.append(h("div", { class: cls, text: l || " " }));
  }
  return h("div", { class: "diff-card" },
    h("div", { class: "diff-head" },
      h("button", { class: "icon-btn", title: "Back", onclick: onBack }, svg(ICONS.chevronLeft, 8, { stroke: 2.4 })),
      h("span", { class: "file", text: baseName(edit.path), title: edit.path }),
      h("span", { class: "diff-add", text: `+${edit.added}` }),
      h("span", { class: "diff-del", text: `−${edit.removed}` }),
    ),
    lines,
  );
}

function buildOverview(actions: ViewActions): ViewHost {
  const ticker = new Ticker((id) => {
    actions.blip();
    State.openDiff = id;
    State.notify();
  });
  const who = h("div", { class: "who" });
  const tickerBody = h("div", { class: "card-body" }, who, ticker.el);
  const leftBody = h("div", { class: "left-body" });
  const jump = h(
    "button",
    { class: "icon-btn jump", title: "Open", onclick: () => actions.openTarget() },
    svg(ICONS.arrowUpRight, 8),
  );
  const el = h("div", { class: "view overview" }, card(null, leftBody, jump));

  let detailOpen = false;
  let lastFocus: string | null = null;
  let mode: "ticker" | "card" | "diff" | null = null;
  let cardKey = "";

  const hooks: IntegrationCardHooks = {
    get detailOpen() {
      return detailOpen;
    },
    openDetail() {
      detailOpen = true;
      cardKey = "";
      State.notify();
    },
    closeDetail() {
      detailOpen = false;
      cardKey = "";
      State.notify();
    },
    openSettings: () => actions.openSettingsWindow(),
  };

  return {
    el,
    tick(nowMs: number) {
      if (mode === "ticker") ticker.tick(nowMs);
    },
    sync() {
      const task = State.focusTask;
      if (task?.id !== lastFocus) {
        lastFocus = task?.id ?? null;
        detailOpen = false;
        State.openDiff = null;
        cardKey = "";
        mode = null;
      }

      const edit = State.openDiff ? task?.edits?.find((e) => e.id === State.openDiff) : undefined;
      if (task?.source === "agent" && edit) {
        if (mode !== "diff" || cardKey !== edit.id) {
          mode = "diff";
          cardKey = edit.id;
          clear(leftBody);
          leftBody.append(diffCard(edit, () => {
            State.openDiff = null;
            State.notify();
          }));
        }
      // Agent sessions (Claude Code, Cursor, Kiro) show the ticker; every other
      // pill shows its own card, exactly like IntegrationCardView.
      } else if (task?.source === "agent") {
        State.openDiff = null;
        if (mode !== "ticker") {
          clear(leftBody);
          leftBody.append(tickerBody);
          mode = "ticker";
          cardKey = "";
        }
        clear(who);
        who.append(
          dot(task.color, 7),
          h("span", { class: "name", text: task.name }),
          h("span", { class: "tool", text: task.sessionCwd?.split(/[\\/]/).filter(Boolean).pop() ?? "" }),
        );
        if (task.steps.length > 1) {
          who.append(h("span", {
            class: "count",
            text: `${Math.min(task.stepIndex + 1, task.steps.length)}/${task.steps.length}`,
          }));
        }
        ticker.sync(task);
      } else if (task) {
        const info = State.integrations[task.id];
        const key = [
          task.id, detailOpen, task.state, task.steps.join("|"),
          info?.loaded, info?.error, info?.configured,
          JSON.stringify(info?.data ?? {}),
          task.id === HOME_ID ? homeKey() : "",
        ].join("~");
        if (key !== cardKey) {
          cardKey = key;
          mode = "card";
          clear(leftBody);
          leftBody.append(renderIntegrationCard(task, hooks));
        }
      }

      jump.style.display = detailOpen || mode === "diff" ? "none" : "";
    },
  };
}

function buildPill(task: AgentTask, onClick: () => void): HTMLElement {
  const pill = h("button", { class: "pill", title: task.name, onclick: onClick }, createMiniBot(task, 14));
  pill.style.borderColor = `${task.color}24`;
  pill.addEventListener("mouseenter", () => {
    pill.style.background = `${task.color}2e`;
    pill.style.borderColor = `${task.color}8c`;
    pill.style.boxShadow = `0 2px 10px ${task.color}59`;
  });
  pill.addEventListener("mouseleave", () => {
    pill.style.background = "";
    pill.style.borderColor = `${task.color}24`;
    pill.style.boxShadow = "";
  });

  if (task.pillBadge) {
    const colors = { approval: "#F5A524", finished: "#22C55E", error: "#F4505E" } as const;
    const icons = { approval: ICONS.bang, finished: ICONS.check, error: ICONS.xmark } as const;
    const inner = h("i", { style: `background:${colors[task.pillBadge]}` }, svg(icons[task.pillBadge], 6, { stroke: task.pillBadge === "finished" ? 3 : 0 }));
    const badge = h("div", { class: "pill-badge" }, inner);
    badge.style.boxShadow = `0 0 4px ${colors[task.pillBadge]}99`;
    pill.append(badge);
  }
  return pill;
}

// ── Empty ─────────────────────────────────────────────────────────────────────

function buildEmpty(actions: ViewActions): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 118px;flex-direction:row;align-items:center;gap:16px" },
    h(
      "div",
      { style: "display:flex;flex-direction:column;gap:5px" },
      h("div", { class: "title", text: "Nothing running right now." }),
      h("div", { class: "sub", text: "Drop a file or window, or ask me anything." }),
    ),
    h("div", { class: "grow" }),
    btn("Ask Claude", "primary", () => actions.setView("prompt")),
  );
  return { el: h("div", { class: "view" }, card(null, body)), sync() {} };
}

// ── Approval ──────────────────────────────────────────────────────────────────

function buildApproval(actions: ViewActions): ViewHost {
  const who = h("div");
  const code = h("div", { class: "code" });
  const row = h("div", { class: "actions" });
  const el = h("div", { class: "view" }, card("amber", stack(116, 16, who, code, row)));
  let rowKey = "";
  return {
    el,
    sync() {
      clear(who);
      who.append(agentWho(State.focusTask, "needs permission"));
      // The whole point of approving here rather than in the terminal: this line
      // is the command, the file path or the URL being authorised, not just the
      // name of the tool asking.
      code.textContent = State.pendingApproval?.command || State.pendingApproval?.tool || "…";
      // Two buttons, built once. Rebuilding them between a mouse-down and a
      // mouse-up would swallow the click, and there is nothing left to vary:
      // "Always" is gone until the remembered-rules list exists to back it.
      if (rowKey === "built") return;
      rowKey = "built";
      clear(row);
      row.append(
        btn("Deny", "secondary", () => actions.decide("deny"), "N"),
        btn("Allow", "primary", () => actions.decide("allow"), "Y"),
      );
    },
  };
}

// ── Question ──────────────────────────────────────────────────────────────────

function buildQuestion(actions: ViewActions): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title" });
  const row = h("div", { class: "actions" });
  const el = h("div", { class: "view" }, card("cyan", stack(116, 16, who, title, row)));

  // A question with options: one at a time, answers kept until the last one.
  let requestId = "";
  let index = 0;
  let picked: string[][] = [];
  const other = h("input", { class: "ask-other", type: "text", placeholder: "Other…", spellcheck: "false" }) as HTMLInputElement;

  const submit = () => {
    const qs = State.pendingApproval?.questions ?? [];
    const answers: Record<string, string | string[]> = {};
    qs.forEach((q, i) => {
      const labels = picked[i] ?? [];
      if (labels.length) answers[q.question] = q.multiSelect ? labels : labels[0];
    });
    actions.answerQuestion(answers);
  };
  /** Records the current question's answer and moves on, or sends them all. */
  const next = () => {
    const typed = other.value.trim();
    const q = State.pendingApproval?.questions?.[index];
    if (!q) return;
    if (typed) picked[index] = q.multiSelect ? [...(picked[index] ?? []), typed] : [typed];
    if (!(picked[index]?.length)) return;
    other.value = "";
    if (index + 1 < (State.pendingApproval?.questions?.length ?? 0)) {
      index++;
      render();
    } else {
      submit();
    }
  };
  other.addEventListener("keydown", (e) => {
    if (e.key === "Enter") next();
  });

  function render() {
    const qs = State.pendingApproval?.questions;
    if (!qs) return;
    const q = qs[index];
    clear(who);
    const count = qs.length > 1 ? ` · ${index + 1}/${qs.length}` : "";
    who.append(agentWho(State.focusTask, `${q.header || "asks"}${count}`));
    title.textContent = q.question;
    title.className = "title ask-q";
    title.title = q.question;
    clear(row);
    const opts = h("div", { class: "ask-opts" });
    for (const o of q.options) {
      const on = picked[index]?.includes(o.label) ?? false;
      const b = h("button", { class: `btn secondary ask-opt${on ? " on" : ""}`, text: o.label });
      if (o.description) b.title = o.description;
      b.addEventListener("click", () => {
        if (q.multiSelect) {
          const cur = picked[index] ?? [];
          picked[index] = cur.includes(o.label) ? cur.filter((l) => l !== o.label) : [...cur, o.label];
          render();
        } else {
          picked[index] = [o.label];
          next();
        }
      });
      opts.append(b);
    }
    const last = index + 1 === qs.length;
    const foot = h("div", { class: "ask-foot" }, other);
    if (q.multiSelect) foot.append(btn(last ? "Send" : "Next", "primary", next));
    foot.append(h("div", { class: "grow" }), h("button", {
      class: "link-btn ask-terminal",
      text: "Reply in terminal",
      onclick: () => actions.replyInTerminal(),
    }));
    row.append(h("div", { class: "ask-body" }, opts, foot));
  }

  return {
    el,
    sync() {
      const req = State.pendingApproval;
      if (req?.questions) {
        if (req.requestId === requestId) return;
        requestId = req.requestId;
        index = 0;
        picked = [];
        other.value = "";
        render();
        return;
      }
      requestId = "";
      title.className = "title";
      title.removeAttribute("title");
      clear(who);
      who.append(agentWho(State.focusTask, "Claude Code is asking a question"));
      const task = State.focusTask;
      title.textContent = stepPlain(task, task?.steps.at(-1)) || "Claude needs an answer.";
      clear(row);
      row.append(h("div", { class: "sub", text: "Answer in your terminal — Coucou can't reply for you yet." }));
    },
  };
}

// ── Error ─────────────────────────────────────────────────────────────────────

function buildError(actions: ViewActions): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title", text: "Workflow stopped." });
  const detail = h("div", { class: "detail clamp-2" });
  const row = h("div", { class: "actions" },
    btn("Retry", "primary", () => actions.setView(State.defaultView())),
    btn("Open in n8n", "secondary", () => actions.openUrl("")),
  );
  const body = stack(116, 16, who, title, detail, row);
  body.classList.add("bot-slot");
  const el = h("div", { class: "view" }, card("red", body));
  return {
    el,
    sync() {
      const task = State.focusTask;
      clear(who);
      who.append(agentWho(task, task?.source === "n8n" ? "n8n" : task?.source === "agent" ? "" : "Claude Code"));
      title.textContent = task?.source === "n8n" ? "Workflow stopped." : "Session stopped on an error.";
      detail.textContent = stepPlain(task, task?.steps.at(-1)) || "No detail available.";
    },
  };
}

// ── Finished ──────────────────────────────────────────────────────────────────

function buildFinished(actions: ViewActions): ViewHost {
  const who = h("div");
  const title = h("div", { class: "title clamp-2" });
  const row = h("div", { class: "actions" },
    btn("Open terminal", "primary", () => actions.openTerminal()),
    btn("OK", "secondary", () => actions.collapse()),
  );
  const body = stack(116, 16, who, title, row);
  body.classList.add("bot-slot");
  const el = h("div", { class: "view" }, card("green", body));
  return {
    el,
    sync() {
      clear(who);
      who.append(agentWho(State.focusTask, State.focusTask?.source === "agent" ? "finished" : "Claude Code finished"));
      // A tool failure the agent got past isn't how the turn ended.
      const last = stepPlain(State.focusTask, State.focusTask?.steps.at(-1));
      title.textContent = last && !last.startsWith("⚠") ? last : "Session finished";
    },
  };
}

// ── Confused ──────────────────────────────────────────────────────────────────

function buildConfused(): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 128px" },
    h("div", { class: "title", text: "Too many hits at once." }),
    h("div", { class: "sub", text: "Give me a sec — back to work in three seconds." }),
  );
  return { el: h("div", { class: "view" }, card("pink", body)), sync() {} };
}

// ── Note ──────────────────────────────────────────────────────────────────────

function buildNote(): ViewHost {
  const title = h("div", { class: "title" });
  const el = h("div", { class: "view" }, card(null, h("div", { class: "stack", style: "padding:0 18px 0 98px" }, title)));
  return {
    el,
    sync() {
      title.textContent = State.noteMessage ?? "";
    },
  };
}

// ── In-island settings ────────────────────────────────────────────────────────

/** Green when the CLI is signed in. Its status spawns the CLI: checked at most every 30 s. */
function cliBadge(name: string, status: () => Promise<CursorStatus | null>) {
  const el = h("span", { class: "status-badge" });
  let loggedIn = false;
  let checkedAt = 0;
  const paint = () => {
    clear(el);
    el.append(dot(loggedIn ? "#22C55E" : "#F4505E", 6), h("span", { text: name }));
  };
  return {
    el,
    sync() {
      paint();
      if (Date.now() - checkedAt < 30_000) return;
      checkedAt = Date.now();
      void status().then((st) => {
        loggedIn = st?.loggedIn ?? false;
        paint();
      });
    },
  };
}

function buildSettings(actions: ViewActions): ViewHost {
  const soundSwitch = h("button", { class: "switch", onclick: () => actions.toggleSound() });
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    oninput: (e: Event) => actions.setVolume(Number((e.target as HTMLInputElement).value)),
  }) as HTMLInputElement;
  const autoLabel = h("span", {});
  const segButtons = [10, 15, 30].map((s) =>
    h("button", { onclick: () => actions.setAutoClose(s) }, `${s}s`),
  );
  const claudeBadge = h("span", { class: "status-badge" });
  const apiBadge = h("span", { class: "status-badge" });
  const cursorBadge = cliBadge("Cursor", Bridge.cursorStatus);
  const kiroBadge = cliBadge("Kiro", Bridge.kiroStatus);

  const rows = h(
    "div",
    { class: "settings-rows" },
    h("div", { class: "settings-row" }, soundSwitch, h("span", { text: "Sound" }), volume),
    h(
      "div",
      { class: "settings-row" },
      svg(ICONS.timer, 12),
      autoLabel,
      h("div", { class: "seg" }, ...segButtons),
    ),
    h(
      "div",
      { class: "settings-row", style: "gap:14px" },
      claudeBadge,
      apiBadge,
      cursorBadge.el,
      kiroBadge.el,
      h("div", { class: "grow" }),
      h("button", {
        class: "link-btn",
        style: "color:#8e939c;font-size:11.5px",
        text: "Settings…",
        onclick: () => actions.openSettingsWindow(),
      }),
    ),
  );

  const el = h("div", { class: "view" },
    card(null, h("div", { class: "stack", style: "padding:14px 16px 14px 84px" }, rows)));

  return {
    el,
    sync() {
      const s = State.settings;
      soundSwitch.classList.toggle("on", s.soundEnabled);
      volume.value = String(s.soundVolume);
      volume.style.opacity = s.soundEnabled ? "1" : "0.4";
      autoLabel.textContent = `Auto-close · ${Math.round(s.autoCloseInterval)}s`;
      segButtons.forEach((b, i) => b.classList.toggle("on", s.autoCloseInterval === [10, 15, 30][i]));
      clear(claudeBadge);
      claudeBadge.append(
        dot(s.hooksInstalled ? "#22C55E" : "#F4505E", 6),
        h("span", { text: "Claude Code" }),
      );
      clear(apiBadge);
      apiBadge.append(dot("#F4505E", 6), h("span", { text: "API" }));
      cursorBadge.sync();
      kiroBadge.sync();
    },
  };
}

// ── Placeholders filled in later stages ───────────────────────────────────────

function buildPlaceholder(title: string, sub: string): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 118px" },
    h("div", { class: "title", text: title }),
    h("div", { class: "sub", text: sub }),
  );
  return { el: h("div", { class: "view" }, card(null, body)), sync() {} };
}

// ── Registry ──────────────────────────────────────────────────────────────────

export function buildViews(
  actions: ViewActions,
  onChatHeightChange: () => void,
): Map<IslandViewName, ViewHost> {
  const map = new Map<IslandViewName, ViewHost>();
  map.set("overview", buildOverview(actions));
  map.set("empty", buildEmpty(actions));
  map.set("approval", buildApproval(actions));
  map.set("question", buildQuestion(actions));
  map.set("error", buildError(actions));
  map.set("finished", buildFinished(actions));
  map.set("confused", buildConfused());
  map.set("note", buildNote());
  map.set("settings", buildSettings(actions));
  map.set("prompt", buildPrompt(onChatHeightChange));
  map.set("todos", buildToday(onChatHeightChange));
  map.set("upload", buildUpload());
  map.set("uploading", buildUploading());
  map.set("choose", buildChoose(actions));
  map.set("wardrobe", buildWardrobe(actions));
  // Not in the Windows v1: sending a file by email, window attach + web result.
  map.set("mail", buildPlaceholder("Sending by email isn't in this version.", ""));
  map.set("searching", buildPlaceholder("Claude is searching…", ""));
  map.set("result", buildPlaceholder("Result", ""));
  return map;
}
