// The Today tab: tasks, habits, notes and the focus timer behind one segmented switch, in the same card.

import { h } from "./dom";
import { Sound } from "../core/sound";
import { State, type TodayTab } from "../core/state";
import { buildTodos } from "./todos";
import { buildNotes } from "./notes";
import { buildHabits } from "./habits";
import { buildFocusPanel } from "./focusPanel";
import type { ViewHost } from "./views";

/** A panel of the Today card; the switch is placed in its `segSlot` while it is shown. */
export interface TodayPanel {
  el: HTMLElement;
  segSlot: HTMLElement;
  sync(): void;
  /** Rows it shows, for the island's height. */
  rows(): number;
  focus(): void;
}

const TABS: [TodayTab, string][] = [["tasks", "Tasks"], ["habits", "Habits"], ["notes", "Notes"], ["focus", "Focus"]];

let rowsOf = () => 0;

/** Rows the open panel shows, for the island's height. */
export function todayRowCount(): number {
  return rowsOf();
}

export function buildToday(onHeightChange: () => void): ViewHost {
  const panels: Record<TodayTab, TodayPanel> = {
    tasks: buildTodos(onHeightChange),
    habits: buildHabits(onHeightChange),
    notes: buildNotes(onHeightChange),
    focus: buildFocusPanel(),
  };
  rowsOf = () => panels[State.todayTab].rows();

  const seg = h("div", { class: "today-seg" });
  const buttons = TABS.map(([id, label]) => {
    const b = h("button", {
      class: "today-seg-btn",
      text: label,
      onclick: () => {
        if (State.todayTab === id) return;
        Sound.play("blip");
        State.todayTab = id;
        sync();
        panels[id].focus();
        State.notify();
      },
    });
    seg.append(b);
    return [id, b] as const;
  });

  let shown: TodayTab | null = null;
  function sync() {
    const tab = State.todayTab;
    if (tab !== shown) {
      shown = tab;
      for (const [id, p] of Object.entries(panels)) p.el.style.display = id === tab ? "" : "none";
      for (const [id, b] of buttons) b.classList.toggle("on", id === tab);
      panels[tab].segSlot.append(seg);
      onHeightChange();
    }
    panels[tab].sync();
  }

  return {
    el: h("div", { class: "view" }, h("div", { class: "card todo-card" }, ...Object.values(panels).map((p) => p.el))),
    sync,
    focus() {
      panels[State.todayTab].focus();
    },
  };
}
