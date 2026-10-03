// The to-do list: data model and pure helpers (quick add, sorting, labels).
// No imports, so scripts/todos.check.ts can run it under plain Node.
// "todo" everywhere because "task" already means an agent session.

export interface TodoList {
  id: string;
  name: string;
  color: string;
}

export type Priority = 0 | 1 | 2 | 3;

export interface TodoItem {
  id: string;
  title: string;
  done: boolean;
  doneAt: number | null;
  /** "YYYY-MM-DD", a local day. */
  due: string | null;
  /** "HH:MM" on the due day; absent in lists saved before reminders. */
  time?: string | null;
  priority: Priority;
  listId: string | null;
  createdAt: number;
}

export interface TodoDoc {
  version: 1;
  lists: TodoList[];
  items: TodoItem[];
}

export const EMPTY_DOC: TodoDoc = { version: 1, lists: [], items: [] };

export const LIST_COLORS = ["#3B82F6", "#22C55E", "#F59E0B", "#EF4444", "#A855F7", "#EC4899", "#14B8A6"];

export const PRIORITY_COLORS: Record<Priority, string> = {
  0: "transparent",
  1: "#60A5FA",
  2: "#F59E0B",
  3: "#F4505E",
};

/** "all", "today", or a list id. */
export type TodoFilter = string;

// ── Days ──────────────────────────────────────────────────────────────────────

