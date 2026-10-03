// Runs the focus timer: one timeout to the end of the phase, and a one-second
// tick for the clock only while a phase runs and the island is on screen.
// Focus ends → the break starts on its own and the island opens; break ends → done.

import { State } from "../core/state";
import { Sound } from "../core/sound";
import { breakFor, pauseRun, remaining, resumeRun, startRun } from "../core/focus";
import type { Island } from "./island";

let island: Island | null = null;
let endTimer = 0;
let tickTimer = 0;

function arm() {
  window.clearTimeout(endTimer);
  window.clearInterval(tickTimer);
  const run = State.focusRun;
  if (island) {
    island.fsm.keepPetit = run !== null;
    if (run && State.mode === "hidden" && !State.paused) island.reveal();
  }
  if (run && run.pausedLeft === null) {
    endTimer = window.setTimeout(finish, remaining(run, Date.now()));
    tickTimer = window.setInterval(() => {
      if (State.mode !== "hidden") State.notify();
    }, 1000);
  }
  State.notify();
}

function finish() {
  const run = State.focusRun;
  if (!run) return;
  if (run.phase === "focus") {
    State.focusRun = startRun("break", breakFor(run.minutes), Date.now());
    Sound.play("finish");
  } else {
    State.focusRun = null;
    Sound.play("greet");
  }
  arm();
  if (island && !State.paused) {
    State.todayTab = "focus";
    island.alert("todos");
  }
}

export const Focus = {
  attach(i: Island) {
    island = i;
  },
  start(minutes: number) {
    State.focusRun = startRun("focus", minutes, Date.now());
    arm();
  },
  pause() {
    if (State.focusRun) State.focusRun = pauseRun(State.focusRun, Date.now());
    arm();
  },
  resume() {
    if (State.focusRun) State.focusRun = resumeRun(State.focusRun, Date.now());
    arm();
  },
  stop() {
    State.focusRun = null;
    arm();
  },
};
