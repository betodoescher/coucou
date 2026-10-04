// Keyboard shortcuts: the global ones Rust registers (src-tauri/src/shortcuts.rs
// keeps the same table) and the ones that work inside the open island.

export type ShortcutAction =
  | "toggleIsland"
  | "openChat"
  | "goToAlert"
  | "jumpToTerminal"
  | "nextPill"
  | "prevPill"
  | "muteToggle"
  | "desktopToggle"
  | "wardrobeToggle";

/** Every global shortcut, its label and default key. */
export const SHORTCUTS: { action: ShortcutAction; label: string; key: string }[] = [
  { action: "toggleIsland", label: "Open / close the island", key: "" },
  { action: "openChat", label: "Open the chat", key: "Space" },
  { action: "goToAlert", label: "Go to the waiting permission or question", key: "A" },
  { action: "jumpToTerminal", label: "Bring the agent's window forward", key: "T" },
  { action: "nextPill", label: "Next pill", key: "]" },
  { action: "prevPill", label: "Previous pill", key: "[" },
  { action: "muteToggle", label: "Mute / unmute Mochi", key: "M" },
  { action: "desktopToggle", label: "Send Mochi to the desktop and back", key: "D" },
  { action: "wardrobeToggle", label: "Open / close the wardrobe", key: "G" },
];

export const MODIFIERS = ["Ctrl+Shift+Alt", "Super+Alt", "Ctrl+Alt", "Super+Shift", "Ctrl+Shift"];

/** Shown in Settings; handled by the island's own key listener. */
export const ISLAND_SHORTCUTS: [string, string][] = [
  ["Ctrl+→ / Ctrl+←", "Next / previous pill"],
  ["Ctrl+1 – Ctrl+9", "Switch to pill by number"],
  ["Ctrl+E", "Open / close the latest diff"],
  ["Ctrl+Enter", "Send the chat message"],
  ["Ctrl+K", "New conversation"],
  ["Ctrl+,", "Open Settings"],
  ["Ctrl+P", "Pin / unpin the island"],
  ["Esc", "Close the island (unless pinned)"],
];

export function keyOf(shortcuts: Record<string, string>, action: ShortcutAction): string {
  const own = shortcuts[action];
  return own != null ? own.trim() : SHORTCUTS.find((s) => s.action === action)?.key ?? "";
}

/** Actions sharing the same key with another one that is on. */
export function duplicates(shortcuts: Record<string, string>): Set<ShortcutAction> {
  const seen = new Map<string, ShortcutAction>();
  const dups = new Set<ShortcutAction>();
  for (const { action } of SHORTCUTS) {
    const key = keyOf(shortcuts, action).toUpperCase();
    if (!key) continue;
    const other = seen.get(key);
    if (other) {
      dups.add(other);
      dups.add(action);
    } else {
      seen.set(key, action);
    }
  }
  return dups;
}

/** The key a keydown names, in the accelerator form Rust parses, or "" for modifiers alone. */
export function keyName(e: { key: string; code: string }): string {
  if (["Control", "Shift", "Alt", "Meta", "AltGraph", "OS"].includes(e.key)) return "";
  if (e.code === "Space") return "Space";
  if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
  if (/^Digit\d$/.test(e.code)) return e.code.slice(5);
  if (/^F\d{1,2}$/.test(e.code)) return e.code;
  const named: Record<string, string> = {
    BracketLeft: "[", BracketRight: "]", Comma: ",", Period: ".", Slash: "/", Semicolon: ";",
    Quote: "'", Backquote: "`", Minus: "-", Equal: "=", Backslash: "\\", Enter: "Enter",
  };
  return named[e.code] ?? "";
}

/** The pill `delta` steps away, wrapping around; the first one when none has the focus. */
export function cyclePill(ids: string[], current: string | null, delta: number): string | null {
  if (ids.length === 0) return null;
  const at = current == null ? -1 : ids.indexOf(current);
  if (at < 0) return delta > 0 ? ids[0] : ids[ids.length - 1];
  return ids[(((at + delta) % ids.length) + ids.length) % ids.length];
}

export type IslandShortcut =
  | { kind: "pill"; delta: number }
  | { kind: "pillAt"; index: number }
  | { kind: "diff" }
  | { kind: "newChat" }
  | { kind: "settings" }
  | { kind: "pin" };

/**
 * What a keydown in the open island asks for. Ctrl (or ⌘) is the modifier;
 * the arrows are left to a text field, where they move by word.
 */
export function islandShortcut(
  e: { key: string; code: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean },
  inTextField: boolean,
): IslandShortcut | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return null;
  if (e.key === "ArrowRight" && !inTextField) return { kind: "pill", delta: 1 };
  if (e.key === "ArrowLeft" && !inTextField) return { kind: "pill", delta: -1 };
  const digit = /^Digit([1-9])$/.exec(e.code);
  if (digit) return { kind: "pillAt", index: Number(digit[1]) - 1 };
  switch (e.code) {
    case "KeyE":
      return { kind: "diff" };
    case "KeyK":
      return { kind: "newChat" };
    case "Comma":
      return { kind: "settings" };
    case "KeyP":
      return { kind: "pin" };
  }
  return null;
}
