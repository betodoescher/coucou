// Claude Code hook events → island state.
// Port of HookServer.processEvent / processPermissionRequest from the macOS app.
// Difference from macOS: no terminal filter. On Windows the hook fires from any
// terminal (Windows Terminal, VS Code, PowerShell…) and all of them are handled.

import { Bridge, onEvent } from "../core/bridge";
import { Sound } from "../core/sound";
import { CLAUDE_ID, State, type ApprovalInfo, type AskQuestion } from "../core/state";
import type { Island } from "./island";
import { refreshUsage } from "./integrations";
import { EDIT_MARK, baseName, editStep, oneLine } from "../core/steps";

/** Clears the approval card if no decision was made before the hook gave up. */
let pendingTimeout: number | null = null;

interface HookPayload {
  hook_event_name?: string;
  request_id?: string;
  session_id?: string;
  cwd?: string;
  message?: string;
  /** UserPromptSubmit carries `prompt`; `message` belongs to Notification/Stop. */
  prompt?: string;
  tool_name?: string;
  tool_input?: Record<string, unknown>;
  /** Kiro's Stop carries the final answer here. */
  assistant_response?: string;
  /** Cursor's and Kiro's SessionEnd: how the session ended. */
  final_status?: string;
  reason?: string;
  /** Optional agent tag: lowercase, digits and hyphens, ≤ 24 chars. */
  coucou_agent?: string;
  /** A file edit, summarised by coucou-hook. */
  coucou_diff?: { path: string; added: number; removed: number; lines: string[] };
  /** Claude Code's Stop: its last answer. */
  last_assistant_message?: string;
  /** Cursor's afterAgentResponse: the answer. */
  text?: string;
  /** Cursor's postToolUseFailure: "error", "timeout" or "permission_denied". */
  failure_type?: string;
  /** Claude Code's PostToolUseFailure: the user interrupted the tool. */
  is_interrupt?: boolean;
}

function failureStep(payload: HookPayload): string {
  const tool = payload.tool_name ?? "Tool";
  if (payload.failure_type === "permission_denied") return `⚠ ${tool} denied`;
  if (payload.failure_type === "timeout") return `⚠ ${tool} timed out`;
  if (payload.is_interrupt) return `⚠ ${tool} interrupted`;
  return `⚠ ${tool} failed`;
}

/** Edits kept per pill; older ones fall off with their ticker steps. */
const MAX_EDITS = 20;
let editSeq = 0;

function recordEdit(taskId: string, diff: NonNullable<HookPayload["coucou_diff"]>) {
  const task = State.tasks.find((t) => t.id === taskId);
  if (!task || !diff.path) return;
  const id = String(++editSeq);
  task.edits = [...(task.edits ?? []), { id, ...diff }].slice(-MAX_EDITS);
  // The PreToolUse step for this file is already there: it becomes the edit.
  const tail = ` · ${baseName(diff.path)}`;
  let at = -1;
  for (let i = task.steps.length - 1; i >= Math.max(0, task.steps.length - 3); i--) {
    const s = task.steps[i];
    if (!s.startsWith(EDIT_MARK) && s.endsWith(tail)) {
      at = i;
      break;
    }
  }
  if (at >= 0) {
    task.steps[at] = editStep(id);
    State.notify();
  } else {
    State.appendStep(taskId, editStep(id));
  }
}

/** Agents Coucou installs hooks for: their pill name and colour. */
const KNOWN_AGENTS: Record<string, [string, string]> = {
  claude: ["Claude Code", "#F5F6F8"],
  cursor: ["Cursor", "#C0C4CC"],
  kiro: ["Kiro", "#9046FF"],
};

/** Same rule as HookServer.validateAgent on macOS. "claude" is reserved. */
function validateAgent(raw: string | undefined): string | null {
  if (!raw || raw.length > 24 || raw === "claude") return null;
  if (!/^[a-z0-9-]+$/.test(raw)) return null;
  return raw;
}

const FALLBACK_COLORS = ["#22C55E", "#EAB308", "#60A5FA", "#E879F9"];

function agentColor(name: string): string {
  let h = 0;
  for (let i = 0; i < name.length; i++) {
    h = (Math.imul(31, h) + name.charCodeAt(i)) | 0;
  }
  return FALLBACK_COLORS[Math.abs(h) % FALLBACK_COLORS.length];
}

