// Quick notes panel of the Today tab: a field to jot one down, and the notes, last touched first.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Sound } from "../core/sound";
import { Todos } from "../core/todoStore";
import { addNote, agoLabel, editNote, removeNote, sortedNotes, type Note } from "../core/todos";
import type { TodayPanel } from "./today";

export function buildNotes(onHeightChange: () => void): TodayPanel {
  let shownRows = 0;
  let editing: string | null = null;
  let rowsKey = "";

  const segSlot = h("div", { class: "today-seg-slot" });
  const input = h("input", {
    type: "text",
    class: "chat-input",
    placeholder: "Write a note…",
    spellcheck: "false",
    maxlength: "5000",
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
    const next = addNote(Todos.doc, input.value, new Date());
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

  function editor(note: Note): HTMLElement {
    const text = h("textarea", {
      class: "todo-edit note-edit",
      rows: "3",
      maxlength: "5000",
      spellcheck: "false",
    }) as HTMLTextAreaElement;
    text.value = note.text;
    const close = () => {
      editing = null;
      rowsKey = "";
      sync();
      input.focus();
    };
    const save = () => {
      editing = null;
      rowsKey = "";
      void commit(editNote(Todos.doc, note.id, text.value, new Date()));
      input.focus();
    };
    const box = h(
      "div",
      { class: "todo-editor" },
      text,
      h(
        "div",
        { class: "todo-editor-row" },
        h("span", { class: "note-hint", text: "Shift+Enter for a new line" }),
        h("div", { class: "grow" }),
        h("button", { class: "todo-text-btn", text: "Cancel", onclick: close }),
        h("button", { class: "todo-text-btn primary", text: "Save", onclick: save }),
      ),
    );
    box.addEventListener("keydown", (e) => {
      const k = e as KeyboardEvent;
      if (k.key === "Enter" && !k.shiftKey && e.target === text) {
        e.preventDefault();
        save();
      } else if (k.key === "Escape") {
        close();
      }
      e.stopPropagation();
    });
    requestAnimationFrame(() => text.focus());
    return box;
  }

  function row(note: Note, now: Date): HTMLElement {
    if (editing === note.id) return editor(note);
    return h(
      "div",
      { class: "todo-row" },
      h("button", {
        class: "todo-title",
        text: note.text.replace(/\s*\n\s*/g, " · "),
        title: note.text,
        onclick: () => {
          editing = note.id;
          rowsKey = "";
          sync();
        },
      }),
      h("span", { class: "todo-due", text: agoLabel(note.updatedAt, now) }),
      h(
        "button",
        { class: "todo-x", title: "Delete", onclick: () => void commit(removeNote(Todos.doc, note.id)) },
        svg(ICONS.xmark, 9),
      ),
    );
  }

  function sync() {
    const now = new Date();
    const notes = sortedNotes(Todos.doc);
    if (editing && !notes.some((n) => n.id === editing)) editing = null;

    // Rebuilt only when something changed (or a minute passed, for the ages):
    // redrawing between mousedown and mouseup would swallow the click.
    const rk = `${editing}|${Math.floor(now.getTime() / 60_000)}|${JSON.stringify(notes)}`;
    if (rk !== rowsKey) {
      rowsKey = rk;
      clear(list);
      if (notes.length === 0) {
        list.append(h("div", { class: "todo-empty", text: "No notes yet. Type one above and press Enter." }));
      }
      for (const n of notes) list.append(row(n, now));
    }

    // The editor's three lines take about two rows more.
    const rows = Math.max(1, notes.length) + (editing ? 2 : 0);
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
