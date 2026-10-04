// Mochi's wardrobe — accessories drawn in code over BotEngine's body. Port of
// upstream design/outfits/mochi-outfits.js (the reference) and
// MochiOutfitDrawing.swift (bunny ears, transitions, front-arc ordering).
//
// Coordinates are body space: origin at the body centre, y down, R = W·0.3,
// rx = 1.14R, ry = 0.88R. The head is a superellipsoid whose ring radius at
// height y (y up, −1…1) is (1 − |y|^2.7)^(1/2.7), so its silhouette matches
// the body path at yaw = pitch = 0.

import { Ease } from "../core/anim";

import type { Outfit, OutfitChoice, Worn } from "./seasons";

export * from "./seasons";

// ── Head model ────────────────────────────────────────────────────────────────

const EXP = 2.7;
/** Accessories are seen slightly from above, so rings read as ellipses. */
const VIEW_TILT = -0.3;
/** Accessories follow the head pitch only partly: hats never flip to a top view. */
const ACC_PITCH = 0.4;
const EYE_W = 0.25;
const EYE_H = 0.27;
const EYE_SP = 0.37;
const EYE_P = -0.12;

export interface Head {
  R: number;
  rx: number;
  ry: number;
  yaw: number;
  pitch: number;
  /** Spring lag of the floppy parts, in head units (−1…1). */
  dx: number;
  dy: number;
}

export function makeHead(R: number, yaw = 0, pitch = 0, dx = 0, dy = 0): Head {
  return { R, rx: R * 1.14, ry: R * 0.88, yaw, pitch, dx, dy };
}

type V3 = readonly [number, number, number];
interface P { x: number; y: number; z: number }

function ringR(y: number): number {
  const a = Math.min(1, Math.abs(y));
  return Math.pow(1 - Math.pow(a, EXP), 1 / EXP);
}

/** Head-local (x right, y up, z toward the viewer) → body space. */
function proj(H: Head, [x, y, z]: V3): P {
  const cy = Math.cos(H.yaw);
  const sy = Math.sin(H.yaw);
  const x1 = x * cy + z * sy;
  const z1 = -x * sy + z * cy;
  const pitch = VIEW_TILT + H.pitch * ACC_PITCH;
  const cp = Math.cos(pitch);
  const sp = Math.sin(pitch);
  return { x: x1 * H.rx, y: -(y * cp + z1 * sp) * H.ry, z: -y * sp + z1 * cp };
}

/** Point on the head surface at height y, longitude lon (0 faces the viewer). */
function surf(y: number, lon: number, s = 1): V3 {
  const r = ringR(y) * s;
  return [r * Math.sin(lon), y, r * Math.cos(lon)];
}

/** The plain body outline, without BotEngine's morph. */
export function mochiPath(rx: number, ry: number): Path2D {
  const p = new Path2D();
  const n = 96;
  const e = 2 / EXP;
  for (let i = 0; i <= n; i++) {
    const a = (i / n) * Math.PI * 2;
    const ca = Math.cos(a);
    const sa = Math.sin(a);
    const x = rx * Math.sign(ca) * Math.pow(Math.abs(ca), e);
    const y = ry * Math.sign(sa) * Math.pow(Math.abs(sa), e);
    if (i) p.lineTo(x, y);
    else p.moveTo(x, y);
  }
  p.closePath();
  return p;
}

type Stops = readonly (readonly [number, string])[];

function lin(x: CanvasRenderingContext2D, x0: number, y0: number, x1: number, y1: number, stops: Stops) {
  const g = x.createLinearGradient(x0, y0, x1, y1);
  for (const [o, c] of stops) g.addColorStop(o, c);
  return g;
}

function rad(x: CanvasRenderingContext2D, cx: number, cy: number, r0: number, r1: number, stops: Stops) {
  const g = x.createRadialGradient(cx, cy, r0, cx, cy, r1);
  for (const [o, c] of stops) g.addColorStop(o, c);
  return g;
}

function roundRect(x: CanvasRenderingContext2D, X: number, Y: number, W: number, H: number, r: number) {
  x.beginPath();
  x.moveTo(X + r, Y);
  x.arcTo(X + W, Y, X + W, Y + H, r);
  x.arcTo(X + W, Y + H, X, Y + H, r);
  x.arcTo(X, Y + H, X, Y, r);
  x.arcTo(X, Y, X + W, Y, r);
  x.closePath();
}

/**
 * The front half of a projected ring, left to right: the run between the two
 * silhouette extremes that sits nearer the viewer. Sorting by x instead breaks
 * as soon as the ring is seen from above.
 */
function frontOf(pts: P[]): P[] {
  const n = pts.length;
  if (n < 2) return pts;
  let lo = 0;
  let hi = 0;
  for (let i = 1; i < n; i++) {
    if (pts[i].x < pts[lo].x) lo = i;
    if (pts[i].x > pts[hi].x) hi = i;
  }
  if (lo === hi) return [pts[lo]];
  const walk = (step: number) => {
    const out: P[] = [];
    for (let i = lo; out.length <= n; i = (i + step + n) % n) {
      out.push(pts[i]);
      if (i === hi) break;
    }
    return out;
  };
  const a = walk(1);
  const b = walk(-1);
  const meanZ = (r: P[]) => r.reduce((s, q) => s + q.z, 0) / r.length;
  return meanZ(a) >= meanZ(b) ? a : b;
}

function frontArc(H: Head, y: number, s: number): P[] {
  const pts: P[] = [];
  for (let i = 0; i < 120; i++) pts.push(proj(H, surf(y, -Math.PI + (i / 120) * 2 * Math.PI, s)));
  return frontOf(pts);
}

/** The part of the head above the front arc of ring y: what a cap covers. */
function capClip(H: Head, y: number, s: number, extraTop = 3): Path2D {
  const arc = frontArc(H, y, s);
  const p = new Path2D();
  p.moveTo(arc[0].x - H.rx, arc[0].y);
  for (const q of arc) p.lineTo(q.x, q.y);
  const last = arc[arc.length - 1];
  p.lineTo(last.x + H.rx, last.y);
  p.lineTo(H.rx * 2, -H.ry * extraTop);
  p.lineTo(-H.rx * 2, -H.ry * extraTop);
  p.closePath();
  return p;
}

