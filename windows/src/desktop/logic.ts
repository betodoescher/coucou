// The desktop Mochi's decisions that need no window: when to sleep, and the
// alert cycle (fly to the island for a permission or a question, come back
// once it is answered). Same rules as DesktopMochiLogic.swift.

import type { BotStateName } from "../core/layout";

/** Two minutes without anything going on… */
export const SLEEP_AFTER_S = 120;
/** …and the cursor at least this far from his body. */
export const SLEEP_MOUSE_DISTANCE = 150;

export interface DesktopState {
  state: BotStateName;
  /** Seconds since the last agent activity, click or emote. */
  idleFor: number;
  /** Cursor distance to his body; Infinity when it cannot be known. */
  mouseDistance: number;
}

export function shouldSleep(s: DesktopState): boolean {
  const quiet = s.state === "idle" || s.state === "sleeping";
  return quiet && s.idleFor > SLEEP_AFTER_S && s.mouseDistance >= SLEEP_MOUSE_DISTANCE;
}

/** What the island has to do with the desktop Mochi when something changes. */
export type DesktopAction = "retract" | "fly-out" | null;

export interface AlertInput {
  /** The user put Mochi on the desktop. */
  onDesktop: boolean;
  /** He is at the island right now, for an alert. */
  atIsland: boolean;
  /** A permission or a question waits for an answer. */
  alert: boolean;
}

/**
 * A new alert sends him to the island; once nothing waits any more he flies
 * back to his spot.
 */
export function alertAction(s: AlertInput): DesktopAction {
  if (!s.onDesktop) return null;
  if (s.alert && !s.atIsland) return "retract";
  if (!s.alert && s.atIsland) return "fly-out";
  return null;
}
