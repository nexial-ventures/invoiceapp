import { describe, expect, it } from "vitest";
import { toAccountingDraft, toAccountingSettings, validateAccounting } from "./accounting";
import type { AccountingLimits, AccountingSettings } from "./types";

/** What the server returns in `limits`. */
const LIMITS: AccountingLimits = { pohoda: { code: 19, numberSeries: 19 }, money: { code: 10, numberSeries: 5 } };

const settings: AccountingSettings = {
  pohoda: {
    ico: "12345678",
    codes: [{ direction: "received", docType: "credit_note", accounting: "3Pd", classificationVat: "PD", numberSeries: null, classificationVatNonDeductible: "PN" }],
  },
  money: {
    ico: "27074358",
    codes: [{ direction: "issued", docType: "simplified", accounting: "1Zj", classificationVat: "U", numberSeries: "ZD", classificationVatNonDeductible: null }],
  },
};

describe("accounting limits", () => {
  it("come from the server's limits, not a client copy", () => {
    const draft = toAccountingDraft(null);
    draft.pohoda.rows[0]!.accounting = "x".repeat(4);
    draft.pohoda.rows[0]!.numberSeries = "x".repeat(3);
    const tight: AccountingLimits = { pohoda: { code: 3, numberSeries: 3 }, money: LIMITS.money };
    expect(validateAccounting(draft, tight)).toEqual({ "pohoda.codes.0.accounting": "too_long" });
    expect(validateAccounting(draft, LIMITS)).toEqual({});
  });
});

describe("accounting draft", () => {
  it("lays out every direction × doc type in display order, missing rows empty", () => {
    const draft = toAccountingDraft(settings);
    const expected = [
      "issued/invoice",
      "issued/credit_note",
      "issued/debit_note",
      "issued/advance_tax_doc",
      "issued/advance_credit_note",
      "issued/simplified",
      "received/invoice",
      "received/credit_note",
      "received/debit_note",
      "received/advance_tax_doc",
      "received/advance_credit_note",
      "received/simplified",
    ];
    expect(draft.pohoda.rows.map((r) => `${r.direction}/${r.docType}`)).toEqual(expected);
    expect(draft.money.rows.map((r) => `${r.direction}/${r.docType}`)).toEqual(expected);
    expect(draft.pohoda.rows[7]).toEqual({
      direction: "received",
      docType: "credit_note",
      accounting: "3Pd",
      classificationVat: "PD",
      numberSeries: "",
      classificationVatNonDeductible: "PN",
    });
    expect(draft.money.rows[5]).toMatchObject({ accounting: "1Zj", classificationVat: "U", numberSeries: "ZD" });
    expect(draft.pohoda.rows[0]).toMatchObject({ accounting: "", classificationVat: "", numberSeries: "" });
    expect(draft.pohoda.ico).toBe("12345678");
    expect(draft.money.ico).toBe("27074358");
  });

  it("starts both sections empty without settings", () => {
    const draft = toAccountingDraft(null);
    expect(draft.money.ico).toBe("");
    expect(draft.money.rows).toHaveLength(12);
  });

  it("trims, sends empty as null and always sends both sections", () => {
    const draft = toAccountingDraft(settings);
    draft.pohoda.ico = "  ";
    draft.pohoda.rows[0]!.numberSeries = " FV ";
    // Not editable on issued rows; a stray value is never sent.
    draft.pohoda.rows[0]!.classificationVatNonDeductible = "X";
    draft.money.rows[0]!.classificationVatNonDeductible = "X";
    draft.money.rows[6]!.classificationVatNonDeductible = " PN ";
    draft.money.rows[5]!.numberSeries = "";
    const out = toAccountingSettings(draft);
    expect(Object.keys(out)).toEqual(["pohoda", "money"]);
    expect(out.pohoda.ico).toBeNull();
    expect(out.pohoda.codes).toHaveLength(12);
    expect(out.pohoda.codes[0]).toEqual({ direction: "issued", docType: "invoice", accounting: null, classificationVat: null, numberSeries: "FV", classificationVatNonDeductible: null });
    expect(out.pohoda.codes[7]!.classificationVatNonDeductible).toBe("PN");
    expect(out.money.ico).toBe("27074358");
    expect(out.money.codes).toHaveLength(12);
    expect(out.money.codes[0]!.classificationVatNonDeductible).toBeNull();
    expect(out.money.codes[5]).toEqual({ direction: "issued", docType: "simplified", accounting: "1Zj", classificationVat: "U", numberSeries: null, classificationVatNonDeductible: null });
    expect(out.money.codes[6]!.classificationVatNonDeductible).toBe("PN");
  });

  it("flags Pohoda codes over 19 characters under the server's keys", () => {
    const draft = toAccountingDraft(null);
    draft.pohoda.rows[3]!.classificationVat = "x".repeat(20);
    draft.pohoda.rows[4]!.accounting = ` ${"y".repeat(19)} `;
    expect(validateAccounting(draft, LIMITS)).toEqual({ "pohoda.codes.3.classificationVat": "too_long" });
  });

  it("flags Money codes over 10 characters and number series over 5", () => {
    const draft = toAccountingDraft(null);
    draft.money.rows[0]!.accounting = "x".repeat(10);
    draft.money.rows[1]!.classificationVat = "x".repeat(11);
    draft.money.rows[7]!.classificationVatNonDeductible = "x".repeat(11);
    draft.money.rows[2]!.numberSeries = "x".repeat(5);
    draft.money.rows[3]!.numberSeries = "x".repeat(6);
    expect(validateAccounting(draft, LIMITS)).toEqual({
      "money.codes.1.classificationVat": "too_long",
      "money.codes.7.classificationVatNonDeductible": "too_long",
      "money.codes.3.numberSeries": "too_long",
    });
  });
});