/** Everything but `p`; clip with "evenodd". */
function invert(p: Path2D, H: Head): Path2D {
  const q = new Path2D();
  q.rect(-H.rx * 4, -H.ry * 4, H.rx * 8, H.ry * 8);
  q.addPath(p);
  return q;
}

function polyline(x: CanvasRenderingContext2D, pts: readonly { x: number; y: number }[]) {
  x.beginPath();
  pts.forEach((q, i) => (i ? x.lineTo(q.x, q.y) : x.moveTo(q.x, q.y)));
}

/** Soft round pompom made of overlapping puffs. */
function pompom(x: CanvasRenderingContext2D, px: number, py: number, r: number, base = "#FFFFFF", shade = "#D5D9E2") {
  x.save();
  x.translate(px, py);
  for (let i = 0; i < 11; i++) {
    const a = (i / 11) * Math.PI * 2;
    const br = r * (0.34 + 0.06 * Math.sin(i * 2.3));
    const bx = Math.cos(a) * r * 0.78;
    const by = Math.sin(a) * r * 0.78;
    x.fillStyle = rad(x, bx - br * 0.4, by - br * 0.5, 0, br * 1.3, [[0, base], [1, shade]]);
    x.beginPath();
    x.arc(bx, by, br, 0, Math.PI * 2);
    x.fill();
  }
  x.fillStyle = rad(x, -r * 0.3, -r * 0.35, 0, r * 1.05, [[0, base], [0.7, base], [1, shade]]);
  x.beginPath();
  x.arc(0, 0, r * 0.86, 0, Math.PI * 2);
  x.fill();
  x.restore();
}

/** Fuzzy band along a polyline (the Santa hat's trim). */
function fuzzyBand(x: CanvasRenderingContext2D, arc: P[], thick: number, base = "#FFFFFF", shade = "#DADDE4") {
  x.save();
  x.lineJoin = "round";
  x.lineCap = "round";
  polyline(x, arc);
  x.strokeStyle = shade;
  x.lineWidth = thick;
  x.stroke();
  polyline(x, arc);
  x.strokeStyle = base;
  x.lineWidth = thick * 0.78;
  x.stroke();
  const step = Math.max(2, Math.floor(arc.length / 16));
  for (let i = 0; i < arc.length; i += step) {
    const q = arc[i];
    const r = thick * (0.32 + 0.1 * Math.sin(i * 1.7));
    x.fillStyle = rad(x, q.x - r * 0.3, q.y - thick * 0.35 - r * 0.3, 0, r * 1.2, [[0, base], [1, shade]]);
    x.beginPath();
    x.arc(q.x, q.y - thick * 0.32, r, 0, Math.PI * 2);
    x.fill();
  }
  x.restore();
}

interface EyeFrame { sd: number; visible: boolean; x: number; y: number; fx: number; fy: number }

/** Same formula as BotEngine.drawEyes, so glasses stay on the eyes. */
function eyeFrames(H: Head): EyeFrame[] {
  return [-1, 1].map((sd) => {
    const eyeYaw = sd * EYE_SP + H.yaw;
    const eyePitch = EYE_P + H.pitch;
    const cp = Math.cos(eyePitch);
    return {
      sd,
      visible: Math.cos(eyeYaw) * cp > 0.04,
      x: Math.sin(eyeYaw) * cp * H.rx,
      y: -Math.sin(eyePitch) * H.ry,
      fx: Math.max(0.18, Math.cos(eyeYaw)),
      fy: Math.max(0.18, cp),
    };
  });
}

// ── Accessories ───────────────────────────────────────────────────────────────
// `body` is the head outline; `simple` drops the fine detail below R = 16 px.

type Draw = (x: CanvasRenderingContext2D, H: Head, body: Path2D, simple: boolean) => void;

const beanie: Draw = (x, H, body, simple) => {
  const s = 1.035;
  const yEdge = 0.42;
  const yCuff = 0.58;
  const head = mochiPath(H.rx * s, H.ry * s);

  x.save();
  x.clip(body);
  x.clip(capClip(H, yEdge - 0.12, 1));
  x.fillStyle = "rgba(30,40,70,0.10)";
  x.fill(body);
  x.restore();

  x.save();
  x.clip(capClip(H, yCuff, s));
  x.fillStyle = lin(x, H.rx * 0.5, -H.ry * 1.1, -H.rx * 0.6, H.ry * 0.2, [[0, "#7DB6FF"], [1, "#2F6FE0"]]);
  x.fill(head);
  if (!simple) {
    x.clip(head);
    for (let k = -6; k <= 6; k++) {
      const lon = k * 0.24;
      const pts: P[] = [];
      for (let i = 0; i <= 16; i++) {
        const q = proj(H, surf(yCuff + ((1.05 - yCuff) * i) / 16, lon, s));
        if (q.z > 0) pts.push(q);
      }
      if (pts.length < 2) continue;
      polyline(x, pts);
      x.strokeStyle = "rgba(20,50,140,0.16)";
      x.lineWidth = H.R * 0.045;
      x.stroke();
    }
  }
  x.restore();

  const cs = s * 1.04;
  const cuff = mochiPath(H.rx * cs, H.ry * cs);
  x.save();
  x.clip(capClip(H, yEdge, cs));
  x.clip(invert(capClip(H, yCuff, cs), H), "evenodd");
  x.fillStyle = lin(x, 0, -H.ry * 0.6, 0, -H.ry * 0.2, [[0, "#3C7BEA"], [1, "#2257C4"]]);
  x.fill(cuff);
  x.clip(cuff);
  for (let k = -14; k <= 14; k++) {
    const lon = k * 0.115;
    const a = proj(H, surf(yEdge, lon, cs));
    const b = proj(H, surf(yCuff, lon, cs));
    if (a.z < 0) continue;
    x.beginPath();
    x.moveTo(a.x, a.y);
    x.lineTo(b.x, b.y);
    x.strokeStyle = "rgba(10,30,100,0.22)";
    x.lineWidth = H.R * 0.035;
    x.stroke();
  }
  x.restore();

  x.save();
  x.clip(capClip(H, yCuff, s));
  x.clip(head);
  x.fillStyle = rad(x, H.rx * 0.3, -H.ry * 0.85, 0, H.R * 0.45, [[0, "rgba(255,255,255,0.35)"], [1, "rgba(255,255,255,0)"]]);
  x.fill(head);
  x.restore();

  const top = proj(H, [0, 1.08 * s, 0]);
  pompom(x, top.x + H.dx * H.rx * 0.25, top.y - H.R * 0.12 + H.dy * H.ry * 0.15, H.R * 0.24);
};

