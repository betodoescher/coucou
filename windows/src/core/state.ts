// App state — mirror of AppState.swift (the parts the island needs).

import type { BotEmoteName, BotStateName, IslandMode, IslandViewName } from "./layout";
import type { EyeShape } from "../mochi/engine";
import type { Worn } from "../mochi/outfits";
import type { ClaudePlan, CursorPlan, KiroPlan } from "./bridge";

export type AgentSource = "home" | "n8n" | "agent" | "todos";

/** The Tasks pill: shown while there are open to-dos, opens the Tasks tab. */
export const TODO_PILL_ID = "todos";
/** The home card: weather, next task, habits, agents. Always first. */
export const HOME_ID = "home";
/** Claude Code's session pill; comes and goes with the session, like Cursor's. */
export const CLAUDE_ID = "integration_claude";

const HOME_TASK: AgentTask = {
  id: HOME_ID, name: "Today", color: "#FBBF24", state: "idle", stepIndex: 0, steps: [],
  source: "home", isIntegration: false,
};

/** Agent session pills: Claude Code and the hook-driven agent_* ones. */
export function isAgentPill(id: string): boolean {
  return id === CLAUDE_ID || id.startsWith("agent_");
}
export type PillBadge = "approval" | "finished" | "error";

/** The open panel of the Today tab. */
export type TodayTab = "tasks" | "habits" | "notes";

export interface AgentTask {
  id: string;
  name: string;
  color: string;
  state: BotStateName;
  stepIndex: number;
  steps: string[];
  source: AgentSource;
  isIntegration: boolean;
  emote?: BotEmoteName | null;
  miniEye?: EyeShape | null;
  pillBadge?: PillBadge | null;
  sessionCwd?: string | null;
  /** Recent file edits, referenced by the ticker's edit steps. */
  edits?: EditInfo[];
  /** The current step is the agent's final message: shown still, no shimmer. */
  finalShown?: boolean;
  /** Cursor sends its answer before `stop`; kept here until then. */
  lastReply?: string;
}

/** One file edit as coucou-hook summarised it. */
export interface EditInfo {
  id: string;
  path: string;
  added: number;
  removed: number;
  /** Changed lines, prefixed "+", "-" or " "; "…" between hunks. */
  lines: string[];
}

export interface ApprovalInfo {
  /** The pill the request belongs to. */
  taskId: string;
  requestId: string;
  sessionId: string;
  tool: string;
  command: string;
  /** Set when this is a Claude Code question rather than a permission. */
  questions?: AskQuestion[];
}

/** One question of Claude Code's AskUserQuestion tool. */
export interface AskQuestion {
  question: string;
  header: string;
  options: { label: string; description: string }[];
  multiSelect: boolean;
}

export interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  content: string;
}

export type PromptContext =
  | { kind: "window"; appName: string; title: string; url?: string }
  | { kind: "file"; name: string; path?: string };

export interface ResultItem {
  label: string;
  detail: string;
  url?: string;
}

export interface SearchResult {
  title: string;
  items: ResultItem[];
  note?: string;
}

const task = (
  id: string, name: string, color: string, source: AgentSource,
): AgentTask => ({
  id, name, color, state: "idle", stepIndex: 0, steps: [], source, isIntegration: true,
});

/** AgentTask.integrationAgents — same ids, names and colours as macOS. */
export const INTEGRATION_AGENTS: AgentTask[] = [
  task("integration_resend", "Resend", "#22C55E", "n8n"),
  task("integration_n8n", "n8n", "#F29B38", "n8n"),
  task("integration_vercel", "Vercel", "#7C5CFF", "n8n"),
  task("integration_github", "GitHub", "#F4505E", "n8n"),
  task("integration_notion", "Notion", "#8C8C8C", "n8n"),
  task("integration_calcom", "Cal.com", "#C9956A", "n8n"),
  task("integration_stripe", "Stripe", "#0570DE", "n8n"),
];

export const TOGGLEABLE_INTEGRATION_IDS = [
  "integration_resend", "integration_n8n", "integration_vercel", "integration_github",
  "integration_notion", "integration_calcom", "integration_stripe",
];

