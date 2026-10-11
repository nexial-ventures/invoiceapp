import { describe, expect, it } from "vitest";
import { toCompany, toCompanyDraft, validateCompany } from "./companyForm";

describe("company form", () => {
  it("defaults an empty seeded row to CZ / 14 days / cs", () => {
    const d = toCompanyDraft(null);
    expect([d.country, d.defaultDueDays, d.defaultLocale]).toEqual(["CZ", "14", "cs"]);
  });

  it("validates name, IČO and due days range", () => {
    const errors = validateCompany({ ...toCompanyDraft(null), ico: "25596642", defaultDueDays: "366" });
    expect(errors).toEqual({ name: "required", ico: "invalid_ico", defaultDueDays: "invalid" });
  });

  it("round-trips to the wire shape with nulls for blanks", () => {
    const company = toCompany({ ...toCompanyDraft(null), name: "Me", ico: "25596641", vatPayer: true });
    expect(company).toMatchObject({ name: "Me", ico: "25596641", dic: null, vatPayer: true, defaultDueDays: 14, web: null });
  });

  it("document defaults: server fallbacks for a seeded row, notes per locale as \"\"", () => {
    expect(toCompanyDraft(null)).toMatchObject({
      defaultPaymentMethod: "bank_transfer",
      defaultCurrency: "CZK",
      defaultRoundingCash: "unit",
      defaultRoundingOther: "none",
      defaultHeaderNote: { cs: "", en: "" },
      defaultFooterNote: { cs: "", en: "" },
    });
  });

  it("validates the default currency and each note with nested keys", () => {
    const d = toCompanyDraft(null);
    const long = "x".repeat(2001);
    const errors = validateCompany({
      ...d,
      name: "Me",
      defaultCurrency: "EU",
      defaultHeaderNote: { cs: "ok", en: long },
      defaultFooterNote: { cs: long, en: "" },
    });
    expect(errors).toEqual({ defaultCurrency: "invalid", "defaultHeaderNote.en": "too_long", "defaultFooterNote.cs": "too_long" });
  });

  it("sends the defaults with blank notes as null and the currency uppercased", () => {
    const company = toCompany({
      ...toCompanyDraft(null),
      defaultCurrency: " eur ",
      defaultPaymentMethod: "cash",
      defaultRoundingCash: "half",
      defaultHeaderNote: { cs: " Děkujeme ", en: " " },
    });
    expect(company).toMatchObject({
      defaultCurrency: "EUR",
      defaultPaymentMethod: "cash",
      defaultRoundingCash: "half",
      defaultRoundingOther: "none",
      defaultHeaderNote: { cs: "Děkujeme", en: null },
      defaultFooterNote: { cs: null, en: null },
    });
  });
});