const santaHat: Draw = (x, H, body) => {
  const s = 1.05;
  const yEdge = 0.52;
  const arc = frontArc(H, yEdge, s);
  const L = arc[0];
  const Rt = arc[arc.length - 1];
  const crown = proj(H, [0, 1.05, 0]);
  const tip = { x: crown.x + H.rx * (0.95 + H.dx * 0.35), y: crown.y + H.ry * (0.05 + H.dy * 0.2) };
  const peak = { x: crown.x + H.rx * 0.25, y: crown.y - H.ry * 0.62 };
  const bag = new Path2D();
  bag.moveTo(L.x, L.y);
  bag.bezierCurveTo(L.x - H.rx * 0.05, L.y - H.ry * 0.7, peak.x - H.rx * 0.55, peak.y - H.ry * 0.05, peak.x, peak.y);
  bag.quadraticCurveTo(tip.x - H.rx * 0.05, peak.y - H.ry * 0.02, tip.x, tip.y);
  bag.quadraticCurveTo(tip.x - H.rx * 0.12, tip.y - H.ry * 0.22, peak.x + H.rx * 0.18, peak.y + H.ry * 0.32);
  bag.bezierCurveTo(Rt.x + H.rx * 0.05, peak.y + H.ry * 0.45, Rt.x + H.rx * 0.08, Rt.y - H.ry * 0.35, Rt.x, Rt.y);
  for (let i = arc.length - 1; i >= 0; i--) bag.lineTo(arc[i].x, arc[i].y);
  bag.closePath();

  x.save();
  x.clip(body);
  x.clip(capClip(H, yEdge - 0.14, 1));
  x.fillStyle = "rgba(120,10,10,0.10)";
  x.fill(body);
  x.restore();

  x.fillStyle = lin(x, -H.rx * 0.6, -H.ry * 1.6, H.rx * 0.7, -H.ry * 0.3, [[0, "#FF6B6B"], [0.55, "#E53935"], [1, "#B71C1C"]]);
  x.fill(bag);

  x.save();
  x.clip(bag);
  x.lineCap = "round";
  for (const [a, b, w] of [[0.15, 0.55, 0.1], [0.45, 0.85, 0.08]]) {
    x.beginPath();
    x.moveTo(peak.x - H.rx * 0.1 + (Rt.x - L.x) * a * 0.3, peak.y + H.ry * 0.15);
    x.quadraticCurveTo(peak.x + H.rx * 0.35, peak.y + H.ry * (0.05 + a * 0.3), tip.x - H.rx * (0.45 - b * 0.3), tip.y - H.ry * 0.12);
    x.strokeStyle = "rgba(90,0,0,0.20)";
    x.lineWidth = H.R * w;
    x.stroke();
  }
  x.fillStyle = rad(x, peak.x - H.rx * 0.25, peak.y + H.ry * 0.05, 0, H.R * 0.5, [[0, "rgba(255,255,255,0.32)"], [1, "rgba(255,255,255,0)"]]);
  x.fill(bag);
  x.restore();

  fuzzyBand(x, arc, H.R * 0.3);
  pompom(x, tip.x, tip.y + H.R * 0.04, H.R * 0.22);
};

const partyHat: Draw = (x, H, _body, simple) => {
  const baseY = 0.82;
  const baseR = 0.42;
  const lean = -0.24 + H.dx * 0.12;
  const c = proj(H, [0.16, baseY + 0.06, 0]);
  const ring: P[] = [];
  for (let i = 0; i <= 48; i++) {
    const a = (i / 48) * Math.PI * 2;
    ring.push(proj(H, [0.16 + baseR * Math.sin(a), baseY + 0.06, baseR * Math.cos(a)]));
  }
  const left = ring.reduce((m, q) => (q.x < m.x ? q : m));
  const right = ring.reduce((m, q) => (q.x > m.x ? q : m));
  const hgt = H.ry * 1.6;
  const apex = { x: c.x + Math.sin(lean) * hgt, y: c.y - Math.cos(lean) * hgt };
  const front = frontOf(ring);

  const cone = new Path2D();
  cone.moveTo(left.x, left.y);
  cone.quadraticCurveTo((left.x + apex.x) / 2 - H.rx * 0.06, (left.y + apex.y) / 2, apex.x - H.R * 0.05, apex.y + H.R * 0.06);
  cone.quadraticCurveTo(apex.x, apex.y - H.R * 0.03, apex.x + H.R * 0.05, apex.y + H.R * 0.06);
  cone.quadraticCurveTo((right.x + apex.x) / 2 + H.rx * 0.06, (right.y + apex.y) / 2, right.x, right.y);
  for (let i = front.length - 1; i >= 0; i--) cone.lineTo(front[i].x, front[i].y);
  cone.closePath();
  x.fillStyle = lin(x, left.x, apex.y, right.x, left.y, [[0, "#FF9BD0"], [0.5, "#F15BAE"], [1, "#C2187A"]]);
  x.fill(cone);

  x.save();
  x.clip(cone);
  if (!simple) {
    const dots = [[0.25, -0.35], [0.3, 0.3], [0.55, -0.05], [0.72, 0.28], [0.8, -0.3], [0.45, 0.6], [0.48, -0.65]];
    for (const [t, u] of dots) {
      const bx = left.x + (right.x - left.x) * (0.5 + u * 0.5);
      const by = left.y + (right.y - left.y) * (0.5 + u * 0.5);
      const r = H.R * 0.075 * (0.6 + t * 0.5);
      x.beginPath();
      x.ellipse(bx + (apex.x - bx) * (1 - t), by + (apex.y - by) * (1 - t), r, r * 0.9, 0, 0, Math.PI * 2);
      x.fillStyle = "rgba(255,255,255,0.92)";
      x.fill();
    }
  }
  x.fillStyle = lin(x, left.x, 0, right.x, 0, [[0, "rgba(255,255,255,0.28)"], [0.35, "rgba(255,255,255,0)"], [1, "rgba(80,0,40,0.18)"]]);
  x.fill(cone);
  x.restore();

  polyline(x, front);
  x.strokeStyle = "#FFD84D";
  x.lineWidth = H.R * 0.07;
  x.lineCap = "round";
  x.stroke();
  pompom(x, apex.x, apex.y - H.R * 0.04, H.R * 0.16, "#FFE27A", "#F2B705");
};

