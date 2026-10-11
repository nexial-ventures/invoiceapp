import { describe, expect, it } from "vitest";
import type { Company } from "@/features/settings/types";
import { changeLocale, changePaymentMethod, changeRounding, defaultNote, defaultRounding } from "./defaults";
import { applyContact, newDocumentDraft, toComputeRequest, toDraft, toInput } from "./form";
import { company, contact, ctx, document } from "./testData";

const withDefaults: Company = {
  ...company,
  defaultRoundingCash: "half",
  defaultRoundingOther: "unit",
  defaultPaymentMethod: "cash",
  defaultCurrency: "EUR",
  defaultHeaderNote: { cs: "Děkujeme", en: "Thank you" },
  defaultFooterNote: { cs: "Zapsáno v OR", en: null },
};
const custom = () => ctx({ company: withDefaults });

describe("company defaults", () => {
  it("rounding: cash has its own default, every other method shares one", () => {
    expect(defaultRounding(withDefaults, "cash")).toBe("half");
    for (const m of ["bank_transfer", "card", "other"] as const) expect(defaultRounding(withDefaults, m)).toBe("unit");
  });

  it("notes per locale, \"\" when unset", () => {
    expect(defaultNote(withDefaults, "header", "en")).toBe("Thank you");
    expect(defaultNote(withDefaults, "footer", "en")).toBe("");
  });

  it("a new draft starts from the company profile", () => {
    expect(newDocumentDraft(custom())).toMatchObject({
      paymentMethod: "cash",
      roundingMode: "half",
      currency: "EUR",
      bankAccountId: "eur",
      locale: "cs",
      headerNote: "Děkujeme",
      footerNote: "Zapsáno v OR",
      roundingAuto: true,
    });
    // Seeded profile: bank transfer, CZK, no rounding, no notes.
    expect(newDocumentDraft(ctx())).toMatchObject({ paymentMethod: "bank_transfer", roundingMode: "none", currency: "CZK", headerNote: "", footerNote: "" });
  });

  it("the mode goes to save and compute", () => {
    const draft = { ...newDocumentDraft(custom()), roundingMode: "unit" as const };
    expect(toInput(draft).roundingMode).toBe("unit");
    expect(toComputeRequest(draft).roundingMode).toBe("unit");
    expect(toInput(draft)).not.toHaveProperty("roundingAuto");
  });
});

describe("payment method / rounding", () => {
  it("changing the method re-applies that method's default", () => {
    const draft = changePaymentMethod(newDocumentDraft(custom()), "bank_transfer", withDefaults);
    expect(draft).toMatchObject({ paymentMethod: "bank_transfer", roundingMode: "unit" });
    expect(changePaymentMethod(draft, "cash", withDefaults).roundingMode).toBe("half");
  });

  it("a manual pick sticks across later method changes", () => {
    const manual = changeRounding(newDocumentDraft(custom()), "none");
    expect(manual).toMatchObject({ roundingMode: "none", roundingAuto: false });
    expect(changePaymentMethod(manual, "card", withDefaults).roundingMode).toBe("none");
    expect(changePaymentMethod(manual, "cash", withDefaults).roundingMode).toBe("none");
  });

  it("a saved draft keeps its mode (PUT applies no defaults)", () => {
    const draft = toDraft(document({ roundingMode: "half", paymentMethod: "cash" }));
    expect(draft).toMatchObject({ roundingMode: "half", roundingAuto: false });
    expect(changePaymentMethod(draft, "bank_transfer", withDefaults).roundingMode).toBe("half");
  });
});

describe("locale / notes", () => {
  it("swaps untouched notes for the new locale's defaults", () => {
    const en = changeLocale(newDocumentDraft(custom()), "en", withDefaults);
    expect(en).toMatchObject({ locale: "en", headerNote: "Thank you", footerNote: "" });
    expect(changeLocale(en, "cs", withDefaults)).toMatchObject({ headerNote: "Děkujeme", footerNote: "Zapsáno v OR" });
  });

  it("an edited note stays, the untouched one still swaps", () => {
    const edited = { ...newDocumentDraft(custom()), headerNote: "Vlastní text" };
    expect(changeLocale(edited, "en", withDefaults)).toMatchObject({ headerNote: "Vlastní text", footerNote: "" });
    const cleared = { ...newDocumentDraft(custom()), footerNote: "" };
    expect(changeLocale(cleared, "en", withDefaults).footerNote).toBe("");
  });

  it("a saved draft's own notes stay", () => {
    const draft = toDraft(document({ headerNote: "Own", footerNote: "Zapsáno v OR" }));
    // Its footer still equals the cs default, so it follows the locale like on a new draft.
    expect(changeLocale(draft, "en", withDefaults)).toMatchObject({ locale: "en", headerNote: "Own", footerNote: "" });
  });

  it("create sends a cleared default note as \"\" (no note), PUT as null", () => {
    const cleared = { ...newDocumentDraft(custom()), headerNote: " " };
    expect(toInput(cleared, [], true)).toMatchObject({ headerNote: "", footerNote: "Zapsáno v OR" });
    expect(toInput(cleared).headerNote).toBeNull();
  });

  it("picking a customer applies its locale with the notes and its currency over the company's", () => {
    const draft = applyContact(newDocumentDraft(custom()), contact({ defaultLocale: "en", defaultCurrency: "CZK" }), custom());
    expect(draft).toMatchObject({ locale: "en", headerNote: "Thank you", footerNote: "", currency: "CZK", bankAccountId: "czk-b" });
    // Without a contact currency the company default applies, not the previous contact's.
    expect(applyContact(newDocumentDraft(custom()), contact(), custom())).toMatchObject({ currency: "EUR", locale: "cs", headerNote: "Děkujeme" });
    const a = applyContact(newDocumentDraft(ctx()), contact({ defaultCurrency: "EUR" }), ctx());
    expect(a).toMatchObject({ currency: "EUR", bankAccountId: "eur" });
    expect(applyContact(a, contact({ id: "c2" }), ctx())).toMatchObject({ contactId: "c2", currency: "CZK", bankAccountId: "czk-b" });
  });
});