/** What an integration poller last reported. */
export interface IntegrationInfo {
  data: Record<string, unknown>;
  error: string | null;
  loaded: boolean;
  configured: boolean;
}

export interface Settings {
  soundEnabled: boolean;
  soundVolume: number;
  autoCloseInterval: number;
  /** The compact island hides after a minute without the mouse. */
  autoHide: boolean;
  absenceInterval: number;
  activeIntegrations: string[];
  screen: "primary" | "cursor";
  autostart: boolean;
  hooksInstalled: boolean;
  /** Claude model used by the chat. */
  model: string;
  /** Who may answer in the chat; Rust tries Cursor, then Kiro, then the API, until one answers. */
  chatProviders: ("anthropic" | "cursor" | "kiro")[];
  cursorModel: string;
  kiroModel: string;
  /** Cursor and Kiro shell commands wait for Allow/Deny on the island. */
  agentApprovals: boolean;
  /** Where the island was dragged to; null = top centre. Written by Rust only. */
  islandOffset: [number, number] | null;
  /** City for the home card's weather; empty = no weather. */
  weatherCity: string;
  /** Mochi's outfit (see mochi/outfits.ts); "auto" follows the seasons. */
  mochiOutfit: string;
}

export const DEFAULT_SETTINGS: Settings = {
  soundEnabled: true,
  soundVolume: 0.12,
  autoCloseInterval: 15,
  autoHide: true,
  absenceInterval: 180,
  activeIntegrations: [
    "integration_resend", "integration_n8n", "integration_vercel", "integration_github",
  ],
  screen: "primary",
  autostart: false,
  hooksInstalled: false,
  model: "claude-opus-5",
  chatProviders: ["anthropic"],
  cursorModel: "auto",
  kiroModel: "auto",
  agentApprovals: false,
  islandOffset: null,
  weatherCity: "",
  mochiOutfit: "auto",
};

type Listener = () => void;

class AppState {
  mode: IslandMode = "hidden";
  view: IslandViewName = "overview";
  todayTab: TodayTab = "tasks";

  tasks: AgentTask[] = [];
  focusId: string | null = null;

  stateOverride: BotStateName | null = null;

  /** Cursor in logical screen pixels, origin top-left (like AppState.mousePosition). */
  mouse = { x: 0, y: 0 };
  /** Cursor relative to the island's top-left corner. */
  mouseInIsland = { x: 0, y: 0 };

  isPinned = false;
  paused = false;

  uploadProgress = 0;
  uploadDuration = 2.4;
  fileDragOver = false;

  promptContext: PromptContext | null = null;
  droppedFile: { name: string; path: string } | null = null;
  noteMessage: string | null = null;
  searchResult: SearchResult | null = null;
  chatHistory: ChatMessage[] = [];
  pendingApproval: ApprovalInfo | null = null;
  /** The outfit hovered in the wardrobe, worn as a preview until the pointer leaves. */
  wardrobePreview: Worn | null = null;

  integrations: Record<string, IntegrationInfo> = {};

  /** AI usage for the home card; null until first read. */
  usage: {
    claudeTokens: number;
    claudePlan: ClaudePlan | null;
    cursor: CursorPlan | null;
    kiro: KiroPlan | null;
  } | null = null;

  lastActivity = performance.now();

  settings: Settings = { ...DEFAULT_SETTINGS };

  private listeners = new Set<Listener>();

  subscribe(fn: Listener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Marks the UI dirty; the island re-renders on the next frame. */
  notify() {
    for (const fn of this.listeners) fn();
  }

  get focusTask(): AgentTask | null {
    return this.tasks.find((t) => t.id === this.focusId) ?? this.tasks[0] ?? null;
  }

  get effectiveState(): BotStateName {
    return this.stateOverride ?? this.focusTask?.state ?? "idle";
  }

  get otherTasks(): AgentTask[] {
    return this.tasks.filter((t) => t.id !== this.focusId);
  }

  setFocus(id: string) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    this.focusId = id;
    t.pillBadge = null;
    this.notify();
  }