const CROWN = { s: 1.06, yb: 0.46, yt: 0.66, n: 8, spikeH: 0.42 };

/** side −1: the half behind the head, drawn before the body; +1: the front. */
function crownPart(x: CanvasRenderingContext2D, H: Head, side: number, simple: boolean) {
  const { s, yb, yt, n, spikeH } = CROWN;
  const seg: { b: P; tt: P }[] = [];
  for (let i = 0; i <= 120; i++) {
    const lon = -Math.PI + (i / 120) * 2 * Math.PI;
    const b = proj(H, surf(yb, lon, s));
    const phase = ((lon + Math.PI) / (2 * Math.PI)) * n;
    const f = phase - Math.floor(phase);
    const spike = Math.pow(Math.max(0, 1 - Math.abs(f - 0.5) * 2), 1.6);
    const sp = surf(yt, lon, s);
    const tt = proj(H, [sp[0] * (1 - 0.08 * spike), yt + spikeH * spike, sp[2] * (1 - 0.08 * spike)]);
    if (side > 0 ? b.z >= 0 : b.z < 0.02) seg.push({ b, tt });
  }
  if (seg.length < 2) return;
  seg.sort((p, q) => p.b.x - q.b.x);
  const shape = new Path2D();
  seg.forEach((q, i) => (i ? shape.lineTo(q.tt.x, q.tt.y) : shape.moveTo(q.tt.x, q.tt.y)));
  for (let i = seg.length - 1; i >= 0; i--) shape.lineTo(seg[i].b.x, seg[i].b.y);
  shape.closePath();
  const dark = side < 0;
  x.fillStyle = lin(x, 0, -H.ry * 1.05, 0, -H.ry * 0.45, dark
    ? [[0, "#C98A12"], [1, "#8A5A06"]]
    : [[0, "#FFE58A"], [0.5, "#FBBF24"], [1, "#D08A0B"]]);
  x.fill(shape);
  if (dark) return;

  x.save();
  x.clip(shape);
  x.fillStyle = lin(x, -H.rx, 0, H.rx, 0, [
    [0, "rgba(120,70,0,0.25)"], [0.45, "rgba(255,255,255,0)"], [0.62, "rgba(255,255,255,0.35)"], [1, "rgba(120,70,0,0.25)"],
  ]);
  x.fill(shape);
  x.restore();
  if (simple) return;

  const gems = ["#EF4444", "#3B82F6", "#22C55E", "#A855F7"];
  for (let k = 0; k < n; k++) {
    const lon = -Math.PI + ((k + 0.5) / n) * 2 * Math.PI;
    const sp = surf(yt, lon, s);
    const tipP = proj(H, [sp[0] * 0.92, yt + spikeH, sp[2] * 0.92]);
    const mid = proj(H, surf((yb + yt) / 2, lon, s * 1.01));
    if (mid.z <= 0.12) continue;
    const r = H.R * 0.055;
    x.beginPath();
    x.arc(tipP.x, tipP.y - r * 0.5, r, 0, Math.PI * 2);
    x.fillStyle = rad(x, tipP.x - r * 0.3, tipP.y - r, 0, r * 1.2, [[0, "#FFF6CC"], [1, "#E0A21A"]]);
    x.fill();
    const gr = H.R * 0.075;
    x.beginPath();
    x.ellipse(mid.x, mid.y, gr * Math.max(0.35, mid.z), gr, 0, 0, Math.PI * 2);
    x.fillStyle = gems[k % gems.length];
    x.fill();
    x.beginPath();
    x.arc(mid.x - gr * 0.25 * mid.z, mid.y - gr * 0.35, gr * 0.28, 0, Math.PI * 2);
    x.fillStyle = "rgba(255,255,255,0.75)";
    x.fill();
  }
}

const crownBack: Draw = (x, H, _body, simple) => crownPart(x, H, -1, simple);

const crownFront: Draw = (x, H, body, simple) => {
  x.save();
  x.clip(body);
  x.clip(capClip(H, CROWN.yb - 0.1, 1));
  x.clip(invert(capClip(H, CROWN.yb, 1), H), "evenodd");
  x.fillStyle = "rgba(80,50,0,0.12)";
  x.fill(body);
  x.restore();
  crownPart(x, H, 1, simple);
};

function witchBrim(H: Head): Path2D {
  const brim = new Path2D();
  for (let i = 0; i <= 120; i++) {
    const a = -Math.PI + (i / 120) * 2 * Math.PI;
    const wob = 1 + 0.035 * Math.sin(a * 3 + 0.6);
    const droop = -0.1 * Math.pow(Math.abs(Math.sin(a)), 2);
    const q = proj(H, [1.42 * wob * Math.sin(a), 0.7 + droop, 1.42 * wob * Math.cos(a)]);
    if (i) brim.lineTo(q.x, q.y);
    else brim.moveTo(q.x, q.y);
  }
  brim.closePath();
  return brim;
}

