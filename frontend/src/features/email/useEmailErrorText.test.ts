import { describe, expect, it } from "vitest";
import { waitParams } from "./useEmailErrorText";

describe("waitParams", () => {
  it.each([
    [1, "errors.waitMinutes", 1],
    [60, "errors.waitMinutes", 1],
    [61, "errors.waitMinutes", 2],
    [3599, "errors.waitMinutes", 60],
    [3600, "errors.waitHours", 1],
    [79_200, "errors.waitHours", 22],
  ])("%i s → %s %i", (secs, key, n) => {
    expect(waitParams(secs)).toEqual({ key, n });
  });
});