export function dayOf(date: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${p(date.getMonth() + 1)}-${p(date.getDate())}`;
}

function addDays(date: Date, n: number): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + n);
}

function parseDay(s: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
  if (!m) return null;
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  return dayOf(d) === s ? d : null;
}

const WEEKDAYS: Record<string, number> = {
  dom: 0, seg: 1, ter: 2, qua: 3, qui: 4, sex: 5, sab: 6,
  sun: 0, mon: 1, tue: 2, wed: 3, thu: 4, fri: 5, sat: 6,
};

/** "@hoje", "@tomorrow", "@sex", "@2026-10-05" → a day; null when it is not a date. */
export function parseDueToken(token: string, now: Date): string | null {
  const t = token.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "");
  if (t === "hoje" || t === "today") return dayOf(now);
  if (t === "amanha" || t === "tomorrow") return dayOf(addDays(now, 1));
  if (t in WEEKDAYS) {
    // The coming one, today included: "@fri" on a Friday means today.
    return dayOf(addDays(now, (WEEKDAYS[t] - now.getDay() + 7) % 7));
  }
  const d = parseDay(t);
  return d ? dayOf(d) : null;
}

/** "15:30", "9h", "14h30" → "HH:MM"; null when it is not a time. */
export function parseTimeToken(token: string): string | null {
  const m = /^(\d{1,2})(?::(\d{2})|h(\d{2})?)$/i.exec(token);
  if (!m) return null;
  const hour = Number(m[1]);
  const min = Number(m[2] ?? m[3] ?? 0);
  if (hour > 23 || min > 59) return null;
  return `${String(hour).padStart(2, "0")}:${String(min).padStart(2, "0")}`;
}

// ── Quick add ─────────────────────────────────────────────────────────────────

export interface QuickAdd {
  title: string;
  priority: Priority;
  listName: string | null;
  due: string | null;
  time: string | null;
}

/**
 * "Send the report !3 #work @tomorrow" → title, priority, list, due day.
 * A token that is not a valid marker stays in the title. Null when no title is left.
 */
export function parseQuickAdd(text: string, now: Date): QuickAdd | null {
  let priority: Priority = 0;
  let listName: string | null = null;
  let due: string | null = null;
  let time: string | null = null;
  const words: string[] = [];
  for (const word of text.trim().split(/\s+/)) {
    const prio = /^!([1-3])$/.exec(word);
    if (prio) {
      priority = Number(prio[1]) as Priority;
      continue;
    }
    if (/^#\S+$/.test(word)) {
      listName = word.slice(1);
      continue;
    }
    if (word.startsWith("@") && word.length > 1) {
      const day = parseDueToken(word.slice(1), now);
      if (day) {
        due = day;
        continue;
      }
      const t = parseTimeToken(word.slice(1));
      if (t) {
        time = t;
        continue;
      }
    }
    if (word) words.push(word);
  }
  const title = words.join(" ");
  return title ? { title, priority, listName, due, time } : null;
}

/** A time alone means its next occurrence: today, or tomorrow once it has passed. */
function dayForTime(time: string, now: Date): string {
  const [hh, mm] = time.split(":").map(Number);
  return dayOf(addDays(now, hh * 60 + mm > now.getHours() * 60 + now.getMinutes() ? 0 : 1));
}

// ── Views of the list ─────────────────────────────────────────────────────────

/** Done items stay in sight for the rest of the day they were ticked. */
export function isVisible(item: TodoItem, today: string): boolean {
  return !item.done || (item.doneAt !== null && dayOf(new Date(item.doneAt)) === today);
}

export function isOverdue(item: TodoItem, today: string): boolean {
  return !item.done && item.due !== null && item.due < today;
}

/** Overdue and due today, still open: the number on the tab. */
export function dueCount(doc: TodoDoc, today: string): number {
  return doc.items.filter((i) => !i.done && i.due !== null && i.due <= today).length;
}

/** Open first: overdue, then by day (undated last), priority, newest. Done last, latest first. */
export function compareTodos(a: TodoItem, b: TodoItem): number {
  if (a.done !== b.done) return a.done ? 1 : -1;
  if (a.done) return (b.doneAt ?? 0) - (a.doneAt ?? 0);
  if (a.due !== b.due) {
    if (a.due === null) return 1;
    if (b.due === null) return -1;
    return a.due < b.due ? -1 : 1;
  }
  if ((a.time ?? null) !== (b.time ?? null)) {
    if (!a.time) return 1;
    if (!b.time) return -1;
    return a.time < b.time ? -1 : 1;
  }
  if (a.priority !== b.priority) return b.priority - a.priority;
  return b.createdAt - a.createdAt;
}

export function visibleTodos(doc: TodoDoc, filter: TodoFilter, today: string): TodoItem[] {
  return doc.items
    .filter((i) => isVisible(i, today))
    .filter((i) =>
      filter === "all" ? true
      : filter === "today" ? i.due !== null && i.due <= today
      : i.listId === filter)
    .sort(compareTodos);
}

/** "Today", "Tomorrow", "Yesterday", "Fri", "Oct 5". */
export function dueLabel(due: string, now: Date): string {
  const day = parseDay(due);
  if (!day) return due;
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const diff = Math.round((day.getTime() - today.getTime()) / 86_400_000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Tomorrow";
  if (diff === -1) return "Yesterday";
  if (diff > 1 && diff < 7) return day.toLocaleDateString("en", { weekday: "short" });
  return day.toLocaleDateString("en", { month: "short", day: "numeric" });
}

/** "Today 15:30", or just the day. */
export function whenLabel(item: TodoItem, now: Date): string {
  if (!item.due) return "";
  return item.time ? `${dueLabel(item.due, now)} ${item.time}` : dueLabel(item.due, now);
}

// ── Changes (each returns a new document) ─────────────────────────────────────

const newId = () => crypto.randomUUID();

/**
 * Adds the quick-add text, creating its list when it is new. `defaults` fill
 * what the text leaves out (the filter in use). Null when there is no title.
 */
export function addTodo(
  doc: TodoDoc,
  text: string,
  now: Date,
  defaults: { listId?: string | null; due?: string | null; time?: string | null; priority?: Priority } = {},
): TodoDoc | null {
  const q = parseQuickAdd(text, now);
  if (!q) return null;
  let lists = doc.lists;
  let listId = defaults.listId ?? null;
  if (q.listName) {
    const name = q.listName;
    const found = lists.find((l) => l.name.toLowerCase() === name.toLowerCase());
    if (found) {
      listId = found.id;
    } else {
      const list = { id: newId(), name, color: LIST_COLORS[lists.length % LIST_COLORS.length] };
      lists = [...lists, list];
      listId = list.id;
    }
  }
  const time = q.time ?? defaults.time ?? null;
  const due = q.due ?? defaults.due ?? (time ? dayForTime(time, now) : null);
  const item: TodoItem = {
    id: newId(),
    title: q.title,
    done: false,
    doneAt: null,
    due,
    time: due ? time : null,
    priority: q.priority || defaults.priority || 0,
    listId,
    createdAt: now.getTime(),
  };
  return { ...doc, lists, items: [...doc.items, item] };
}

function updateItem(doc: TodoDoc, id: string, change: (i: TodoItem) => TodoItem): TodoDoc {
  return { ...doc, items: doc.items.map((i) => (i.id === id ? change(i) : i)) };
}

export function toggleTodo(doc: TodoDoc, id: string, now: Date): TodoDoc {
  return updateItem(doc, id, (i) => ({ ...i, done: !i.done, doneAt: i.done ? null : now.getTime() }));
}

export type TodoFields = Pick<TodoItem, "title" | "due" | "priority" | "listId"> & { time: string | null };

/** The editor's fields; a blank title keeps the old one, a time needs a day. */
export function editTodo(doc: TodoDoc, id: string, fields: TodoFields): TodoDoc {
  const title = fields.title.trim();
  return updateItem(doc, id, (i) => ({
    ...i, ...fields, title: title || i.title, time: fields.due ? fields.time : null,
  }));
}

/** Open tasks by urgency for the home card: dated ones by day, then undated by priority. */
export function upNext(doc: TodoDoc, limit: number): TodoItem[] {
  return doc.items.filter((i) => !i.done).sort(compareTodos).slice(0, limit);
}

// ── Reminders ─────────────────────────────────────────────────────────────────

export const REMINDER_HOURS = [9, 14];

/** The next 9:00 or 14:00 strictly after `now`. */
export function nextReminder(now: Date): Date {
  for (let n = 0; ; n++) {
    for (const hour of REMINDER_HOURS) {
      const at = new Date(now.getFullYear(), now.getMonth(), now.getDate() + n, hour);
      if (at > now) return at;
    }
  }
}

/** Open tasks due today or tomorrow: what a reminder slot opens the island for. */
export function reminderDue(doc: TodoDoc, now: Date): TodoItem[] {
  const days = [dayOf(now), dayOf(addDays(now, 1))];
  return doc.items.filter((i) => !i.done && i.due !== null && days.includes(i.due));
}

/** When a timed task rings; null without a day and a time. */
export function taskAt(item: TodoItem): Date | null {
  const day = item.due ? parseDay(item.due) : null;
  if (!day || !item.time) return null;
  const [hh, mm] = item.time.split(":").map(Number);
  return new Date(day.getFullYear(), day.getMonth(), day.getDate(), hh, mm);
}

/**
 * The next moment the island should open, strictly after `now`: a 9:00/14:00
 * slot or an open task's own time. Whether it has anything to show is checked
 * when it fires, with `ringsAt`.
 */
export function nextWake(doc: TodoDoc, now: Date): Date {
  let at = nextReminder(now);
  for (const i of doc.items) {
    const t = i.done ? null : taskAt(i);
    if (t && t > now && t < at) at = t;
  }
  return at;
}

/** At `at`: the open tasks timed for it, else (on a slot) those due today or tomorrow. */
export function ringsAt(doc: TodoDoc, at: Date): TodoItem[] {
  const timed = doc.items.filter((i) => !i.done && taskAt(i)?.getTime() === at.getTime());
  if (timed.length) return timed;
  const slot = at.getMinutes() === 0 && REMINDER_HOURS.includes(at.getHours());
  return slot ? reminderDue(doc, at) : [];
}

/** Choices for the day picker: no date, the next seven days. */
export function dayChoices(now: Date): [string, string][] {
  const out: [string, string][] = [["", "No date"]];
  for (let n = 0; n < 7; n++) {
    const day = dayOf(addDays(now, n));
    out.push([day, dueLabel(day, now)]);
  }
  return out;
}

export function removeTodo(doc: TodoDoc, id: string): TodoDoc {
  return { ...doc, items: doc.items.filter((i) => i.id !== id) };
}

export function clearCompleted(doc: TodoDoc): TodoDoc {
  return { ...doc, items: doc.items.filter((i) => !i.done) };
}

export function addList(doc: TodoDoc, name: string): TodoDoc {
  const n = name.trim();
  if (!n) return doc;
  const color = LIST_COLORS[doc.lists.length % LIST_COLORS.length];
  return { ...doc, lists: [...doc.lists, { id: newId(), name: n, color }] };
}

export function updateList(doc: TodoDoc, id: string, change: Partial<Omit<TodoList, "id">>): TodoDoc {
  return { ...doc, lists: doc.lists.map((l) => (l.id === id ? { ...l, ...change } : l)) };
}

/** The list goes; its tasks stay, without a list. */
export function removeList(doc: TodoDoc, id: string): TodoDoc {
  return {
    ...doc,
    lists: doc.lists.filter((l) => l.id !== id),
    items: doc.items.map((i) => (i.listId === id ? { ...i, listId: null } : i)),
  };
}