function witchBrimFront(H: Head): P[] {
  const pts: P[] = [];
  for (let i = 0; i <= 120; i++) {
    const a = -Math.PI + (i / 120) * 2 * Math.PI;
    const wob = 1 + 0.035 * Math.sin(a * 3 + 0.6);
    const droop = -0.1 * Math.pow(Math.abs(Math.sin(a)), 2);
    const q = proj(H, [1.42 * wob * Math.sin(a), 0.7 + droop, 1.42 * wob * Math.cos(a)]);
    if (q.z >= 0) pts.push(q);
  }
  return pts.sort((p, q) => p.x - q.x);
}

/** The whole brim behind the head; its front half is drawn again over it. */
const witchHatBack: Draw = (x, H) => {
  x.fillStyle = lin(x, 0, -H.ry, 0, -H.ry * 0.4, [[0, "#2A0A4F"], [1, "#3B0F6B"]]);
  x.fill(witchBrim(H));
};

const witchHatFront: Draw = (x, H, body) => {
  x.save();
  x.clip(body);
  x.clip(capClip(H, 0.5, 1));
  x.fillStyle = "rgba(40,0,70,0.10)";
  x.fill(body);
  x.restore();

  x.fillStyle = lin(x, 0, -H.ry * 0.9, 0, -H.ry * 0.3, [[0, "#5B21B6"], [1, "#3B0764"]]);
  x.fill(witchBrim(H));
  polyline(x, witchBrimFront(H));
  x.strokeStyle = "rgba(190,150,255,0.35)";
  x.lineWidth = H.R * 0.035;
  x.stroke();

  const baseR = 0.62;
  const by = 0.74;
  const bl = proj(H, [-baseR, by, 0]);
  const br = proj(H, [baseR, by, 0]);
  const c = proj(H, [0, by, 0]);
  const lean = 0.1 + H.dx * 0.15;
  const top = { x: c.x + H.rx * 0.18 + Math.sin(lean) * H.ry * 0.3, y: c.y - H.ry * 1.25 };
  const tip = { x: top.x + H.rx * (0.45 + H.dx * 0.25), y: top.y + H.ry * (0.22 + H.dy * 0.1) };
  const cone = new Path2D();
  cone.moveTo(bl.x, bl.y);
  cone.bezierCurveTo(bl.x + H.rx * 0.12, bl.y - H.ry * 0.5, top.x - H.rx * 0.28, top.y + H.ry * 0.25, top.x - H.rx * 0.02, top.y - H.ry * 0.02);
  cone.quadraticCurveTo(top.x + H.rx * 0.25, top.y - H.ry * 0.08, tip.x, tip.y);
  cone.quadraticCurveTo(top.x + H.rx * 0.22, top.y + H.ry * 0.08, top.x + H.rx * 0.14, top.y + H.ry * 0.22);
  cone.bezierCurveTo(br.x - H.rx * 0.18, c.y - H.ry * 0.45, br.x - H.rx * 0.02, br.y - H.ry * 0.2, br.x, br.y);
  const capFront = frontArc(H, by, baseR / ringR(by)).filter((q) => q.x >= bl.x - 1 && q.x <= br.x + 1);
  for (let i = capFront.length - 1; i >= 0; i--) cone.lineTo(capFront[i].x, capFront[i].y);
  cone.closePath();
  x.fillStyle = lin(x, bl.x, top.y, br.x, bl.y, [[0, "#7C3AED"], [0.55, "#4C1D95"], [1, "#2E1065"]]);
  x.fill(cone);

  x.save();
  x.clip(cone);
  x.fillStyle = lin(x, bl.x, 0, br.x, 0, [[0, "rgba(255,255,255,0.22)"], [0.4, "rgba(255,255,255,0)"], [1, "rgba(0,0,0,0.15)"]]);
  x.fill(cone);
  x.beginPath();
  x.moveTo(top.x - H.rx * 0.05, top.y + H.ry * 0.05);
  x.quadraticCurveTo(top.x + H.rx * 0.1, top.y + H.ry * 0.12, top.x + H.rx * 0.2, top.y + H.ry * 0.06);
  x.strokeStyle = "rgba(20,0,40,0.35)";
  x.lineWidth = H.R * 0.05;
  x.lineCap = "round";
  x.stroke();
  const fc = proj(H, [0, by, baseR]);
  const lift = H.ry * 0.11;
  x.beginPath();
  x.moveTo(bl.x - 2, bl.y - lift);
  x.quadraticCurveTo(fc.x, 2 * (fc.y - lift) - (bl.y + br.y) / 2, br.x + 2, br.y - lift);
  x.strokeStyle = "#F97316";
  x.lineWidth = H.ry * 0.17;
  x.lineCap = "butt";
  x.stroke();
  x.restore();

  const bw = H.R * 0.2;
  const bh = H.R * 0.16;
  x.save();
  x.translate(fc.x, fc.y - lift);
  roundRect(x, -bw / 2, -bh / 2, bw, bh, bh * 0.25);
  x.fillStyle = "#FCD34D";
  x.fill();
  roundRect(x, -bw / 2 + bw * 0.24, -bh / 2 + bh * 0.28, bw * 0.52, bh * 0.44, bh * 0.1);
  x.fillStyle = "#C2410C";
  x.fill();
  x.restore();
};

function lens(x: CanvasRenderingContext2D, e: EyeFrame, w: number, h: number, r: number) {
  x.save();
  x.translate(e.x, e.y);
  x.scale(e.fx, e.fy);
  roundRect(x, -w / 2, -h / 2, w, h, r);
  x.restore();
}

