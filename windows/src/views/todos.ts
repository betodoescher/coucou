// To-do view: filter chips, the add form (title, day, priority) and the list.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Sound } from "../core/sound";
import { Todos } from "../core/todoStore";
import {
  PRIORITY_COLORS, addTodo, dayChoices, dayOf, dueCount, dueLabel, editTodo, isOverdue,
  removeTodo, toggleTodo, visibleTodos, whenLabel, type Priority, type TodoFields, type TodoFilter, type TodoItem,
} from "../core/todos";
import type { ViewHost } from "./views";

let shownRows = 0;

/** Rows the view shows, for the island's height. */
export function todoRowCount(): number {
  return shownRows;
}

const PRIORITY_LABELS: [Priority, string][] = [[0, "No priority"], [1, "Low"], [2, "Medium"], [3, "High"]];

/** The day and priority pickers, shared by the add form and the editor. */
function fieldPickers(initial: Omit<TodoFields, "title">) {
  const now = new Date();
  const day = h("select", { class: "todo-select", title: "Due day" }) as HTMLSelectElement;
  const choices = dayChoices(now);
  if (initial.due && !choices.some(([v]) => v === initial.due)) choices.push([initial.due, dueLabel(initial.due, now)]);
  for (const [v, label] of choices) day.append(h("option", { value: v, text: label }));
  day.append(h("option", { value: "pick", text: "Pick a date…" }));
  day.value = initial.due ?? "";

  // Shown only for "Pick a date…": the native date field.
  const picker = h("input", { type: "date", class: "todo-select todo-date" }) as HTMLInputElement;
  picker.style.display = "none";
  day.addEventListener("change", () => {
    picker.style.display = day.value === "pick" ? "" : "none";
    if (day.value === "pick") picker.focus();
  });
  picker.addEventListener("change", () => {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(picker.value)) return;
    if (![...day.options].some((o) => o.value === picker.value)) {
      day.insertBefore(h("option", { value: picker.value, text: dueLabel(picker.value, new Date()) }), day.lastChild);
    }
    day.value = picker.value;
    picker.style.display = "none";
  });

  // A time only makes sense on a day, so it hides without one.
  const time = h("input", { type: "time", class: "todo-select todo-time", title: "Remind me at" }) as HTMLInputElement;
  const paintTime = () => {
    time.style.display = day.value && day.value !== "pick" ? "" : "none";
  };
  day.addEventListener("change", paintTime);
  picker.addEventListener("change", paintTime);

  const prio = h("select", { class: "todo-select", title: "Priority" }) as HTMLSelectElement;
  for (const [v, label] of PRIORITY_LABELS) prio.append(h("option", { value: String(v), text: label }));
  const paintPrio = () => {
    const p = Number(prio.value) as Priority;
    prio.style.color = p ? PRIORITY_COLORS[p] : "";
  };
  prio.addEventListener("change", paintPrio);

  // No list picker: the list comes from the open filter or `#name`.
  let listId = initial.listId;

  const set = (f: Omit<TodoFields, "title">) => {
    day.value = f.due ?? "";
    picker.style.display = "none";
    time.value = f.time ?? "";
    prio.value = String(f.priority);
    listId = f.listId;
    paintPrio();
    paintTime();
  };
  set(initial);

  return {
    els: [day, picker, time, prio] as HTMLElement[],
    read: (): Omit<TodoFields, "title"> => {
      const due = day.value && day.value !== "pick" ? day.value : null;
      return {
        due,
        time: due && /^\d{2}:\d{2}$/.test(time.value) ? time.value : null,
        priority: Number(prio.value) as Priority,
        listId,
      };
    },
    set,
  };
}

