// Mochi on the desktop — the page of his own little window (desktop.rs).
// Port of DesktopBotView in DesktopMochi.swift.
//
// The island decides what he feels and wears and sends it here; this page
// draws him, follows the cursor with his eyes, falls asleep when nothing goes
// on, and turns clicks and drags into requests for the island and Rust.

import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";
import type { BotEmoteName, BotStateName } from "../core/layout";
import { BotEngine } from "../mochi/engine";
import type { Worn } from "../mochi/outfits";
import { shouldSleep, type DesktopState } from "./logic";

/** Same numbers as src-tauri/src/desktop.rs. */
const WIN_W = 128;
const WIN_H = 144;
const BODY_X = 64;
const BODY_Y = 90;
const HIT_R = 36;
/** Mochi's canvas inside the window, as the island draws him. */
const BOT_W = 96;
const OVERHANG = 40;

const DOUBLE_CLICK_MS = 300;
const DRAG_THRESHOLD = 4;

const canvas = document.getElementById("mochi") as HTMLCanvasElement;
const engine = new BotEngine();
engine.particleOverhang = OVERHANG;

let cursorPoll = false;
let visible = false;
/** What the island says Mochi is up to. */
let state: BotStateName = "idle";
let sleeping = false;
let dizzyUntil = 0;
let lastActive = performance.now();
/** Cursor in window coordinates; null when we cannot know where it is. */
let cursor: { x: number; y: number } | null = null;

const toIsland = (event: string, payload?: unknown) => void emitTo("island", event, payload ?? null);

function resize() {
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  canvas.width = Math.round(WIN_W * dpr);
  canvas.height = Math.round(WIN_H * dpr);
  canvas.style.width = `${WIN_W}px`;
  canvas.style.height = `${WIN_H}px`;
}

function wake() {
  lastActive = performance.now();
  if (sleeping) {
    sleeping = false;
    applyState();
  }
}

function applyState() {
  if (performance.now() < dizzyUntil) return;
  engine.setState(sleeping ? "sleeping" : state);
}

// ── Frame loop: 30 fps awake, 10 asleep, nothing while hidden ────────────────

let timer = 0;
let lastFrame = performance.now();

function frame() {
  timer = 0;
  if (!visible) return;
  const now = performance.now();
  const dt = Math.min(0.1, (now - lastFrame) / 1000);
  lastFrame = now;

  const distance = cursor ? Math.hypot(cursor.x - BODY_X, cursor.y - BODY_Y) : Infinity;
  const ds: DesktopState = { state, idleFor: (now - lastActive) / 1000, mouseDistance: distance };
  if (!sleeping && shouldSleep(ds)) {
    sleeping = true;
    applyState();
  } else if (sleeping && !shouldSleep(ds)) {
    wake();
  }

  engine.lookX = cursor ? Math.tanh((cursor.x - BODY_X) / 260) : 0;
  engine.lookY = cursor ? -Math.tanh((cursor.y - BODY_Y) / 200) : 0;
  engine.update(dt);

  const ctx = canvas.getContext("2d");
  if (ctx) {
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, WIN_W, WIN_H);
    ctx.translate((WIN_W - BOT_W) / 2, 0);
    engine.draw(ctx, BOT_W, BOT_W + OVERHANG);
  }
  schedule();
}

function schedule() {
  if (timer || !visible) return;
  timer = window.setTimeout(frame, sleeping ? 100 : 33);
}

// ── Input ─────────────────────────────────────────────────────────────────────

let press: { x: number; y: number } | null = null;
let dragging = false;
let dragBusy = false;
let clickTimer = 0;

const onBody = (x: number, y: number) => Math.hypot(x - BODY_X, y - BODY_Y) <= HIT_R;

window.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || !onBody(e.clientX, e.clientY)) return;
  wake();
  press = { x: e.screenX, y: e.screenY };
  dragging = false;
});

window.addEventListener("mousemove", (e) => {
  // Without Rust's cursor poll (Linux) the page only knows the cursor while it
  // is over Mochi, which is enough to look at it and to wake up.
  if (!cursorPoll) cursor = { x: e.clientX, y: e.clientY };
  if (!press) return;
  if (!dragging) {
    if (Math.hypot(e.screenX - press.x, e.screenY - press.y) < DRAG_THRESHOLD) return;
    dragging = true;
    void invoke("desktop_mochi_drag_start", { x: e.screenX, y: e.screenY });
    return;
  }
  if (cursorPoll || dragBusy) return;
  dragBusy = true;
  void invoke("desktop_mochi_drag", { x: e.screenX, y: e.screenY }).finally(() => { dragBusy = false; });
});

window.addEventListener("mouseup", () => {
  if (!press) return;
  press = null;
  if (dragging) {
    dragging = false;
    if (!cursorPoll) void invoke("desktop_mochi_drop");
    return;
  }
  if (clickTimer) {
    // Double-click: fly home.
    window.clearTimeout(clickTimer);
    clickTimer = 0;
    toIsland("desktop-sound", "peek");
    void invoke("desktop_mochi_home");
    return;
  }
  clickTimer = window.setTimeout(() => {
    clickTimer = 0;
    engine.slap();
  }, DOUBLE_CLICK_MS);
});

window.addEventListener("mouseleave", () => {
  if (!cursorPoll) cursor = null;
});

window.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  if (!onBody(e.clientX, e.clientY)) return;
  wake();
  toIsland("desktop-wardrobe");
});

engine.onDizzy = () => {
  dizzyUntil = performance.now() + 3300;
  engine.setState("dizzy");
  toIsland("desktop-sound", "dizzy");
  window.setTimeout(() => {
    dizzyUntil = 0;
    applyState();
    engine.triggerEmote("happy");
  }, 3300);
};

// ── Events ────────────────────────────────────────────────────────────────────

async function main() {
  resize();
  window.addEventListener("resize", resize);

  const boot = await invoke<{ cursorPoll: boolean }>("boot").catch(() => null);
  cursorPoll = boot?.cursorPoll ?? false;

  await listen<{ state: BotStateName; outfit: Worn; animated: boolean }>("desktop-state", (e) => {
    const next = e.payload.state;
    if (next !== state && next !== "idle" && next !== "sleeping") wake();
    state = next;
    engine.setOutfit(e.payload.outfit, e.payload.animated);
    applyState();
  });

  await listen<BotEmoteName>("desktop-emote", (e) => {
    wake();
    engine.triggerEmote(e.payload);
  });

  await listen<{ visible: boolean; landed: boolean }>("desktop-mochi", (e) => {
    visible = e.payload.visible;
    if (visible) {
      lastFrame = performance.now();
      schedule();
    }
    if (e.payload.landed) {
      wake();
      engine.triggerEmote("happy");
    }
  });

  await listen<{ x: number; y: number }>("desktop-cursor", (e) => {
    cursor = e.payload;
    if (Math.hypot(cursor.x - BODY_X, cursor.y - BODY_Y) < 150 && sleeping) wake();
  });

  // Whichever page loads first, the other one hears about it.
  await listen("desktop-ping", () => toIsland("desktop-hello"));
  toIsland("desktop-hello");
}

void main();