const sunglasses: Draw = (x, H, body) => {
  const eyes = eyeFrames(H);
  const w = H.R * 0.62;
  const h = H.R * 0.46;
  x.save();
  x.clip(body);
  const [l, r] = eyes;
  if (l.visible && r.visible) {
    x.beginPath();
    x.moveTo(l.x + (w / 2) * l.fx * 0.9, l.y - h * 0.18);
    x.quadraticCurveTo((l.x + r.x) / 2, (l.y + r.y) / 2 - h * 0.42, r.x - (w / 2) * r.fx * 0.9, r.y - h * 0.18);
    x.strokeStyle = "#111317";
    x.lineWidth = H.R * 0.07;
    x.stroke();
  }
  for (const e of eyes) {
    if (!e.visible) continue;
    x.beginPath();
    x.moveTo(e.x + (e.sd * w) / 2 * e.fx, e.y - h * 0.2);
    x.lineTo(e.sd * H.rx * 1.05, e.y - h * 0.35);
    x.strokeStyle = "#111317";
    x.lineWidth = H.R * 0.06;
    x.stroke();
  }
  for (const e of eyes) {
    if (!e.visible) continue;
    lens(x, e, w, h, h * 0.42);
    x.fillStyle = "rgba(17,19,23,0.82)";
    x.fill();
    x.lineWidth = H.R * 0.05;
    x.strokeStyle = "#0B0C0F";
    x.stroke();
    x.save();
    x.translate(e.x, e.y);
    x.scale(e.fx, e.fy);
    x.beginPath();
    x.moveTo(-w * 0.28, -h * 0.05);
    x.lineTo(-w * 0.05, -h * 0.3);
    x.strokeStyle = "rgba(255,255,255,0.45)";
    x.lineWidth = H.R * 0.05;
    x.lineCap = "round";
    x.stroke();
    x.restore();
  }
  x.restore();
};

const roundGlasses: Draw = (x, H, body) => {
  const eyes = eyeFrames(H);
  const d = H.R * 0.56;
  x.save();
  x.clip(body);
  const [l, r] = eyes;
  if (l.visible && r.visible) {
    x.beginPath();
    x.moveTo(l.x + (d / 2) * l.fx, l.y - d * 0.08);
    x.quadraticCurveTo((l.x + r.x) / 2, (l.y + r.y) / 2 - d * 0.3, r.x - (d / 2) * r.fx, r.y - d * 0.08);
    x.strokeStyle = "#8A4B12";
    x.lineWidth = H.R * 0.055;
    x.stroke();
  }
  for (const e of eyes) {
    if (!e.visible) continue;
    x.beginPath();
    x.moveTo(e.x + (e.sd * d) / 2 * e.fx, e.y - d * 0.1);
    x.lineTo(e.sd * H.rx * 1.05, e.y - d * 0.25);
    x.strokeStyle = "#8A4B12";
    x.lineWidth = H.R * 0.05;
    x.stroke();
  }
  for (const e of eyes) {
    if (!e.visible) continue;
    x.save();
    x.translate(e.x, e.y);
    x.scale(e.fx, e.fy);
    x.beginPath();
    x.arc(0, 0, d / 2, 0, Math.PI * 2);
    x.fillStyle = "rgba(190,225,255,0.18)";
    x.fill();
    x.lineWidth = H.R * 0.065;
    x.strokeStyle = "#9A5A1A";
    x.stroke();
    x.beginPath();
    x.arc(0, 0, d / 2 - H.R * 0.03, Math.PI * 1.1, Math.PI * 1.45);
    x.strokeStyle = "rgba(255,255,255,0.55)";
    x.lineWidth = H.R * 0.03;
    x.stroke();
    x.restore();
  }
  x.restore();
};

const scarf: Draw = (x, H) => {
  const s = 1.05;
  const y0 = -0.34;
  const y1 = -0.66;
  const top = frontArc(H, y0, s);
  const bot = frontArc(H, y1, s);
  const band = new Path2D();
  top.forEach((q, i) => (i ? band.lineTo(q.x, q.y) : band.moveTo(q.x, q.y)));
  for (let i = bot.length - 1; i >= 0; i--) band.lineTo(bot[i].x, bot[i].y);
  band.closePath();

  x.save();
  x.clip(mochiPath(H.rx * s, H.ry * s));
  x.fillStyle = lin(x, 0, -H.ry * 0.2, 0, H.ry * 0.7, [[0, "#F87171"], [1, "#B91C1C"]]);
  x.fill(band);
  x.clip(band);
  for (const lon of [-1.0, -0.45, 0.1, 0.65, 1.2]) {
    const a = proj(H, surf(y0, lon, s));
    const b = proj(H, surf(y1, lon, s));
    if (a.z < 0) continue;
    x.beginPath();
    x.moveTo(a.x, a.y - 4);
    x.lineTo(b.x, b.y + 4);
    x.strokeStyle = "rgba(255,255,255,0.85)";
    x.lineWidth = H.R * 0.09 * Math.max(0.3, a.z);
    x.stroke();
  }
  x.fillStyle = lin(x, 0, -H.ry * 0.5, 0, H.ry * 0.3, [[0, "rgba(255,255,255,0.18)"], [1, "rgba(0,0,0,0.1)"]]);
  x.fill(band);
  x.restore();

  const k = proj(H, surf((y0 + y1) / 2, -0.55, s * 1.03));
  if (k.z <= 0) return;
  const sw = H.dx * H.rx * 0.12;
  const end = new Path2D();
  end.moveTo(k.x - H.R * 0.16, k.y);
  end.quadraticCurveTo(k.x - H.R * 0.24 + sw, k.y + H.ry * 0.35, k.x - H.R * 0.2 + sw * 1.4, k.y + H.ry * 0.62);
  end.lineTo(k.x + H.R * 0.06 + sw * 1.4, k.y + H.ry * 0.6);
  end.quadraticCurveTo(k.x + H.R * 0.02 + sw, k.y + H.ry * 0.3, k.x + H.R * 0.12, k.y);
  end.closePath();
  x.fillStyle = lin(x, 0, k.y, 0, k.y + H.ry * 0.6, [[0, "#EF4444"], [1, "#B91C1C"]]);
  x.fill(end);
  x.save();
  x.clip(end);
  x.fillStyle = "rgba(255,255,255,0.85)";
  for (const t of [0.35, 0.7]) x.fillRect(k.x - H.R * 0.4 + sw, k.y + H.ry * 0.62 * t, H.R * 0.8, H.R * 0.07);
  x.restore();
  for (let i = 0; i < 4; i++) {
    const fx = k.x - H.R * 0.17 + sw * 1.4 + i * H.R * 0.075;
    x.beginPath();
    x.moveTo(fx, k.y + H.ry * 0.6);
    x.lineTo(fx, k.y + H.ry * 0.72);
    x.strokeStyle = "#DC2626";
    x.lineWidth = H.R * 0.035;
    x.lineCap = "round";
    x.stroke();
  }
  x.beginPath();
  x.ellipse(k.x, k.y, H.R * 0.17, H.R * 0.14, 0.2, 0, Math.PI * 2);
  x.fillStyle = rad(x, k.x - H.R * 0.05, k.y - H.R * 0.05, 0, H.R * 0.2, [[0, "#F87171"], [1, "#B91C1C"]]);
  x.fill();
};

