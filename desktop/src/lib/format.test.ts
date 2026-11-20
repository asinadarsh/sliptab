import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { format, plain, type Span } from "./format.ts";

// Shared with the phone's tokeniser: core/tests/fixtures/format.json. Codes: m marker, b bold,
// i italic, u underline, s strike, l link.
const cases: { in: string; out: [string, string][] }[] = JSON.parse(
  readFileSync(new URL("../../../core/tests/fixtures/format.json", import.meta.url), "utf8"),
);
const code = (s: Span) =>
  (s.bold ? "b" : "") + (s.italic ? "i" : "") + (s.underline ? "u" : "") + (s.strike ? "s" : "") + (s.href ? "l" : "") + (s.marker ? "m" : "");
const norm = (c: string) => [...c].sort().join("");

for (const c of cases) {
  test(`format: ${JSON.stringify(c.in)}`, () => {
    assert.deepEqual(
      format(c.in).map((s) => [s.text, norm(code(s))]),
      c.out.map(([t, k]) => [t, norm(k)]),
    );
  });
}

test("plain drops markers only", () => {
  assert.equal(plain("a **bold** and __u__ https://x.io"), "a bold and u https://x.io");
});

test("www links open over https", () => {
  assert.equal(format("go www.a.io").find((s) => s.href)?.href, "https://www.a.io");
});
