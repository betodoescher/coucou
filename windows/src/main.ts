// Entry point: boot the bridge, wire the island, start the greeting.

import "./style.css";
import { Bridge, IS_TAURI, onEvent } from "./core/bridge";
import { Sound } from "./core/sound";
import { State, type Settings } from "./core/state";
import { Todos } from "./core/todoStore";
import { nextWake, ringsAt } from "./core/todos";
import { Island } from "./island/island";
import { registerHookHandlers } from "./island/hooks";
import { registerIntegrationHandlers, refreshConfigured } from "./island/integrations";

async function main() {
  const root = document.getElementById("root");
  if (!root) return;

  void Sound.preload();

  const island = new Island(root);

  const boot = await Bridge.boot();
  if (boot) {
    State.settings = { ...State.settings, ...boot.settings };
  }
  island.applySettings();
  State.loadIntegrationTasks();
  if (boot && !boot.cursorPoll) island.followPageCursor();
  island.setMovable(boot?.islandMovable ?? false);

  await onEvent<{ x: number; y: number }>("cursor", ({ x, y }) => island.onCursor(x, y));

  /** Pause has to reach Rust too, or the pollers keep calling out. */
  const setPaused = (on: boolean) => {
    if (State.paused === on) return;
    State.paused = on;
    void Bridge.setPaused(on);
  };

  await onEvent<string>("tray", (what) => {
    switch (what) {
      case "settings":
        setPaused(false);
        island.alert("settings");
        break;
      case "open":
        setPaused(false);
        island.alert(State.defaultView());
        break;
      case "pause":
        setPaused(!State.paused);
        if (State.paused) island.fsm.forceHidden();
        else island.reveal();
        break;
    }
  });

  await onEvent<null>("screen-changed", () => void Bridge.reposition());

  // The settings window writes preferences; apply them here without a restart.
  await onEvent<Settings>("settings-changed", (s) => {
    const cityChanged = (s.weatherCity ?? "") !== State.settings.weatherCity;
    State.settings = { ...State.settings, ...s };
    if (cityChanged) {
      delete State.integrations.weather;
      if (State.settings.weatherCity.trim()) void Bridge.refreshIntegration("weather");
    }
    island.applySettings();
    State.loadIntegrationTasks();
    void refreshConfigured();
  });

  // One timer to the next 9:00/14:00 slot or task time, no polling; re-aimed
  // whenever the list changes. A reminder missed by more than half an hour
  // (the machine was asleep) is skipped, not replayed.
  let reminderTimer = 0;
  let lastRing = 0;
  const scheduleReminder = () => {
    window.clearTimeout(reminderTimer);
    // From the last ring, so a timer that fires a hair early never rings twice.
    const at = nextWake(Todos.doc, new Date(Math.max(Date.now(), lastRing)));
    reminderTimer = window.setTimeout(() => {
      lastRing = at.getTime();
      const late = Date.now() - at.getTime() > 30 * 60_000;
      if (!State.paused && !late && ringsAt(Todos.doc, at).length) {
        State.todayTab = "tasks";
        island.alert("todos");
      }
      scheduleReminder();
    }, at.getTime() - Date.now());
  };

  const syncTodoPill = () => State.setTodoPill(Todos.doc.items.filter((i) => !i.done).length);
  Todos.subscribe(() => {
    syncTodoPill();
    scheduleReminder();
    State.notify();
  });
  await Todos.init();
  syncTodoPill();
  scheduleReminder();

  registerHookHandlers(island);
  registerIntegrationHandlers(island);

  island.launch();

  // In a plain browser there is no wake strip behind the cursor: make the whole
  // page wake the island so the visuals can be checked with `npm run dev`.
  if (!IS_TAURI) {
    document.addEventListener("click", () => Sound.resume(), { once: true });
  }
}

void main();
