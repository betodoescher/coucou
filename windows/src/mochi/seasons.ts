// The outfits Mochi can wear and the one he picks for the season on "auto".
// Same values and calendar as MochiWardrobe.swift. No imports: checked by
// scripts/outfits.check.ts under plain node.

export type Outfit =
  | "partyHat" | "beanie" | "crown" | "sunglasses" | "roundGlasses" | "bow"
  | "scarf" | "witchHat" | "pumpkin" | "santaHat" | "bunnyEars";

/** What Mochi wears once "auto" is resolved. */
export type Worn = Outfit | "none";

/** Stored in settings.json as `mochiOutfit`; the same values as macOS. */
export type OutfitChoice = "auto" | Worn;

export const OUTFIT_CHOICES: readonly OutfitChoice[] = [
  "auto", "none", "partyHat", "beanie", "crown", "sunglasses", "roundGlasses",
  "bow", "scarf", "witchHat", "pumpkin", "santaHat", "bunnyEars",
];

export const OUTFIT_NAMES: Record<OutfitChoice, string> = {
  auto: "Auto (seasons)",
  none: "None",
  partyHat: "Party hat",
  beanie: "Beanie",
  crown: "Crown",
  sunglasses: "Sunglasses",
  roundGlasses: "Round glasses",
  bow: "Bow",
  scarf: "Scarf",
  witchHat: "Witch hat",
  pumpkin: "Pumpkin",
  santaHat: "Santa hat",
  bunnyEars: "Bunny ears",
};

/** Anything unknown (a value from a newer or older build) falls back to "auto". */
export function parseOutfitChoice(raw: unknown): OutfitChoice {
  return OUTFIT_CHOICES.includes(raw as OutfitChoice) ? (raw as OutfitChoice) : "auto";
}

/** Easter Sunday (Meeus/Jones/Butcher) as [month 1–12, day]. */
export function easter(year: number): [number, number] {
  const a = year % 19;
  const b = Math.floor(year / 100);
  const c = year % 100;
  const d = Math.floor(b / 4);
  const e = b % 4;
  const f = Math.floor((b + 8) / 25);
  const g = Math.floor((b - f + 1) / 3);
  const h = (19 * a + b - d - g + 15) % 30;
  const i = Math.floor(c / 4);
  const k = c % 4;
  const l = (32 + 2 * e + 2 * i - h - k) % 7;
  const m = Math.floor((a + 11 * h + 22 * l) / 451);
  const n = h + l - 7 * m + 114;
  return [Math.floor(n / 31), (n % 31) + 1];
}

/** The outfit of the season on the user's local calendar. */
export function seasonalOutfit(date = new Date()): Worn {
  const day = date.getDate();
  const month = date.getMonth() + 1;
  const year = date.getFullYear();
  if ((month === 12 && day === 31) || (month === 1 && day <= 2)) return "partyHat";
  if (month === 12 && day <= 26) return "santaHat";
  if (month === 10 || (month === 11 && day === 1)) return "witchHat";
  const [em, ed] = easter(year);
  const delta = Math.round((Date.UTC(year, month - 1, day) - Date.UTC(year, em - 1, ed)) / 86_400_000);
  if (delta >= -2 && delta <= 1) return "bunnyEars";
  if ((month === 6 && day >= 21) || month === 7 || month === 8) return "sunglasses";
  return "none";
}