function lastPathComponent(p: string): string {
  const cleaned = p.replace(/[\\/]+$/, "");
  const idx = Math.max(cleaned.lastIndexOf("\\"), cleaned.lastIndexOf("/"));
  return idx >= 0 ? cleaned.slice(idx + 1) : cleaned;
}

/** frenchStep() — same labels as the macOS app. */
const TOOL_LABELS: Record<string, string> = {
  Bash: "Exécute",
  Read: "Lit",
  Write: "Écrit",
  Edit: "Modifie",
  Glob: "Cherche",
  Grep: "Recherche",
  WebSearch: "Recherche web",
  WebFetch: "Récupère",
  TodoWrite: "Tâches",
  Task: "Agent",
  LS: "Liste",
  MultiEdit: "Modifie",
  NotebookEdit: "Notebook",
  PowerShell: "Exécute",
  Shell: "Exécute",
  execute_bash: "Exécute",
  shell: "Exécute",
  fs_read: "Lit",
  fs_write: "Écrit",
};

function stepLabel(tool: string, input: Record<string, unknown>): string {
  const label = TOOL_LABELS[tool] ?? tool;
  const str = (k: string) => (typeof input[k] === "string" ? (input[k] as string) : null);
  // Kiro's fs_* tools use `command` for the operation ("append"), not a shell line.
  const cmd = tool.startsWith("fs_") ? null : str("command");
  if (cmd) return `${label} · ${cmd.slice(0, 40)}`;
  const path = str("path");
  if (path) return `${label} · ${lastPathComponent(path)}`;
  const file = str("file_path");
  if (file) return `${label} · ${lastPathComponent(file)}`;
  const query = str("query");
  if (query) return `${label} · ${query.slice(0, 40)}`;
  return label;
}

/**
 * What the Allow button actually authorises. Approving "Write" tells you nothing
 * — approving `Write · C:\…\.env` tells you everything, and the difference is
 * the whole point of approving from the island rather than blind.
 *
 * Ordered by how specific the field is, so an unfamiliar tool still shows
 * whatever identifying string it carries instead of falling back to its name.
 */
const APPROVAL_FIELDS = [
  "command", // Bash, PowerShell
  "file_path", // Write, Edit, MultiEdit, NotebookEdit
  "path", // Read, LS
  "url", // WebFetch
  "query", // WebSearch
  "pattern", // Glob, Grep
  "prompt", // Task
] as const;

function approvalTarget(tool: string, input: Record<string, unknown>): string {
  for (const field of APPROVAL_FIELDS) {
    const value = input[field];
    if (typeof value === "string" && value.trim()) {
      return `${tool} · ${value.trim()}`;
    }
  }
  return tool;
}

/** Claude Code's question tool, answered on the island through its own hook. */
const QUESTION_TOOL = "AskUserQuestion";

/** 1–4 questions of 2–4 options each, or null: a malformed one goes to the terminal. */
export function parseQuestions(input: Record<string, unknown>): AskQuestion[] | null {
  const raw = input.questions;
  if (!Array.isArray(raw) || raw.length === 0 || raw.length > 4) return null;
  const out: AskQuestion[] = [];
  for (const q of raw as Record<string, unknown>[]) {
    const options = q?.options;
    if (typeof q?.question !== "string" || !q.question || !Array.isArray(options)) return null;
    if (options.length < 2 || options.length > 4) return null;
    const opts = (options as Record<string, unknown>[]).map((o) => ({
      label: typeof o?.label === "string" ? o.label : "",
      description: typeof o?.description === "string" ? o.description : "",
    }));
    if (opts.some((o) => !o.label)) return null;
    out.push({
      question: q.question,
      header: typeof q.header === "string" ? q.header.slice(0, 12) : "",
      options: opts,
      multiSelect: q.multiSelect === true,
    });
  }
  return out;
}

/** Agent pills that already announced the end of this turn. */
const stopped = new Set<string>();
/** Pills waiting to go once their session ended and the island closed. */
const removals = new Map<string, number>();

export function registerHookHandlers(island: Island) {
  void onEvent<HookPayload>("hook", (payload) => handleHook(island, payload));
}

