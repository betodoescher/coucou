// Thin wrapper over the Tauri commands/events. Every call is a no-op when the
// page is opened in a plain browser, so the island can be iterated on with
// `npm run dev` alone.

import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Settings } from "./state";
import type { TodoDoc } from "./todos";

export const IS_TAURI =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!IS_TAURI) return null;
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    console.error(`[coucou] ${cmd} failed`, err);
    return null;
  }
}

/** Cursor's `/usage`: percents of the monthly plan. */
export interface CursorPlan {
  plan: string;
  percent: number;
  auto: number | null;
  api: number | null;
  resets: string;
}

/** Kiro's `/usage`: credits of the monthly plan. */
export interface KiroPlan {
  plan: string;
  used: number;
  limit: number;
  resets: string;
}

/** Claude plan limits from Claude Code's status line. */
export interface PlanWindow {
  usedPct: number;
  /** Unix seconds. */
  resetsAt: number;
}

export interface ClaudePlan {
  fiveHour: PlanWindow | null;
  sevenDay: PlanWindow | null;
  /** Unix milliseconds. */
  updatedAt: number;
}

export interface BootInfo {
  settings: Settings;
  /** Logical screen rect of the monitor the island lives on. */
  screen: { x: number; y: number; width: number; height: number; scale: number };
  version: string;
  hookPath: string;
  /** False where the OS has no global cursor (Wayland): see Island.followPageCursor. */
  cursorPoll: boolean;
  /** False where the compositor pins the island to the top edge (layer-shell). */
  islandMovable: boolean;
  /** Mochi can be dragged out onto the desktop. */
  desktopMochi: boolean;
}

export const Bridge = {
  boot: () => call<BootInfo>("boot"),

  saveSettings: (settings: Settings) => call<void>("save_settings", { settings }),

  /** Shrink the window down to the invisible wake strip (hidden) or back to full. */
  setCollapsed: (collapsed: boolean) => call<void>("set_collapsed", { collapsed }),

  /**
   * Pushes the island shape in window coordinates. Rust flips click-through from
   * its own cursor poll, so the flag is never a frame behind a click.
   */
  setIslandRect: (x: number, y: number, width: number, height: number) =>
    call<void>("set_island_rect", { x, y, width, height }),

  /** Give the window keyboard focus (chat field) and take it away again. */
  focusWindow: (focused: boolean) => call<void>("focus_window", { focused }),

  reposition: () => call<void>("reposition"),

  /** One drag step, in logical pixels. */
  moveIsland: (dx: number, dy: number) => call<void>("move_island", { dx, dy }),
  /** End of a drag: Rust remembers where the island now is. */
  saveIslandPosition: () => call<void>("save_island_position"),
  /** Back to the top centre of the display. */
  resetIslandPosition: () => call<void>("reset_island_position"),

  openUrl: (url: string) => call<void>("open_url", { url }),

  /** "Open terminal" → opens the folder in VS Code when `code` is on PATH. */
  openInVSCode: (path: string | null) => call<boolean>("open_in_vscode", { path }),

  quit: () => call<void>("quit_app"),

  openSettingsWindow: () => call<void>("open_settings_window"),

  /** Writes to %LOCALAPPDATA%\Coucou\coucou.log, next to the Rust lines. */
  log: (message: string) => call<void>("log_line", { message }),

  // ── Agent hooks (Claude Code, Cursor, Kiro) ───────────────────────────────
  hooksStatus: (target: HookTarget = "claude") => call<HookStatus>("hooks_status", { target }),
  /** Diff to show before anything is written. `install: false` previews removal. */
  hooksPreview: (install: boolean, target: HookTarget = "claude") =>
    callOrThrow<HookPreview>("hooks_preview", { target, install }),
  /**
   * Writes the agent's hook file — only ever after an explicit click, and only
   * when the file still matches the preview the user looked at.
   */
  hooksApply: (install: boolean, fingerprint: string, target: HookTarget = "claude") =>
    callOrThrow<string>("hooks_apply", { target, install, fingerprint }),

  approvalDecision: (requestId: string, decision: "allow" | "deny") =>
    call<void>("approval_decision", { requestId, decision }),
  /** Answers to a Claude Code question: a label, or labels for a multi-select, per question text. */
  questionAnswer: (requestId: string, answers: Record<string, string | string[]>) =>
    call<void>("question_answer", { requestId, answers }),
  /** "The card is up" — until this lands the relay only waits a moment. */
  approvalAck: (requestId: string) => call<void>("approval_ack", { requestId }),
  /** "Nobody can act on this" — Claude Code asks in the terminal right away. */
  approvalDecline: (requestId: string) => call<void>("approval_decline", { requestId }),

  // ── Chat, files, secrets ──────────────────────────────────────────────────
  /** One chat turn. The API key and any file bytes never leave Rust. */
  chatSend: (query: string, context: ChatContext | null) =>
    callOrThrow<{ text: string }>("chat_send", { query, context }),
  chatReset: () => call<void>("chat_reset"),
  /** Whether the Cursor CLI is installed and signed in. */
  cursorStatus: () => call<CursorStatus>("cursor_status"),
  /** [id, label] pairs the user's Cursor plan can use. */
  cursorModels: () => call<[string, string][]>("cursor_models"),
  /** Opens Cursor's browser sign-in. */
  cursorLogin: () => callOrThrow<void>("cursor_login"),
  /** Whether the Kiro CLI is installed and signed in. */
  kiroStatus: () => call<CursorStatus>("kiro_status"),
  /** [id, label] pairs the user's Kiro plan can use. */
  kiroModels: () => call<[string, string][]>("kiro_models"),
  todosLoad: () => call<TodoDoc>("todos_load"),
  /** Validated and written atomically by Rust, then sent to every window as "todos-changed". */
  todosSave: (doc: TodoDoc) => callOrThrow<void>("todos_save", { doc }),
  /** Copies a dropped file into the inbox. */
  ingestFile: (path: string) => callOrThrow<DroppedFile>("ingest_file", { path }),
  /** Only ever tells you whether a key exists — never its value. */
  secretPresent: (key: string) => call<boolean>("secret_present", { key }),
  secretSet: (key: string, value: string) => callOrThrow<void>("secret_set", { key, value }),
  secretClear: (key: string) => callOrThrow<void>("secret_clear", { key }),

  // ── Integrations ──────────────────────────────────────────────────────────
  refreshIntegration: (id: string) => call<void>("refresh_integration", { id }),
  /** Claude tokens since `sinceMs`, from its local logs. */
  usageToday: (sinceMs: number) =>
    call<{ claudeTokens: number; claudePlan: ClaudePlan | null }>("usage_today", { sinceMs }),
  /** Cursor and Kiro plan usage from their CLIs' `/usage`; takes ~15 s. */
  planUsage: () => call<{ cursor: CursorPlan | null; kiro: KiroPlan | null }>("plan_usage"),
  /** Opens the configured n8n instance in the browser. */
  openN8n: () => call<void>("open_n8n"),

  /** Tray → Pause. Stops the integration pollers, not just the island. */
  setPaused: (paused: boolean) => call<void>("set_paused", { paused }),

  // ── Mochi on the desktop ──────────────────────────────────────────────────
  /** Mochi grabbed; `x`/`y` are the pointer's screen coordinates. */
  desktopDragStart: (x: number, y: number) => call<void>("desktop_mochi_drag_start", { x, y }),
  /** Only used where Rust cannot follow the cursor itself (Linux). */
  desktopDrag: (x: number, y: number) => call<void>("desktop_mochi_drag", { x, y }),
  desktopDrop: () => call<void>("desktop_mochi_drop"),
  /** `out`: from the island to his spot; otherwise into the island for an alert. */
  desktopFly: (out: boolean) => call<void>("desktop_mochi_fly", { out }),
  /** Tells the desktop Mochi's page something (state, emote). */
  toDesktop: (event: string, payload: unknown) => {
    if (IS_TAURI) void emitTo("mochi", event, payload).catch(() => {});
  },
};

