// Self-check for the final-message cleanup: node scripts/steps.check.ts
import assert from "node:assert/strict";
import { unjoinSentences } from "../src/core/steps.ts";

assert.equal(
  unjoinSentences("Vou esperar ele terminar.CI verde. Faço o merge.O Mochi voltou"),
  "Vou esperar ele terminar. CI verde. Faço o merge. O Mochi voltou",
);
assert.equal(unjoinSentences("Pronto!Agora sim?Ótimo"), "Pronto! Agora sim? Ótimo", "accents and other endings");
assert.equal(unjoinSentences("Ran the tests (all green).Next up"), "Ran the tests (all green). Next up");
assert.equal(unjoinSentences("Use `React.Component` here"), "Use `React.Component` here", "inline code stays");
assert.equal(unjoinSentences("Built for the U.S.A market"), "Built for the U.S.A market", "initials stay");
assert.equal(unjoinSentences("See README.md and v0.2.1"), "See README.md and v0.2.1");
assert.equal(unjoinSentences("Already fine. Nothing to do."), "Already fine. Nothing to do.");

console.log("steps: all checks passed");
