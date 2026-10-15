// The to-do document of this window, kept in step with Rust and the other window.

import { Bridge, onEvent } from "./bridge";
import { EMPTY_DOC, type TodoDoc } from "./todos";

type Listener = () => void;

let doc: TodoDoc = EMPTY_DOC;
const listeners = new Set<Listener>();

function set(next: TodoDoc) {
  doc = next;
  for (const fn of listeners) fn();
}

export const Todos = {
  get doc(): TodoDoc {
    return doc;
  },

  subscribe(fn: Listener) {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },

  /** Loads the saved list and follows changes made in the other window. */
  async init() {
    await onEvent<TodoDoc>("todos-changed", set);
    set((await Bridge.todosLoad()) ?? EMPTY_DOC);
  },

  /** Shows the change at once and saves it; on failure puts the previous list back. */
  async commit(next: TodoDoc): Promise<string | null> {
    const previous = doc;
    set(next);
    try {
      await Bridge.todosSave(next);
      return null;
    } catch (err) {
      set(previous);
      return String(err);
    }
  },
};