export const PUMPKIN_BODY = ["#FFA94D", "#E8590C"] as const;

const pumpkin: Draw = (x, H, body, simple) => {
  if (!simple) {
    x.save();
    x.clip(body);
    for (const lon of [-1.15, -0.55, 0.0, 0.55, 1.15]) {
      const pts: P[] = [];
      for (let i = 0; i <= 30; i++) {
        const q = proj(H, surf(-0.98 + (1.96 * i) / 30, lon, 1));
        if (q.z > 0) pts.push(q);
      }
      if (pts.length < 2) continue;
      const zz = pts[Math.floor(pts.length / 2)].z;
      polyline(x, pts);
      x.strokeStyle = `rgba(150,50,0,${0.22 * zz})`;
      x.lineWidth = H.R * 0.12;
      x.lineCap = "round";
      x.stroke();
      x.save();
      x.translate(H.R * 0.07, 0);
      x.strokeStyle = `rgba(255,220,170,${0.18 * zz})`;
      x.lineWidth = H.R * 0.04;
      x.stroke();
      x.restore();
    }
    x.restore();
  }
  const t = proj(H, [0.02, 1.0, 0]);
  const R = H.R;
  x.beginPath();
  x.moveTo(t.x - R * 0.09, t.y + R * 0.04);
  x.quadraticCurveTo(t.x - R * 0.08, t.y - R * 0.22, t.x + R * 0.08, t.y - R * 0.3);
  x.lineTo(t.x + R * 0.13, t.y - R * 0.22);
  x.quadraticCurveTo(t.x + R * 0.04, t.y - R * 0.15, t.x + R * 0.08, t.y + R * 0.04);
  x.closePath();
  x.fillStyle = lin(x, t.x - R * 0.1, 0, t.x + R * 0.1, 0, [[0, "#65A30D"], [1, "#3F6212"]]);
  x.fill();
  x.save();
  x.translate(t.x - R * 0.06, t.y - R * 0.02);
  x.rotate(-0.5);
  x.beginPath();
  x.moveTo(0, 0);
  x.quadraticCurveTo(-R * 0.18, -R * 0.2, -R * 0.38, -R * 0.02);
  x.quadraticCurveTo(-R * 0.18, R * 0.1, 0, 0);
  x.fillStyle = lin(x, 0, -R * 0.15, -R * 0.3, 0, [[0, "#84CC16"], [1, "#4D7C0F"]]);
  x.fill();
  x.beginPath();
  x.moveTo(-R * 0.02, -R * 0.01);
  x.quadraticCurveTo(-R * 0.18, -R * 0.08, -R * 0.32, -R * 0.03);
  x.strokeStyle = "rgba(30,60,0,0.4)";
  x.lineWidth = R * 0.02;
  x.stroke();
  x.restore();
  x.beginPath();
  x.moveTo(t.x + R * 0.1, t.y - R * 0.12);
  x.bezierCurveTo(t.x + R * 0.3, t.y - R * 0.25, t.x + R * 0.35, t.y - R * 0.02, t.x + R * 0.22, t.y - R * 0.06);
  x.strokeStyle = "#4D7C0F";
  x.lineWidth = R * 0.03;
  x.lineCap = "round";
  x.stroke();
};

const bow: Draw = (x, H) => {
  const a = proj(H, surf(0.86, 0.55, 1.02));
  if (a.z < -0.2) return;
  const s = H.R * 0.26;
  const sq = Math.max(0.45, Math.cos(0.55 + H.yaw));
  x.save();
  x.translate(a.x, a.y);
  x.rotate(0.35 + H.yaw * 0.3);
  x.scale(sq, 1);
  for (const sd of [-1, 1]) {
    x.beginPath();
    x.moveTo(0, 0);
    x.bezierCurveTo(sd * s * 0.6, -s * 0.85, sd * s * 1.35, -s * 0.55, sd * s * 1.15, 0);
    x.bezierCurveTo(sd * s * 1.35, s * 0.55, sd * s * 0.6, s * 0.85, 0, 0);
    x.fillStyle = lin(x, 0, -s, 0, s, [[0, "#FF8CC6"], [1, "#DB2777"]]);
    x.fill();
    x.beginPath();
    x.moveTo(sd * s * 0.25, -s * 0.05);
    x.quadraticCurveTo(sd * s * 0.7, -s * 0.15, sd * s * 0.95, -s * 0.05);
    x.strokeStyle = "rgba(140,10,70,0.35)";
    x.lineWidth = s * 0.08;
    x.lineCap = "round";
    x.stroke();
  }
  x.beginPath();
  x.ellipse(0, 0, s * 0.24, s * 0.3, 0, 0, Math.PI * 2);
  x.fillStyle = rad(x, -s * 0.06, -s * 0.1, 0, s * 0.35, [[0, "#FFB3D9"], [1, "#C2185B"]]);
  x.fill();
  x.restore();
};

