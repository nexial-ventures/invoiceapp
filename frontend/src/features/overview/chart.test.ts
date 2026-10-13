import { describe, expect, it } from "vitest";
import { barPath, HEIGHT, layout, WIDTH } from "./chart";

const month = (m: string, income: string, expense: string, inPeriod = true) => ({ month: m, income, expense, inPeriod });

describe("overview chart layout", () => {
  it("puts the baseline at the bottom without negatives and scales to the largest bar", () => {
    const { columns, zero } = layout([month("2026-01", "1000.00", "500.00"), month("2026-02", "0.00", "250.00", false)]);
    expect(zero).toBe(HEIGHT - 22);
    expect(columns.map((c) => c.center)).toEqual([WIDTH / 4, (3 * WIDTH) / 4]);
    expect(columns[0]!.incomePath).toMatch(/Q[\d.]+,8 /); // the largest value reaches the top padding
    expect(columns[1]!.incomePath).toBe("");
    expect(columns[1]!.inPeriod).toBe(false);
  });

  it("keeps zero inside the scale for negative months", () => {
    const { columns, zero } = layout([month("2026-01", "-100.00", "100.00")]);
    expect(zero).toBeCloseTo((8 + HEIGHT - 22) / 2);
    expect(columns[0]!.incomePath.startsWith(`M`)).toBe(true);
    expect(layout([]).columns).toEqual([]);
  });

  it("rounds only the data end", () => {
    expect(barPath(0, 10, 100, 50)).toBe("M0,100V54Q0,50 4,50H6Q10,50 10,54V100Z");
    expect(barPath(0, 10, 100, 150)).toBe("M0,100V146Q0,150 4,150H6Q10,150 10,146V100Z");
    expect(barPath(0, 10, 100, 100)).toBe("");
  });
});
