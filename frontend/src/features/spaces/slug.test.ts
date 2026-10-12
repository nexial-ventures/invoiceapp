import { describe, expect, it } from "vitest";
import { baseHost, slugError } from "./slug";

describe("slugError", () => {
  it.each([
    ["firma", null],
    ["abc", null],
    ["a-1-b", null],
    ["x".repeat(30), null],
    ["", "required"],
    ["ab", "invalid"],
    ["x".repeat(31), "invalid"],
    ["-firma", "invalid"],
    ["firma-", "invalid"],
    ["Firma", "invalid"],
    ["fir ma", "invalid"],
    ["firmá", "invalid"],
    ["www", "reserved"],
    ["support", "reserved"],
    ["paypal", "reserved"],
    ["faktura", "reserved"],
  ])("%s → %s", (slug, reason) => {
    expect(slugError(slug)).toBe(reason);
  });
});

describe("baseHost", () => {
  it("keeps the port and drops scheme and path", () => {
    expect(baseHost("https://invoiceapp.cz")).toBe("invoiceapp.cz");
    expect(baseHost("http://localhost:3000/")).toBe("localhost:3000");
  });
});
