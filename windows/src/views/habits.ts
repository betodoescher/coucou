// Habits panel of the Today tab: a daily check, the last seven days and the streak.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Sound } from "../core/sound";
import { Todos } from "../core/todoStore";
import { addHabit, dayOf, lastDays, removeHabit, streak, toggleHabit, type Habit } from "../core/todos";
import type { TodayPanel } from "./today";

const DONE = "#22C55E";

export function buildHabits(onHeightChange: () => void): TodayPanel {
  let shownRows = 0;
  let rowsKey = "";

  const segSlot = h("div", { class: "today-seg-slot" });
  const input = h("input", {
    type: "text",
    class: "chat-input",
    placeholder: "New habit…",
    spellcheck: "false",
    maxlength: "60",
  }) as HTMLInputElement;
  const addBtn = h("button", { class: "send-btn todo-add", title: "Add" }, svg(ICONS.plus, 12));
  const bar = h("div", { class: "chat-bar todo-bar" }, input, addBtn);
  const error = h("div", { class: "todo-error" });
  const list = h("div", { class: "todo-list" });
  const el = h("div", { class: "todo-body" }, h("div", { class: "todo-top" }, segSlot), bar, error, list);

  async function commit(next: Parameters<typeof Todos.commit>[0]) {
    const err = await Todos.commit(next);
    error.textContent = err ? err.replace(/^Error:\s*/, "") : "";
  }

  function add() {
    const next = addHabit(Todos.doc, input.value, new Date());
    if (!next) {
      input.focus();
      return;
    }
    input.value = "";
    Sound.play("blip");
    void commit(next);
    input.focus();
  }

  addBtn.addEventListener("click", add);
  bar.addEventListener("keydown", (e) => {
    const key = (e as KeyboardEvent).key;
    if (key === "Enter" && e.target === input) {
      e.preventDefault();
      add();
    } else if (key === "Escape") {
      input.value = "";
    }
    e.stopPropagation(); // Escape clears the field, it does not close the island
  });

  function toggle(habit: Habit, day: string) {
    if (!habit.days.includes(day)) Sound.play("pop");
    void commit(toggleHabit(Todos.doc, habit.id, day));
  }

  function row(habit: Habit, now: Date): HTMLElement {
    const today = dayOf(now);
    const doneToday = habit.days.includes(today);
    const run = streak(habit, now);
    // Earlier days stay clickable, to catch up on one that was not ticked.
    const week = h(
      "div",
      { class: "habit-week" },
      ...lastDays(now, 7).slice(0, 6).map((day) =>
        h("button", {
          class: habit.days.includes(day) ? "habit-day on" : "habit-day",
          title: day,
          onclick: () => toggle(habit, day),
        })),
    );
    return h(
      "div",
      { class: "todo-row" },
      h(
        "button",
        {
          class: doneToday ? "todo-check habit-check done" : "todo-check habit-check",
          title: doneToday ? "Not done today" : "Done today",
          style: `border-color:${DONE}`,
          onclick: () => toggle(habit, today),
        },
        doneToday ? svg(ICONS.check, 10, { stroke: 3 }) : null,
      ),
      h("span", { class: "todo-title habit-name", text: habit.name }),
      week,
      h("span", {
        class: run ? "todo-due habit-streak on" : "todo-due habit-streak",
        text: `${run}d`,
        title: run === 1 ? "1 day in a row" : `${run} days in a row`,
      }),
      h(
        "button",
        { class: "todo-x", title: "Delete", onclick: () => void commit(removeHabit(Todos.doc, habit.id)) },
        svg(ICONS.xmark, 9),
      ),
    );
  }

  function sync() {
    const now = new Date();
    const habits = Todos.doc.habits ?? [];
    // Rebuilt only when something changed (or the day turned): redrawing
    // between mousedown and mouseup would swallow the click.
    const rk = `${dayOf(now)}|${JSON.stringify(habits)}`;
    if (rk !== rowsKey) {
      rowsKey = rk;
      clear(list);
      if (habits.length === 0) {
        list.append(h("div", { class: "todo-empty", text: "No habits yet. Add one to tick it every day." }));
      }
      for (const habit of habits) list.append(row(habit, now));
    }

    const rows = Math.max(1, habits.length);
    if (rows !== shownRows) {
      shownRows = rows;
      onHeightChange();
    }
  }

  return {
    el,
    segSlot,
    sync,
    rows: () => shownRows,
    focus() {
      input.focus();
    },
  };
}
