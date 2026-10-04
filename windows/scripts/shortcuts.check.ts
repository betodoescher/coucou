// Self-check for the keyboard shortcuts: node scripts/shortcuts.check.ts
import assert from "node:assert/strict";
import { cyclePill, duplicates, islandShortcut, keyName, keyOf } from "../src/core/shortcuts.ts";

assert.equal(keyOf({}, "openChat"), "Space");
assert.equal(keyOf({}, "toggleIsland"), "", "off by default, as on macOS");
assert.equal(keyOf({ goToAlert: "" }, "goToAlert"), "", "turned off");
assert.equal(keyOf({ goToAlert: " Q " }, "goToAlert"), "Q");

assert.deepEqual([...duplicates({})], [], "the defaults never clash");
assert.deepEqual([...duplicates({ muteToggle: "a" })].sort(), ["goToAlert", "muteToggle"]);
assert.deepEqual([...duplicates({ muteToggle: "", goToAlert: "" })], []);

assert.equal(keyName({ key: "a", code: "KeyA" }), "A");
assert.equal(keyName({ key: " ", code: "Space" }), "Space");
assert.equal(keyName({ key: "ç", code: "Semicolon" }), ";", "the physical key, whatever the layout");
assert.equal(keyName({ key: "Control", code: "ControlLeft" }), "");
assert.equal(keyName({ key: "]", code: "BracketRight" }), "]");

assert.equal(cyclePill(["a", "b", "c"], "c", 1), "a", "wraps forwards");
assert.equal(cyclePill(["a", "b", "c"], "a", -1), "c", "wraps backwards");
assert.equal(cyclePill(["a", "b"], null, 1), "a");
assert.equal(cyclePill([], "a", 1), null);

const key = (code: string, k = "", mods: Partial<Record<"ctrlKey" | "metaKey" | "altKey" | "shiftKey", boolean>> = { ctrlKey: true }) => ({
  key: k, code, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods,
});
assert.deepEqual(islandShortcut(key("ArrowRight", "ArrowRight"), false), { kind: "pill", delta: 1 });
assert.equal(islandShortcut(key("ArrowRight", "ArrowRight"), true), null, "a text field keeps word moves");
assert.deepEqual(islandShortcut(key("Digit3"), true), { kind: "pillAt", index: 2 });
assert.deepEqual(islandShortcut(key("KeyK", "k", { metaKey: true }), false), { kind: "newChat" });
assert.equal(islandShortcut(key("KeyE", "e", {}), false), null, "needs Ctrl");
assert.equal(islandShortcut(key("KeyE", "e", { ctrlKey: true, altKey: true }), false), null, "AltGr is Ctrl+Alt");

console.log("shortcuts: all checks passed");
