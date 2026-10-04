// Dev sheet: the launch greeting frozen at key moments. Open /dev/greeting-preview.html.

import { Greeting } from "../src/mochi/greeting";

const FRAMES = [0.1, 0.3, 0.5, 0.6, 0.73, 1.0, 1.3, 1.4, 1.7, 2.0, 2.6, 2.8, 3.2, 3.76, 4.3];
const grid = document.getElementById("grid")!;
const dpr = window.devicePixelRatio || 1;

for (const t of FRAMES) {
  const fig = document.createElement("figure");
  const canvas = document.createElement("canvas");
  canvas.width = 640 * dpr;
  canvas.height = 150 * dpr;
  canvas.style.width = "640px";
  canvas.style.height = "150px";
  const cap = document.createElement("figcaption");
  cap.textContent = `t = ${t.toFixed(2)} s`;
  fig.append(canvas, cap);
  grid.append(fig);

  const g = new Greeting() as unknown as { startMs: number; fired: boolean; draw(x: CanvasRenderingContext2D): void };
  g.startMs = performance.now() - t * 1000;
  g.fired = true;
  const x = canvas.getContext("2d")!;
  x.scale(dpr, dpr);
  g.draw(x);
}
