// Focus timer (Pomodoro): the run and its pure helpers.
// No imports, so scripts/todos.check.ts can run it under plain Node.

export type FocusPhase = "focus" | "break";

export interface FocusRun {
  phase: FocusPhase;
  minutes: number;
  /** Epoch ms the phase ends; meaningless while paused. */
  endsAt: number;
  /** Ms left when paused, null while running. */
  pausedLeft: number | null;
}

export const FOCUS_PRESETS = [15, 25, 50];

/** 5 minutes of break per 25 of focus, at least 5. */
export function breakFor(minutes: number): number {
  return Math.max(5, Math.round(minutes / 5));
}

export function startRun(phase: FocusPhase, minutes: number, now: number): FocusRun {
  return { phase, minutes, endsAt: now + minutes * 60_000, pausedLeft: null };
}

export function remaining(run: FocusRun, now: number): number {
  return Math.max(0, run.pausedLeft ?? run.endsAt - now);
}

export function pauseRun(run: FocusRun, now: number): FocusRun {
  return run.pausedLeft !== null ? run : { ...run, pausedLeft: remaining(run, now) };
}

export function resumeRun(run: FocusRun, now: number): FocusRun {
  return run.pausedLeft === null ? run : { ...run, endsAt: now + run.pausedLeft, pausedLeft: null };
}

/** "24:05"; seconds round up so the clock reads 25:00 at the start and 0:01 at the end. */
export function clockLabel(ms: number): string {
  const s = Math.ceil(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}