function handleHook(island: Island, payload: HookPayload) {
  if (State.paused) {
    // Silence here used to cost Claude Code nearly two minutes: the relay waited
    // for a decision from an island that had already decided not to look. Say so,
    // and the terminal takes the question immediately.
    if (payload.request_id) void Bridge.approvalDecline(payload.request_id);
    return;
  }

  const name = payload.hook_event_name ?? "";
  const cwd = payload.cwd ?? "";

  // Route to the right pill. Valid coucou_agent → dynamic "agent_<name>" pill.
  // "claude" is reserved; absent or invalid → the Claude Code pill.
  const validAgent = validateAgent(payload.coucou_agent);
  const agentId = validAgent ? `agent_${validAgent}` : CLAUDE_ID;
  /** Cursor/Kiro: their shell commands only wait on the island when approvals are on. */
  const isExternalAgent = validAgent !== null;

  /** Alerts force the island open; work events only reveal the compact island. */
  const surface = (view: Parameters<Island["alert"]>[0], isAlert: boolean) => {
    if (State.mode === "expanded") {
      if (isAlert) island.setView(view);
    } else if (isAlert) {
      island.alert(view);
    } else if (State.mode === "hidden") {
      island.reveal();
    }
  };

  const ensurePill = () => {
    const pending = removals.get(agentId);
    if (pending != null) {
      window.clearTimeout(pending);
      removals.delete(agentId);
    }
    const key = validAgent ?? "claude";
    const [label, color] = KNOWN_AGENTS[key] ?? [key, agentColor(key)];
    State.upsertExternalAgent(agentId, label, color);
    const t = State.tasks.find((x) => x.id === agentId);
    if (t && cwd) t.sessionCwd = cwd;
  };

  /** Open the island on this agent, whatever had the focus. */
  const announce = (kind: "finished" | "error") => {
    ensurePill();
    State.focusId = agentId;
    State.updateTask(agentId, kind);
    State.setPillBadge(agentId, null);
    Sound.play(kind === "error" ? "error" : "finish");
    surface(kind, true);
    window.setTimeout(() => State.updateTask(agentId, "idle"), 5200);
    // The agent writes its usage log right after the turn ends.
    window.setTimeout(() => void refreshUsage(true), 2000);
  };

  /**
   * Puts a request that waits for a human on screen. One card, one request: a
   * second one must never quietly replace the first — that would leave a human
   * staring at request B while request A waits for a decision nobody can give.
   * It goes straight back to the terminal instead.
   */
  const holdCard = (info: ApprovalInfo, view: "approval" | "question", sound: "approval" | "question") => {
    const { requestId } = info;
    if (State.pendingApproval && State.pendingApproval.requestId !== requestId) {
      if (requestId) void Bridge.approvalDecline(requestId);
      return;
    }
    ensurePill();
    if (pendingTimeout != null) window.clearTimeout(pendingTimeout);
    State.pendingApproval = info;
    // The relay's short ack window closes in 800 ms; everything below this
    // line is synchronous, so the card really is up by the time it lands.
    if (requestId) void Bridge.approvalAck(requestId);
    State.updateTask(agentId, view);
    State.isPinned = true;
    Sound.play(sound);
    // Agents always open on their own card, whatever had the focus.
    State.focusId = agentId;
    island.alert(view);
    // Coucou answers within 108 s or not at all; after that the terminal has
    // taken over and the card would be lying.
    pendingTimeout = window.setTimeout(() => {
      pendingTimeout = null;
      if (!State.pendingApproval) return;
      State.pendingApproval = null;
      State.isPinned = false;
      island.dropPin();
      State.updateTask(agentId, "working");
      State.setPillBadge(agentId, null);
      if (State.view === view) island.setView(State.defaultView());
      State.notify();
    }, 110_000);
  };

  if (name === "SessionStart" || name === "UserPromptSubmit") stopped.delete(agentId);
  if (name !== "Stop" && name !== "SessionEnd" && name !== "AfterAgentResponse") {
    const t = State.tasks.find((x) => x.id === agentId);
    if (t) t.finalShown = false;
  }
  if (payload.coucou_diff) {
    ensurePill();
    recordEdit(agentId, payload.coucou_diff);
  }

  switch (name) {
    case "SessionStart":
      ensurePill();
      surface("overview", false);
      Sound.play("work");
      break;

    case "UserPromptSubmit": {
      ensurePill();
      State.updateTask(agentId, "thinking");
      // The field is `prompt`; reading `message` meant this step was always blank.
      const asked = payload.prompt ?? payload.message;
      if (asked) State.appendStep(agentId, asked.slice(0, 60));
      surface("overview", false);
      break;
    }

    case "PreToolUse": {
      ensurePill();
      const tool = payload.tool_name ?? "Tool";
      // The question has its own hook and card; this copy is only noise.
      if (tool === QUESTION_TOOL) break;
      State.updateTask(agentId, "working");
      State.appendStep(agentId, stepLabel(tool, payload.tool_input ?? {}));
      surface("overview", false);
      break;
    }

    case "PostToolUse":
      State.updateTask(agentId, "working");
      break;

    case "PostToolUseFailure":
      State.updateTask(agentId, "working");
      State.appendStep(agentId, failureStep(payload));
      break;

    case "Notification": {
      const message = payload.message ?? "";
      const lower = message.toLowerCase();
      if (lower.includes("rate limit") || lower.includes("limite d")) {
        State.updateTask(agentId, "ratelimit");
        Sound.play("rate");
      } else if (message.endsWith("?")) {
        State.updateTask(agentId, "question");
        State.appendStep(agentId, message);
      }
      break;
    }

    case "AfterAgentResponse": {
      const t = State.tasks.find((x) => x.id === agentId);
      if (t && payload.text) t.lastReply = payload.text;
      break;
    }

    case "Stop": {
      const t = State.tasks.find((x) => x.id === agentId);
      const said = oneLine(
        payload.last_assistant_message ?? payload.assistant_response ?? t?.lastReply ?? payload.message ?? "",
      );
      if (t) t.lastReply = undefined;
      if (said) {
        State.appendStep(agentId, said);
        if (t) t.finalShown = true;
      }
      stopped.add(agentId);
      announce("finished");
      break;
    }

    case "StopFailure":
      stopped.add(agentId);
      announce("error");
      break;

    case "SessionEnd": {
      // `agent -p` sends no Stop: the session's end is the turn's end. A
      // session that did nothing (a composer opened and closed) stays quiet.
      const worked = (State.tasks.find((t) => t.id === agentId)?.steps.length ?? 0) > 0;
      if (!stopped.has(agentId) && worked) {
        const status = `${payload.final_status ?? ""} ${payload.reason ?? ""}`;
        announce(/error|fail/i.test(status) ? "error" : "finished");
      }
      stopped.delete(agentId);
      // The pill goes once the island had time to show how it ended.
      const prev = removals.get(agentId);
      if (prev != null) window.clearTimeout(prev);
      removals.set(agentId, window.setTimeout(() => {
        removals.delete(agentId);
        State.removeTask(agentId);
        State.notify();
      }, (State.settings.autoCloseInterval + 1) * 1000));
      break;
    }

    case "SubagentStart":
      State.appendStep(agentId, "+ subagent");
      break;

    case "SubagentStop":
      State.appendStep(agentId, "• subagent done");
      break;

    case QUESTION_TOOL: {
      const requestId = payload.request_id ?? "";
      const questions = parseQuestions(payload.tool_input ?? {});
      if (isExternalAgent || !questions) {
        if (requestId) void Bridge.approvalDecline(requestId);
        break;
      }
      holdCard(
        {
          taskId: agentId,
          requestId,
          sessionId: payload.session_id ?? "",
          tool: QUESTION_TOOL,
          command: questions[0].question,
          questions,
        },
        "question",
        "question",
      );
      break;
    }

    case "PermissionRequest": {
      const requestId = payload.request_id ?? "";
      const tool = payload.tool_name ?? "Tool";
      const input = payload.tool_input ?? {};
      // Reaching here means the question was sent back to the terminal: it
      // stays there rather than turning into an Allow/Deny card.
      if (tool === QUESTION_TOOL) {
        if (requestId) void Bridge.approvalDecline(requestId);
        break;
      }
      // Cursor and Kiro send every shell command here (the relay renames their
      // events). Kiro has no PreToolUse step for it, so the ticker gets it here;
      // Cursor already sent one. Unless approvals for agents are on, the command
      // runs as the agent decides.
      if (isExternalAgent) {
        ensurePill();
        State.updateTask(agentId, "working");
        if (validAgent !== "cursor") State.appendStep(agentId, stepLabel(tool, input));
        surface("overview", false);
        if (!State.settings.agentApprovals) {
          if (requestId) void Bridge.approvalDecline(requestId);
          break;
        }
      }
      holdCard(
        {
          taskId: agentId,
          requestId,
          sessionId: payload.session_id ?? "",
          tool,
          command: approvalTarget(tool, input),
        },
        "approval",
        "approval",
      );
      break;
    }

    default:
      break;
  }
  State.notify();
}
