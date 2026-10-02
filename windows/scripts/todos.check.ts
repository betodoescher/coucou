// Self-check for the to-do helpers: node scripts/todos.check.ts
import assert from "node:assert/strict";
import {
  addTodo, compareTodos, dayChoices, dueCount, editTodo, upNext, dueLabel, parseQuickAdd, removeList, toggleTodo, visibleTodos,
  EMPTY_DOC, type TodoItem,
} from "../src/core/todos.ts";

// Friday 2 October 2026, 15:00 local.
const now = new Date(2026, 9, 2, 15, 0);

assert.deepEqual(parseQuickAdd("Send the report !3 #work @tomorrow", now), {
  title: "Send the report", priority: 3, listName: "work", due: "2026-10-03",
});
assert.equal(parseQuickAdd("Pagar conta @amanhã", now)?.due, "2026-10-03");
assert.equal(parseQuickAdd("x @hoje", now)?.due, "2026-10-02");
assert.equal(parseQuickAdd("x @sex", now)?.due, "2026-10-02", "the coming Friday on a Friday is today");
assert.equal(parseQuickAdd("x @seg", now)?.due, "2026-10-05");
assert.equal(parseQuickAdd("x @2026-12-24", now)?.due, "2026-12-24");
assert.deepEqual(parseQuickAdd("Email @fulano about 2026-02-30 @2026-02-30", now), {
  title: "Email @fulano about 2026-02-30 @2026-02-30", priority: 0, listName: null, due: null,
}, "anything that is not a real date stays in the title");
assert.equal(parseQuickAdd("Call !4 mom", now)?.title, "Call !4 mom");
assert.equal(parseQuickAdd("  !2 #home @today ", now), null, "markers alone are not a task");

assert.equal(dueLabel("2026-10-02", now), "Today");
assert.equal(dueLabel("2026-10-03", now), "Tomorrow");
assert.equal(dueLabel("2026-10-01", now), "Yesterday");
assert.equal(dueLabel("2026-10-06", now), "Tue");
assert.equal(dueLabel("2026-11-05", now), "Nov 5");

const base: TodoItem = {
  id: "", title: "t", done: false, doneAt: null, due: null, priority: 0, listId: null, createdAt: 0,
};
const items: TodoItem[] = [
  { ...base, id: "undated-old", createdAt: 1 },
  { ...base, id: "undated-new", createdAt: 2 },
  { ...base, id: "tomorrow", due: "2026-10-03" },
  { ...base, id: "today-low", due: "2026-10-02", priority: 1 },
  { ...base, id: "today-high", due: "2026-10-02", priority: 3 },
  { ...base, id: "overdue", due: "2026-09-30" },
  { ...base, id: "done-early", done: true, doneAt: now.getTime() - 60_000 },
  { ...base, id: "done-late", done: true, doneAt: now.getTime() },
];
assert.deepEqual(
  [...items].sort(compareTodos).map((i) => i.id),
  ["overdue", "today-high", "today-low", "tomorrow", "undated-new", "undated-old", "done-late", "done-early"],
);

// Quick add creates the list once, reuses it case-insensitively, and the rest flows through.
let doc = addTodo(EMPTY_DOC, "Report !2 #Work @hoje", now)!;
doc = addTodo(doc, "Slides #work", now)!;
assert.equal(doc.lists.length, 1);
assert.ok(doc.items.every((i) => i.listId === doc.lists[0].id));
assert.equal(addTodo(doc, "   ", now), null);
const withDefaults = addTodo(EMPTY_DOC, "Plain", now, { listId: "L", due: "2026-10-02" })!;
assert.deepEqual([withDefaults.items[0].listId, withDefaults.items[0].due], ["L", "2026-10-02"]);
assert.equal(addTodo(EMPTY_DOC, "Later @amanha", now, { due: "2026-10-02" })!.items[0].due, "2026-10-03",
  "what the text says wins over the filter");
assert.equal(dueCount(doc, "2026-10-02"), 1);

doc = toggleTodo(doc, doc.items[0].id, now);
assert.equal(dueCount(doc, "2026-10-02"), 0);
assert.equal(visibleTodos(doc, "all", "2026-10-02").length, 2, "ticked today: still shown");
assert.equal(visibleTodos(doc, "all", "2026-10-03").length, 1, "gone the next day");
assert.equal(visibleTodos(doc, "today", "2026-10-02").length, 1);

const firstId = doc.items[0].id;
const edited = editTodo(doc, firstId, { title: "  ", due: "2026-10-09", priority: 1, listId: null });
assert.deepEqual(
  [edited.items[0].title, edited.items[0].due, edited.items[0].priority, edited.items[0].listId],
  [doc.items[0].title, "2026-10-09", 1, null],
  "a blank title keeps the old one",
);
assert.equal(addTodo(EMPTY_DOC, "x", now, { priority: 2 })!.items[0].priority, 2);
assert.equal(addTodo(EMPTY_DOC, "x !3", now, { priority: 2 })!.items[0].priority, 3);
assert.deepEqual(upNext(doc, 5).map((i) => i.title), ["Slides"], "done tasks are not up next");
assert.deepEqual(dayChoices(now).slice(0, 3), [["", "No date"], ["2026-10-02", "Today"], ["2026-10-03", "Tomorrow"]]);

doc = removeList(doc, doc.lists[0].id);
assert.equal(doc.lists.length, 0);
assert.ok(doc.items.every((i) => i.listId === null), "tasks survive their list");

console.log("todos: all checks passed");