export function buildTodos(onHeightChange: () => void): ViewHost {
  let filter: TodoFilter = "all";
  let editing: string | null = null;
  let rowsKey = "";
  let chipsKey = "";

  const chips = h("div", { class: "todo-chips" });
  const input = h("input", {
    type: "text",
    class: "chat-input",
    placeholder: "Add a task…",
    spellcheck: "false",
    maxlength: "500",
  }) as HTMLInputElement;
  const addBtn = h("button", { class: "send-btn todo-add", title: "Add" }, svg(ICONS.plus, 12));
  const pickersSlot = h("div", { class: "todo-pickers" });
  const bar = h("div", { class: "chat-bar todo-bar" }, input, pickersSlot, addBtn);
  const error = h("div", { class: "todo-error" });
  const list = h("div", { class: "todo-list" });

  const el = h(
    "div",
    { class: "view" },
    h("div", { class: "card todo-card" }, h("div", { class: "todo-body" }, chips, bar, error, list)),
  );

  /** What a new task gets from the filter in use. */
  const filterDefaults = (): Omit<TodoFields, "title"> => ({
    due: filter === "today" ? dayOf(new Date()) : null,
    time: null,
    priority: 0,
    listId: filter !== "all" && filter !== "today" ? filter : null,
  });

  let addPickers = fieldPickers(filterDefaults());
  let pickersKey = "";
  function syncAddPickers() {
    // Rebuilt when the lists or the filter change; otherwise left alone so a
    // choice in progress is not reset.
    const k = `${filter}|${Todos.doc.lists.map((l) => l.id + l.name).join(",")}|${dayOf(new Date())}`;
    if (k === pickersKey) return;
    pickersKey = k;
    addPickers = fieldPickers(filterDefaults());
    clear(pickersSlot);
    pickersSlot.append(...addPickers.els);
  }

  async function commit(next: Parameters<typeof Todos.commit>[0]) {
    const err = await Todos.commit(next);
    error.textContent = err ? err.replace(/^Error:\s*/, "") : "";
  }

  function add() {
    // Markers typed in the title (!3 #list @friday) win over the pickers.
    const next = addTodo(Todos.doc, input.value, new Date(), addPickers.read());
    if (!next) {
      input.focus();
      return;
    }
    input.value = "";
    addPickers.set(filterDefaults());
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

  function chip(id: TodoFilter, label: string, color?: string): HTMLElement {
    return h(
      "button",
      {
        class: filter === id ? "todo-chip on" : "todo-chip",
        onclick: () => {
          filter = id;
          rowsKey = chipsKey = "";
          Sound.play("blip");
          sync();
          input.focus();
        },
      },
      color ? h("i", { class: "todo-dot", style: `background:${color}` }) : null,
      h("span", { text: label }),
    );
  }

  function editor(item: TodoItem): HTMLElement {
    const title = h("input", {
      type: "text",
      class: "todo-edit",
      value: item.title,
      maxlength: "500",
      spellcheck: "false",
    }) as HTMLInputElement;
    const pickers = fieldPickers({ ...item, time: item.time ?? null });
    const close = () => {
      editing = null;
      rowsKey = "";
      sync();
      input.focus();
    };
    const save = () => {
      editing = null;
      rowsKey = "";
      void commit(editTodo(Todos.doc, item.id, { title: title.value, ...pickers.read() }));
      input.focus();
    };
    const box = h(
      "div",
      { class: "todo-editor" },
      title,
      h(
        "div",
        { class: "todo-editor-row" },
        ...pickers.els,
        h("div", { class: "grow" }),
        h("button", { class: "todo-text-btn", text: "Cancel", onclick: close }),
        h("button", { class: "todo-text-btn primary", text: "Save", onclick: save }),
      ),
    );
    box.addEventListener("keydown", (e) => {
      const key = (e as KeyboardEvent).key;
      if (key === "Enter" && e.target === title) save();
      else if (key === "Escape") close();
      e.stopPropagation();
    });
    requestAnimationFrame(() => {
      title.focus();
      title.select();
    });
    return box;
  }

  function row(item: TodoItem, today: string, now: Date): HTMLElement {
    if (editing === item.id) return editor(item);
    const listColor = Todos.doc.lists.find((l) => l.id === item.listId)?.color;
    const check = h(
      "button",
      {
        class: item.done ? "todo-check done" : "todo-check",
        title: item.done ? "Mark as not done" : "Done",
        style: `border-color:${item.priority ? PRIORITY_COLORS[item.priority] : "var(--dim-3)"}`,
        onclick: () => {
          if (!item.done) Sound.play("pop");
          void commit(toggleTodo(Todos.doc, item.id, new Date()));
        },
      },
      item.done ? svg(ICONS.check, 10, { stroke: 3 }) : null,
    );
    return h(
      "div",
      { class: item.done ? "todo-row done" : "todo-row" },
      check,
      h("button", {
        class: "todo-title",
        text: item.title,
        title: "Edit",
        onclick: () => {
          editing = item.id;
          rowsKey = "";
          sync();
        },
      }),
      listColor && filter !== item.listId ? h("i", { class: "todo-dot", style: `background:${listColor}` }) : null,
      item.due
        ? h("span", {
            class: isOverdue(item, today) ? "todo-due late" : "todo-due",
            text: whenLabel(item, now),
          })
        : null,
      h(
        "button",
        {
          class: "todo-x",
          title: "Delete",
          onclick: () => void commit(removeTodo(Todos.doc, item.id)),
        },
        svg(ICONS.xmark, 9),
      ),
    );
  }

  function sync() {
    const now = new Date();
    const today = dayOf(now);
    const doc = Todos.doc;
    if (filter !== "all" && filter !== "today" && !doc.lists.some((l) => l.id === filter)) filter = "all";
    if (editing && !doc.items.some((i) => i.id === editing)) editing = null;

    const due = dueCount(doc, today);
    const ck = `${filter}|${due}|${doc.lists.map((l) => l.id + l.name + l.color).join(",")}`;
    if (ck !== chipsKey) {
      chipsKey = ck;
      clear(chips);
      chips.append(chip("all", "All"), chip("today", due ? `Today · ${due}` : "Today"));
      for (const l of doc.lists) chips.append(chip(l.id, l.name, l.color));
    }
    syncAddPickers();

    // Rebuilt only when something changed: redrawing between mousedown and
    // mouseup would swallow the click, and would reset the editor.
    const items = visibleTodos(doc, filter, today);
    const rk = `${filter}|${today}|${editing}|${JSON.stringify(items)}|${JSON.stringify(doc.lists)}`;
    if (rk !== rowsKey) {
      rowsKey = rk;
      clear(list);
      if (items.length === 0) {
        list.append(h("div", {
          class: "todo-empty",
          text: filter === "today" ? "Nothing due today." : "No tasks yet. Type one above and press Enter.",
        }));
      }
      for (const item of items) list.append(row(item, today, now));
    }

    const rows = Math.max(1, items.length) + (editing ? 1 : 0);
    if (rows !== shownRows) {
      shownRows = rows;
      onHeightChange();
    }
  }

  return {
    el,
    sync,
    focus() {
      input.focus();
    },
  };
}
