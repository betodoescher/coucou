// Wardrobe — right-click Mochi to dress him. Port of WardrobeView in
// IslandViewContent.swift: hovering a tile previews it on Mochi, a click keeps it.

import { h } from "./dom";
import { State } from "../core/state";
import {
  OUTFIT_CHOICES, OUTFIT_NAMES, drawOutfitIcon, parseOutfitChoice, seasonalOutfit,
  type OutfitChoice, type Worn,
} from "../mochi/outfits";
import type { ViewActions, ViewHost } from "./views";

const TILE = 30;

function seasonName(): string {
  const s = seasonalOutfit();
  return s === "none" ? "None" : OUTFIT_NAMES[s];
}

export function buildWardrobe(actions: ViewActions): ViewHost {
  const status = h("span", { class: "wardrobe-status" });
  let hovered: OutfitChoice | null = null;
  let drawnFor = "";

  const tiles = OUTFIT_CHOICES.map((choice) => {
    const canvas = h("canvas") as HTMLCanvasElement;
    const tile = h(
      "button",
      {
        class: "wardrobe-tile",
        title: OUTFIT_NAMES[choice],
        onmouseenter: () => {
          hovered = choice;
          actions.previewOutfit(choice === "auto" ? seasonalOutfit() : choice);
          paintStatus();
        },
        onmouseleave: () => {
          if (hovered !== choice) return;
          hovered = null;
          actions.previewOutfit(null);
          paintStatus();
        },
        onclick: () => actions.wearOutfit(choice),
      },
      canvas,
    );
    return { choice, tile, canvas };
  });

  function paintStatus() {
    if (hovered) {
      status.textContent = hovered === "auto"
        ? `Auto · follows the seasons (now: ${seasonName()})`
        : OUTFIT_NAMES[hovered];
      return;
    }
    const sel = parseOutfitChoice(State.settings.mochiOutfit);
    status.textContent = sel === "auto" ? `Auto · ${seasonName()}` : OUTFIT_NAMES[sel];
  }

  /** The tiles only change with the season (the Auto tile) or the screen scale. */
  function paintTiles() {
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    const seasonal: Worn = seasonalOutfit();
    const key = `${dpr}|${seasonal}`;
    if (key === drawnFor) return;
    drawnFor = key;
    for (const { choice, canvas } of tiles) {
      canvas.width = Math.round(TILE * dpr);
      canvas.height = Math.round(TILE * dpr);
      const ctx = canvas.getContext("2d");
      if (!ctx) continue;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, TILE, TILE);
      drawOutfitIcon(ctx, TILE, choice, seasonal);
    }
  }

  const body = h(
    "div",
    { class: "stack wardrobe" },
    h("div", { class: "wardrobe-head" }, h("span", { class: "wardrobe-title", text: "Wardrobe" }), status),
    h("div", { class: "wardrobe-grid" }, ...tiles.map((t) => t.tile)),
  );
  const el = h("div", { class: "view" }, h("div", { class: "card" }, body));

  return {
    el,
    sync() {
      paintTiles();
      const sel = parseOutfitChoice(State.settings.mochiOutfit);
      for (const { choice, tile } of tiles) tile.classList.toggle("on", choice === sel);
      if (!State.wardrobePreview) hovered = null;
      paintStatus();
    },
  };
}
