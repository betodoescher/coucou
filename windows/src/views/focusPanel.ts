// Focus panel of the Today tab: pick a length, start, pause, stop; the clock ticks here and on the compact island.

import { h, clear } from "./dom";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import { FOCUS_PRESETS, clockLabel, remaining } from "../core/focus";
import { Focus } from "../island/focusTimer";
import type { TodayPanel } from "./today";

export function buildFocusPanel(): TodayPanel {
  let minutes = 25;
  let controlsKey = "";

  const segSlot = h("div", { class: "today-seg-slot" });
  const clock = h("div", { class: "focus-clock" });
  const label = h("div", { class: "focus-label" });
  const controls = h("div", { class: "focus-controls" });
  const el = h(
    "div",
    { class: "todo-body" },
    h("div", { class: "todo-top" }, segSlot),
    h("div", { class: "focus-main" }, h("div", { class: "focus-face" }, clock, label), controls),
  );

  const act = (fn: () => void) => () => {
    Sound.play("blip");
    fn();
  };
  const textBtn = (text: string, fn: () => void, primary = false) =>
    h("button", { class: primary ? "todo-text-btn primary" : "todo-text-btn", text, onclick: act(fn) });

  function sync() {
    const run = State.focusRun;
    const now = Date.now();
    clock.textContent = clockLabel(run ? remaining(run, now) : minutes * 60_000);
    clock.classList.toggle("break", run?.phase === "break");
    label.textContent =
      !run ? "Ready to focus"
      : run.phase === "break" ? "Break · stretch a little"
      : run.pausedLeft !== null ? `Paused · ${run.minutes} min`
      : `Focus · ${run.minutes} min`;

    // Rebuilt only when the buttons change, never on the clock's tick:
    // redrawing between mousedown and mouseup would swallow the click.
    const key = run ? `${run.phase}|${run.pausedLeft !== null}` : `idle|${minutes}`;
    if (key === controlsKey) return;
    controlsKey = key;
    clear(controls);
    if (!run) {
      for (const m of FOCUS_PRESETS) {
        controls.append(h("button", {
          class: m === minutes ? "todo-chip on" : "todo-chip",
          text: `${m} min`,
          onclick: act(() => {
            minutes = m;
            sync();
          }),
        }));
      }
      controls.append(textBtn("Start", () => Focus.start(minutes), true));
    } else if (run.phase === "break") {
      controls.append(textBtn("Skip break", () => Focus.stop()));
    } else {
      controls.append(
        run.pausedLeft !== null ? textBtn("Resume", () => Focus.resume(), true) : textBtn("Pause", () => Focus.pause()),
        textBtn("Stop", () => Focus.stop()),
      );
    }
  }

  return {
    el,
    segSlot,
    sync,
    rows: () => 2,
    focus() {},
  };
}
