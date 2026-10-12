import { describe, expect, it } from "vitest";
import { fromCents, isNegative, negate, parseCents, remainingAmount, sameAmount, signed, toCents } from "./money";

describe("cents", () => {
  it("parses entered amounts exactly, accepting a decimal comma, ≤ 2 dp only", () => {
    expect(toCents("1210,5")).toBe(121050n);
    expect(toCents(" 0.1 ")).toBe(10n);
    expect(toCents("-0.05")).toBe(-5n);
    expect(toCents("1.234")).toBeNull();
    expect(toCents("")).toBeNull();
    expect(toCents("1e3")).toBeNull();
  });

  it("parses any decimal rounded half away from zero", () => {
    expect(parseCents("1210.0000")).toBe(121000n);
    expect(parseCents("0.005")).toBe(1n);
    expect(parseCents("0.0049")).toBe(0n);
    expect(parseCents("-0.005")).toBe(-1n);
    expect(parseCents("-1.004")).toBe(-100n);
    expect(parseCents(".5")).toBe(50n);
    expect(parseCents("+2")).toBe(200n);
    expect(parseCents("123456789012345.67")).toBe(12345678901234567n);
    expect(parseCents("")).toBeNull();
    expect(parseCents("-")).toBeNull();
    expect(parseCents("abc")).toBeNull();
    expect(parseCents(null)).toBeNull();
  });

  it("formats cents as 2-dp strings, negatives included", () => {
    expect(fromCents(-5n)).toBe("-0.05");
    expect(fromCents(-100n)).toBe("-1.00");
    expect(fromCents(0n)).toBe("0.00");
    expect(fromCents(121050n)).toBe("1210.50");
  });

  it("compares amounts by cents", () => {
    expect(sameAmount("210", "210.00")).toBe(true);
    expect(sameAmount("-0,5", "-0.50")).toBe(true);
    expect(sameAmount(null, "0")).toBe(false);
  });

  it("computes the remaining amount exactly, never below zero", () => {
    // 0.1 + 0.2-style float traps: 1210.10 − 210.20 = 999.9000000000001 in floats.
    expect(remainingAmount("1210.10", "210.20")).toBe("999.90");
    expect(remainingAmount("0.30", "0.10")).toBe("0.20");
    expect(remainingAmount("100.00", "150.00")).toBe("0.00");
    expect(remainingAmount("-50.00", "0.00")).toBe("0.00");
    expect(remainingAmount("123456789012345.67", "0.01")).toBe("123456789012345.66");
  });
});

describe("sign helpers", () => {
  it("negates decimal strings exactly; zero stays unsigned", () => {
    expect(negate("1210.00")).toBe("-1210.00");
    expect(negate("-0.10")).toBe("0.10");
    expect(negate("0.00")).toBe("0.00");
    expect(negate("123456789012345.67")).toBe("-123456789012345.67");
  });

  it("applies a document sign", () => {
    expect(signed("100.00", -1)).toBe("-100.00");
    expect(signed("100.00", 1)).toBe("100.00");
  });

  it("detects negative amounts", () => {
    expect(isNegative("-0.01")).toBe(true);
    expect(isNegative("0.00")).toBe(false);
    expect(isNegative("-0.00")).toBe(false);
    expect(isNegative(null)).toBe(false);
  });
});
