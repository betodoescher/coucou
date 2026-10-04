// Every outfit on the real BotEngine, turned left, front, right, up, down and
// mid-roll, plus its wardrobe tile and the compact size. `npm run dev`, then
// open /dev/outfits-preview.html.

import { BotEngine } from "../src/mochi/engine";
import { OUTFIT_CHOICES, drawOutfitIcon, seasonalOutfit, type Worn } from "../src/mochi/outfits";

const POSES = [
  { yaw: -0.5, pitch: 0, roll: 0 },
  { yaw: 0, pitch: 0, roll: 0 },
  { yaw: 0.5, pitch: 0, roll: 0 },
  { yaw: 0.15, pitch: 0.4, roll: 0 },
  { yaw: -0.2, pitch: -0.5, roll: 0 },
  { yaw: 0, pitch: 0, roll: 1.2 },
];
const SIZE = 97;
const OVERHANG = 40;
const dpr = Math.min(2, window.devicePixelRatio || 1);
const sheet = document.getElementById("sheet")!;

function canvas(w: number, h: number): CanvasRenderingContext2D {
  const c = document.createElement("canvas");
  c.width = w * dpr;
  c.height = h * dpr;
  c.style.width = `${w}px`;
  c.style.height = `${h}px`;
  const ctx = c.getContext("2d")!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return ctx;
}

function mochi(worn: Worn, size: number, pose: (typeof POSES)[number]) {
  const e = new BotEngine();
  e.particleOverhang = OVERHANG;
  e.setOutfit(worn, false);
  e.yaw = pose.yaw;
  e.pitch = pose.pitch;
  e.roll = pose.roll;
  const ctx = canvas(size, size + OVERHANG);
  e.draw(ctx, size, size + OVERHANG);
  return ctx.canvas;
}

for (const choice of OUTFIT_CHOICES) {
  if (choice === "auto") continue;
  const row = document.createElement("div");
  row.className = "row";
  const label = document.createElement("span");
  label.textContent = choice;
  row.append(label);
  for (const pose of POSES) row.append(mochi(choice, SIZE, pose));
  row.append(mochi(choice, 33, POSES[1]));
  const tile = canvas(30, 30);
  drawOutfitIcon(tile, 30, choice, seasonalOutfit());
  row.append(tile.canvas);
  sheet.append(row);
}
