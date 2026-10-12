import { describe, expect, it } from "vitest";
import { sameAmount } from "@/lib/money";
import { derivedRoundingMode, enforceRecapVatMode, newRecapRow, recapTotal, suggestVat, updateRow, validateRecap } from "./recap";

describe("recap amounts", () => {
  it("suggests VAT as round2(base × rate / 100), half away from zero", () => {
    expect(suggestVat("1000", "21")).toBe("210.00");
    expect(suggestVat("0.05", "10")).toBe("0.01");
    expect(suggestVat("-0.05", "10")).toBe("-0.01");
    expect(suggestVat("99.99", "12")).toBe("12.00");
    expect(suggestVat("100", "12.5")).toBe("12.50");
    expect(suggestVat("", "21")).toBe("");
    expect(suggestVat("100", "101")).toBe("");
  });

  it("sums base + vat + rounding, null while incomplete", () => {
    const rows = [
      { base: "1000", vat: "210" },
      { base: "100,5", vat: "" },
    ];
    expect(recapTotal(rows, "-0.5")).toBe("1310.00");
    expect(recapTotal([{ base: "x", vat: "0" }], "0")).toBeNull();
    expect(sameAmount("210", "210.00")).toBe(true);
    expect(sameAmount(null, "0")).toBe(false);
  });
});

describe("recap rows", () => {
  it("auto-suggests VAT until the user types one; clearing it resumes the suggestion", () => {
    let row = updateRow(newRecapRow("21"), { base: "100" }, "standard");
    expect(row).toMatchObject({ vat: "21.00", vatAuto: true });
    row = updateRow(row, { vat: "20.99" }, "standard");
    expect(row).toMatchObject({ vat: "20.99", vatAuto: false });
    row = updateRow(row, { base: "200" }, "standard");
    expect(row.vat).toBe("20.99");
    row = updateRow(row, { vat: "" }, "standard");
    expect(row).toMatchObject({ vat: "42.00", vatAuto: true });
  });

  it("forces VAT 0 outside the standard mode and restores suggestions back in it", () => {
    const rows = [updateRow(newRecapRow("21"), { base: "100" }, "standard"), { ...newRecapRow("12"), base: "50", vat: "7", vatAuto: false }];
    const exempt = enforceRecapVatMode(rows, "exempt");
    expect(exempt.map((r) => r.vat)).toEqual(["0", "0"]);
    expect(updateRow(exempt[0]!, { vat: "5" }, "exempt").vat).toBe("0");
    expect(enforceRecapVatMode(exempt, "standard").map((r) => r.vat)).toEqual(["21.00", "6.00"]);
  });

  it("validates rows with the server's 422 keys", () => {
    const rows = [
      { ...newRecapRow("21"), base: "100", vat: "21" },
      { ...newRecapRow("21.00"), base: "", vat: "x" },
      { ...newRecapRow("150"), base: "1", vat: "0" },
    ];
    expect(validateRecap(rows, "standard")).toEqual({
      "vatRecap.1.rate": "duplicate",
      "vatRecap.1.base": "required",
      "vatRecap.1.vat": "invalid",
      "vatRecap.2.rate": "invalid",
    });
    expect(validateRecap([{ ...newRecapRow("0"), base: "1", vat: "1" }], "reverse_charge")).toEqual({ "vatRecap.0.vat": "invalid" });
    expect(validateRecap([], "standard")).toEqual({ vatRecap: "required" });
  });
});

describe("derived rounding mode", () => {
  const rows = (base: string, vat = "0") => [{ base, vat }];
  it("none without a rounding amount, unit / half on whole / 0.50 totals, none for an odd amount", () => {
    expect(derivedRoundingMode(rows("100.30"), "")).toBe("none");
    expect(derivedRoundingMode(rows("100.30"), "-0.30")).toBe("unit");
    expect(derivedRoundingMode(rows("100.30"), "0.20")).toBe("half");
    expect(derivedRoundingMode(rows("100.30"), "0.05")).toBe("none");
    // Negative totals (credit notes entered with signs) follow the same rule.
    expect(derivedRoundingMode(rows("-100.30"), "-0.20")).toBe("half");
    expect(derivedRoundingMode(rows("100", "21.40"), "0.60")).toBe("unit");
  });

  it("null while an amount is invalid", () => {
    expect(derivedRoundingMode(rows("x"), "0.20")).toBeNull();
    expect(derivedRoundingMode(rows("100"), "abc")).toBeNull();
  });
});
