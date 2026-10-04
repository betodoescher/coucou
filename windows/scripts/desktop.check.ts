// Self-check for the desktop Mochi's rules: node scripts/desktop.check.ts
import assert from "node:assert/strict";
import { alertAction, shouldSleep } from "../src/desktop/logic.ts";

const far = Infinity;
assert.equal(shouldSleep({ state: "idle", idleFor: 121, mouseDistance: far }), true);
assert.equal(shouldSleep({ state: "idle", idleFor: 119, mouseDistance: far }), false, "two minutes first");
assert.equal(shouldSleep({ state: "idle", idleFor: 300, mouseDistance: 149 }), false, "the cursor nearby keeps him up");
assert.equal(shouldSleep({ state: "idle", idleFor: 300, mouseDistance: 150 }), true);
assert.equal(shouldSleep({ state: "working", idleFor: 300, mouseDistance: far }), false, "never while an agent works");
assert.equal(shouldSleep({ state: "approval", idleFor: 300, mouseDistance: far }), false);

assert.equal(alertAction({ onDesktop: false, atIsland: false, alert: true }), null, "only a desktop Mochi flies");
assert.equal(alertAction({ onDesktop: true, atIsland: false, alert: true }), "retract");
assert.equal(alertAction({ onDesktop: true, atIsland: true, alert: true }), null, "already at the island");
assert.equal(alertAction({ onDesktop: true, atIsland: true, alert: false }), "fly-out");
assert.equal(alertAction({ onDesktop: true, atIsland: false, alert: false }), null);

console.log("desktop: all checks passed");