/** What desktop.rs reports whenever the desktop Mochi changes. */
export interface DesktopEvent {
  onDesktop: boolean;
  visible: boolean;
  landed: boolean;
}

export interface IntegrationUpdate {
  id: string;
  data: Record<string, unknown>;
  error: string | null;
  /** `alert`: open the island on it rather than only badging the pill. */
  event: { success: boolean; label: string; detail: string | null; alert?: boolean } | null;
}

export type ChatContext =
  | { kind: "file"; name: string; path: string }
  | { kind: "window"; appName: string; title: string; url?: string };

export interface DroppedFile {
  name: string;
  path: string;
  size: number;
}

export interface CursorStatus {
  /** Path of the CLI, or null when it is not installed. */
  cli: string | null;
  status: string;
  loggedIn: boolean;
}

export type HookTarget = "claude" | "cursor" | "kiro";

export interface HookStatus {
  installed: boolean;
  /** Installed, but missing an entry this version adds. */
  outdated: boolean;
  settingsPath: string;
  hookPath: string;
  hookReady: boolean;
}

export interface HookPreview {
  diff: string;
  backup: string;
  settingsPath: string;
  /** Hand back to hooksApply so only the reviewed diff is ever written. */
  fingerprint: string;
}

/** Same as `call`, but surfaces the error so the UI can show what went wrong. */
async function callOrThrow<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!IS_TAURI) throw new Error("not running inside Coucou");
  return invoke<T>(cmd, args);
}

export type BridgeEvent =
  | { name: "cursor"; payload: { x: number; y: number } }
  | { name: "tray"; payload: string }
  | { name: "hook"; payload: Record<string, unknown> }
  | { name: "screen-changed"; payload: null };

export interface DragDropPayload {
  type: "enter" | "over" | "drop" | "leave";
  paths?: string[];
}

/** Files dragged onto the island. Only reaches us when the window takes the mouse. */
export async function onDragDrop(handler: (e: DragDropPayload) => void) {
  if (!IS_TAURI) return () => {};
  return getCurrentWebview().onDragDropEvent((event) => {
    handler(event.payload as DragDropPayload);
  });
}

export async function onEvent<T>(name: string, handler: (payload: T) => void) {
  if (!IS_TAURI) return () => {};
  return listen<T>(name, (e) => handler(e.payload));
}
