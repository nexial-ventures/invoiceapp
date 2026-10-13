import { describe, expect, it } from "vitest";
import type { LocationQuery } from "vue-router";
import { currentMonth, fromQuery, presetOf, quarterOf, samePeriod, shift, switchKind, toQuery } from "./period";

const NOW = new Date(2026, 9, 13, 12);

describe("overview period", () => {
  it("presets contain the day", () => {
    expect(currentMonth(NOW)).toEqual({ kind: "month", from: "2026-10-01", to: "2026-10-31" });
    expect(presetOf("quarter", NOW)).toEqual({ kind: "quarter", from: "2026-10-01", to: "2026-12-31" });
    expect(presetOf("year", NOW)).toEqual({ kind: "year", from: "2026-01-01", to: "2026-12-31" });
    expect(presetOf("month", new Date(2028, 1, 10))).toEqual({ kind: "month", from: "2028-02-01", to: "2028-02-29" });
  });

  it("steps over year ends; a custom period does not move", () => {
    const jan = presetOf("month", new Date(2026, 0, 5));
    expect(shift(jan, -1)).toEqual({ kind: "month", from: "2025-12-01", to: "2025-12-31" });
    expect(shift(presetOf("quarter", NOW), 1)).toEqual({ kind: "quarter", from: "2027-01-01", to: "2027-03-31" });
    expect(shift(presetOf("year", NOW), -1).from).toBe("2025-01-01");
    const custom = { kind: "custom" as const, from: "2026-01-05", to: "2026-02-10" };
    expect(shift(custom, 1)).toBe(custom);
  });

  it("switches kinds around the period's start", () => {
    const may = presetOf("month", new Date(2026, 4, 20));
    expect(switchKind(may, "quarter")).toEqual({ kind: "quarter", from: "2026-04-01", to: "2026-06-30" });
    expect(quarterOf(switchKind(may, "quarter"))).toBe(2);
    expect(switchKind(may, "custom")).toEqual({ ...may, kind: "custom" });
    expect(switchKind({ kind: "custom", from: "2026-11-15", to: "2026-11-20" }, "year").from).toBe("2026-01-01");
  });

  it("reads the URL query, normalizing presets and falling back to the current month", () => {
    expect(fromQuery({}, NOW)).toEqual(currentMonth(NOW));
    expect(fromQuery({ period: "quarter", from: "2026-05-17" }, NOW)).toEqual({ kind: "quarter", from: "2026-04-01", to: "2026-06-30" });
    expect(fromQuery({ period: "custom", from: "2026-01-05", to: "2026-02-10" }, NOW)).toEqual({
      kind: "custom",
      from: "2026-01-05",
      to: "2026-02-10",
    });
    const bads: LocationQuery[] = [
      { period: "week", from: "2026-01-01" },
      { period: "month", from: "2026-02-30" },
      { period: "custom", from: "2026-03-01", to: "2026-02-01" },
      { period: "custom", from: "2026-03-01" },
    ];
    for (const bad of bads) {
      expect(fromQuery(bad, NOW)).toEqual(currentMonth(NOW));
    }
    const p = presetOf("year", NOW);
    expect(samePeriod(fromQuery(toQuery(p), NOW), p)).toBe(true);
  });
});
