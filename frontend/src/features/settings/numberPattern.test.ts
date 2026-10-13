import { describe, expect, it } from "vitest";
import { formatNumber, isValidPattern, parsePattern } from "./numberPattern";

describe("formatNumber", () => {
  it.each([
    ["{YYYY}{NNNN}", 2026, 7, "20260007"],
    ["D{YYYY}{NNNN}", 2026, 1, "D20260001"],
    ["FV-{YY}/{NNN}", 2026, 42, "FV-26/042"],
    ["{YY}{N}", 2009, 5, "095"],
    ["P{YYYY}{NNNN}", 2026, 12345, "P202612345"],
    ["{YY}{NNNNNNNNN}", 2026, 1, "26000000001"],
    ["a_b-{YYYY}-{N}", 2026, 0, "a_b-2026-0"],
  ])("%s / %i / %i → %s", (pattern, year, n, expected) => {
    expect(formatNumber(pattern, year, n)).toBe(expected);
  });

  it("returns null for an invalid pattern", () => {
    expect(formatNumber("{YYYY}", 2026, 1)).toBeNull();
  });
});

describe("pattern validation", () => {
  it.each([
    ["", "no counter"],
    ["{YYYY}", "no counter"],
    ["{NN}{NNN}", "two counters, no year"],
    ["{YYYY}{NN}{NNN}", "two counters"],
    ["F{NNNN}", "no year token"],
    ["{YYYY}{YY}{NNNN}", "two year tokens"],
    ["{YY}-{YY}{N}", "repeated year token"],
    ["{YYYY}{NNNNNNNNNN}", "ten N's"],
    ["{YYYY}{N", "unclosed brace"],
    ["{YYY}{N}", "unknown token"],
    ["{YYYY}{n}", "lowercase counter"],
    ["F {YY}{N}", "space literal"],
    ["F.{YY}{N}", "dot literal"],
    ["Č{YY}{N}", "non-ascii literal"],
    [`${"A".repeat(34)}{YY}{N}`, "over 40 chars"],
  ])("rejects %j (%s)", (pattern) => {
    expect(isValidPattern(pattern)).toBe(false);
  });

  it("accepts exactly 40 chars", () => {
    expect(isValidPattern(`${"A".repeat(33)}{YY}{N}`)).toBe(true);
  });

  it("merges adjacent literal characters", () => {
    expect(parsePattern("FV{YY}{N}")).toEqual([
      { kind: "literal", text: "FV" },
      { kind: "yy" },
      { kind: "n", width: 1 },
    ]);
  });
});

describe("client patterns", () => {
  it("renders {C} with the code", () => {
    expect(formatNumber("{C}-{YYYY}{NNN}", 2026, 1, "ACME")).toBe("ACME-2026001");
  });

  it("needs exactly one {C} in a client pattern and none in a global one", () => {
    expect(parsePattern("{C}{YY}{N}", true)).not.toBeNull();
    expect(parsePattern("{YY}{N}", true)).toBeNull();
    expect(parsePattern("{C}{C}{YY}{N}", true)).toBeNull();
    expect(isValidPattern("{C}{YY}{N}")).toBe(false);
  });
});