  updateTask(id: string, state: BotStateName) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.state = state;
    this.notify();
  }

  appendStep(id: string, step: string) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.steps.push(step);
    if (t.steps.length > 20) t.steps.shift();
    t.stepIndex = t.steps.length - 1;
    this.notify();
  }

  setPillBadge(id: string, badge: PillBadge | null) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.pillBadge = badge;
    this.notify();
  }

  /** loadIntegrationTasks() — the home card always on, integrations opt-in (max 4). */
  loadIntegrationTasks() {
    if (!this.tasks.some((t) => t.id === HOME_ID)) this.tasks.push({ ...HOME_TASK, steps: [] });
    for (const proto of INTEGRATION_AGENTS) {
      const shouldLoad = this.settings.activeIntegrations.includes(proto.id);
      const idx = this.tasks.findIndex((t) => t.id === proto.id);
      if (shouldLoad && idx < 0) this.tasks.push({ ...proto, steps: [] });
      if (!shouldLoad && idx >= 0) this.tasks.splice(idx, 1);
    }
    // Order: home first, then agent pills (visible in slice(0,4)), the Tasks
    // pill, then integrations in declaration order.
    const order = INTEGRATION_AGENTS.map((t) => t.id);
    const rank = (id: string) =>
      id === HOME_ID ? 0 : isAgentPill(id) ? 1 : id === TODO_PILL_ID ? 2 : 3;
    this.tasks.sort((a, b) =>
      rank(a.id) - rank(b.id) || (rank(a.id) === 3 ? order.indexOf(a.id) - order.indexOf(b.id) : 0),
    );
    if (!this.focusId) this.focusId = HOME_ID;
    this.notify();
  }

  removeTask(id: string) {
    const idx = this.tasks.findIndex((t) => t.id === id);
    if (idx < 0) return;
    this.tasks.splice(idx, 1);
    if (this.focusId === id) this.focusId = HOME_ID;
    this.notify();
  }

  /** Shows the Tasks pill while `open` to-dos are left, labelled with the count. */
  setTodoPill(open: number) {
    const t = this.tasks.find((x) => x.id === TODO_PILL_ID);
    if (open === 0) {
      if (t) this.removeTask(TODO_PILL_ID);
      return;
    }
    const name = open === 1 ? "1 task" : `${open} tasks`;
    if (t) {
      t.name = name;
      return;
    }
    // After home and the agent pills, before the integrations.
    let at = this.tasks.findIndex((x) => x.id === HOME_ID) + 1;
    while (at < this.tasks.length && isAgentPill(this.tasks[at].id)) at++;
    this.tasks.splice(at, 0, {
      id: TODO_PILL_ID, name, color: "#60A5FA",
      state: "idle", stepIndex: 0, steps: [],
      source: "todos", isIntegration: false,
    });
  }

  /** Creates an agent session pill on first event; no-ops if it already exists.
   *  Inserted right after home so it appears in the visible slice(0,4). */
  upsertExternalAgent(id: string, name: string, color: string) {
    if (this.tasks.some((t) => t.id === id)) return;
    const at = this.tasks.findIndex((t) => t.id === HOME_ID) + 1;
    this.tasks.splice(at, 0, {
      id, name, color,
      state: "idle", stepIndex: 0, steps: [],
      source: "agent", isIntegration: false,
    });
    if (!this.focusId) this.focusId = id;
    this.notify();
  }

  toggleIntegration(id: string) {
    if (!TOGGLEABLE_INTEGRATION_IDS.includes(id)) return;
    const active = this.settings.activeIntegrations;
    if (active.includes(id)) {
      this.settings.activeIntegrations = active.filter((x) => x !== id);
      if (this.focusId === id) this.focusId = HOME_ID;
    } else {
      if (active.length >= 4) return;
      this.settings.activeIntegrations = [...active, id];
    }
    this.loadIntegrationTasks();
  }

  defaultView(): IslandViewName {
    return this.tasks.length === 0 ? "empty" : "overview";
  }
}

export const State = new AppState();
