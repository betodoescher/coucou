// Self-check for Mochi's seasonal outfits: node scripts/outfits.check.ts
import assert from "node:assert/strict";
import { easter, parseOutfitChoice, seasonalOutfit } from "../src/mochi/seasons.ts";

const on = (y: number, m: number, d: number) => seasonalOutfit(new Date(y, m - 1, d, 12));

assert.deepEqual(easter(2024), [3, 31]);
assert.deepEqual(easter(2025), [4, 20]);
assert.deepEqual(easter(2026), [4, 5]);
assert.deepEqual(easter(2027), [3, 28]);

assert.equal(on(2026, 12, 31), "partyHat");
assert.equal(on(2027, 1, 1), "partyHat");
assert.equal(on(2027, 1, 2), "partyHat");
assert.equal(on(2027, 1, 3), "none");
assert.equal(on(2026, 12, 1), "santaHat");
assert.equal(on(2026, 12, 26), "santaHat");
assert.equal(on(2026, 12, 27), "none");
assert.equal(on(2026, 10, 1), "witchHat");
assert.equal(on(2026, 10, 31), "witchHat");
assert.equal(on(2026, 11, 1), "witchHat");
assert.equal(on(2026, 11, 2), "none");
assert.equal(on(2026, 4, 2), "none", "Easter 2026 is 5 April: the ears start two days before");
assert.equal(on(2026, 4, 3), "bunnyEars");
assert.equal(on(2026, 4, 6), "bunnyEars");
assert.equal(on(2026, 4, 7), "none");
assert.equal(on(2026, 6, 20), "none");
assert.equal(on(2026, 6, 21), "sunglasses");
assert.equal(on(2026, 8, 31), "sunglasses");
assert.equal(on(2026, 9, 1), "none");

assert.equal(parseOutfitChoice("crown"), "crown");
assert.equal(parseOutfitChoice("none"), "none");
assert.equal(parseOutfitChoice("topHat"), "auto", "outfits removed on macOS fall back to auto");
assert.equal(parseOutfitChoice(undefined), "auto");

console.log("outfits: all checks passed");