/** Always behind the body, poking up from the top of the head. */
const bunnyEars: Draw = (x, H) => {
  const earH = H.R * 0.85;
  for (const sd of [-1, 1]) {
    const root = proj(H, [sd * 0.45, 0.92, 0]);
    const rootL = proj(H, [sd * 0.45 - 0.22, 0.92, 0]);
    const rootR = proj(H, [sd * 0.45 + 0.22, 0.92, 0]);
    const hw = Math.max(H.R * 0.04, Math.abs(rootR.x - rootL.x) / 2);
    x.save();
    x.translate(root.x, root.y - earH * 0.15);
    x.beginPath();
    x.ellipse(0, 0, hw, earH / 2, 0, 0, Math.PI * 2);
    x.fillStyle = "#F9F0F0";
    x.fill();
    x.strokeStyle = "rgba(0,0,0,0.06)";
    x.lineWidth = 0.8;
    x.stroke();
    x.beginPath();
    x.ellipse(0, -earH / 2 + H.R * 0.1 + earH * 0.325, hw * 0.5, earH * 0.325, 0, 0, Math.PI * 2);
    x.fillStyle = "rgba(252,165,165,0.70)";
    x.fill();
    x.restore();
  }
};

const BACK: Partial<Record<Outfit, Draw>> = {
  crown: crownBack,
  witchHat: witchHatBack,
  bunnyEars,
};

const FRONT: Partial<Record<Outfit, Draw>> = {
  beanie, santaHat, partyHat, crown: crownFront, witchHat: witchHatFront,
  sunglasses, roundGlasses, scarf, pumpkin, bow,
};

const HATS: ReadonlySet<Outfit> = new Set(["beanie", "santaHat", "partyHat", "crown", "witchHat", "bunnyEars"]);

/**
 * Draws one pass of an outfit in body space (the caller has translated to the
 * body centre and applied tilt and squash). `presence` 0…1 drives the
 * entrance: hats drop in from above with a little overshoot, glasses and
 * the scarf rise into place, the bow pops.
 */
export function drawOutfit(
  x: CanvasRenderingContext2D,
  pass: "back" | "front",
  outfit: Outfit,
  H: Head,
  presence: number,
  fade = 1,
) {
  const draw = (pass === "back" ? BACK : FRONT)[outfit];
  if (!draw) return;
  const alpha = fade * Math.min(1, presence * 2.5);
  if (alpha <= 0.005) return;
  const settle = Ease.back(presence);
  x.save();
  x.globalAlpha *= alpha;
  if (HATS.has(outfit)) {
    const scale = 0.85 + 0.15 * settle;
    x.translate(0, -(1 - settle) * H.ry);
    x.scale(scale, scale);
  } else if (outfit === "sunglasses" || outfit === "roundGlasses") {
    x.translate(0, (1 - presence) * 0.25 * H.ry);
  } else if (outfit === "scarf") {
    x.translate(0, (1 - presence) * 0.3 * H.ry);
  } else if (outfit === "bow") {
    const k = Math.max(0.001, settle);
    x.scale(k, k);
  }
  draw(x, H, mochiPath(H.rx, H.ry), H.R < 16);
  x.restore();
}

/** A wardrobe tile: a small Mochi wearing `worn`, or ⊘ for "none". */
export function drawOutfitIcon(x: CanvasRenderingContext2D, size: number, choice: OutfitChoice, seasonal: Worn) {
  const c = size / 2;
  if (choice === "none") {
    const r = 6.5;
    x.save();
    x.translate(c, c);
    x.strokeStyle = "#454850";
    x.lineWidth = 1.4;
    x.lineCap = "round";
    x.beginPath();
    x.arc(0, 0, r * 0.82, 0, Math.PI * 2);
    x.stroke();
    x.beginPath();
    x.moveTo(-r * 0.56, r * 0.56);
    x.lineTo(r * 0.56, -r * 0.56);
    x.stroke();
    x.restore();
    return;
  }
  const worn: Worn = choice === "auto" ? seasonal : choice;
  const R = 10;
  const H = makeHead(R);
  const body = mochiPath(H.rx, H.ry);
  x.save();
  x.translate(c, c + R * 0.62);
  if (worn !== "none") drawOutfit(x, "back", worn, H, 1);
  const [top, bottom] = worn === "pumpkin" ? PUMPKIN_BODY : ["#EDEDEF", "#C4C5CA"];
  x.fillStyle = lin(x, H.rx * 0.7, -H.ry * 0.85, -H.rx * 0.8, H.ry * 0.9, [[0, top], [1, bottom]]);
  x.fill(body);
  x.fillStyle = rad(x, 0, 0, R * 0.15, R * 1.25, [[0, "rgba(0,0,0,0)"], [0.6, "rgba(0,0,0,0)"], [1, "rgba(0,0,0,0.2)"]]);
  x.fill(body);
  x.fillStyle = rad(x, H.rx * 0.34, -H.ry * 0.46, 0, R * 0.42, [[0, "rgba(255,255,255,0.55)"], [1, "rgba(255,255,255,0)"]]);
  x.fill(body);
  x.save();
  x.clip(body);
  x.fillStyle = "rgb(26,20,18)";
  for (const e of eyeFrames(H)) {
    if (!e.visible) continue;
    const w = R * EYE_W;
    const hh = R * EYE_H;
    x.save();
    x.translate(e.x, e.y);
    x.scale(e.fx, e.fy);
    roundRect(x, -w / 2, -hh / 2, w, hh, Math.min(w, hh) / 2);
    x.fill();
    x.restore();
  }
  x.restore();
  if (worn !== "none") drawOutfit(x, "front", worn, H, 1);
  if (choice === "auto") {
    const bw = 14;
    const bh = 6.5;
    x.translate(0, H.ry * 0.72);
    roundRect(x, -bw / 2, -bh / 2, bw, bh, bh / 2);
    x.fillStyle = "rgba(0,0,0,0.6)";
    x.fill();
    x.fillStyle = "#FFFFFF";
    x.font = `600 4.2px system-ui, "Segoe UI", sans-serif`;
    x.textAlign = "center";
    x.textBaseline = "middle";
    x.fillText("AUTO", 0, 0.3);
  }
  x.restore();
}
