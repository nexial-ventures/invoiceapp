import { describe, expect, it } from "vitest";
import {
  addDays,
  applyContact,
  changeCurrency,
  defaultVatRate,
  newDocumentDraft,
  toComputeRequest,
  toDraft,
  toInput,
  validateDocument,
} from "./form";
import { formatMoney, remainingAmount } from "./format";
import { newItemLine, newTextLine, type LineDraft } from "./lines";
import { bankAccounts, company, contact, ctx, document, vatRates } from "./testData";

describe("new document defaults", () => {
  it("mirrors the server create defaults", () => {
    expect(newDocumentDraft(ctx())).toMatchObject({
      contactId: null,
      issueDate: "2026-10-08",
      taxPointDate: "2026-10-08",
      dueDate: "2026-10-22",
      currency: "CZK",
      locale: "cs",
      vatMode: "standard",
      bankAccountId: "czk-b",
      paymentMethod: "bank_transfer",
      roundingMode: "none",
      lines: [],
    });
  });

  it("non-payer company → non_payer mode and rate 0 for new items", () => {
    const draft = newDocumentDraft(ctx({ company: { ...company, vatPayer: false } }));
    expect(draft.vatMode).toBe("non_payer");
    expect(defaultVatRate(vatRates, "non_payer")).toBe("0");
  });

  it("new item rate = default active rate", () => {
    expect(defaultVatRate(vatRates, "standard")).toBe("21");
    const noDefault = vatRates.map((r) => ({ ...r, isDefault: false }));
    expect(defaultVatRate(noDefault, "standard")).toBe("21");
  });

  it("addDays crosses month ends", () => {
    expect(addDays("2026-10-25", 14)).toBe("2026-11-08");
  });
});

describe("applyContact / changeCurrency", () => {
  it("applies contact due days, locale and currency with its default account", () => {
    const draft = applyContact(
      newDocumentDraft(ctx()),
      contact({ defaultDueDays: 30, defaultLocale: "en", defaultCurrency: "EUR" }),
      ctx(),
    );
    expect(draft).toMatchObject({ contactId: "c1", dueDate: "2026-11-07", locale: "en", currency: "EUR", bankAccountId: "eur" });
  });

  it("falls back to company defaults and keeps the currency", () => {
    const start = { ...newDocumentDraft(ctx()), locale: "en" as const };
    expect(applyContact(start, contact(), ctx())).toMatchObject({ dueDate: "2026-10-22", locale: "cs", currency: "CZK", bankAccountId: "czk-b" });
  });

  it("switching currency resets mismatched account and rate; rounding is allowed in any currency", () => {
    const czk = { ...newDocumentDraft(ctx()), roundingMode: "half" as const, bankAccountId: "czk-a" };
    const eur = changeCurrency(czk, "eur", bankAccounts);
    expect(eur).toMatchObject({ currency: "EUR", bankAccountId: "eur", roundingMode: "half" });
    const back = changeCurrency({ ...eur, exchangeRate: "24.5" }, "CZK", bankAccounts);
    expect(back).toMatchObject({ currency: "CZK", bankAccountId: "czk-b", exchangeRate: "" });
    expect(changeCurrency(czk, "USD", bankAccounts).bankAccountId).toBe("");
  });

  it("any currency switch clears the manual rate; re-entering the same code keeps it", () => {
    const eur = { ...changeCurrency(newDocumentDraft(ctx()), "EUR", bankAccounts), exchangeRate: "24.5" };
    expect(changeCurrency(eur, "USD", bankAccounts).exchangeRate).toBe("");
    expect(changeCurrency(eur, " eur ", bankAccounts).exchangeRate).toBe("24.5");
  });
});

describe("draft ↔ wire", () => {
  it("round-trips a document; empty strings become null", () => {
    const draft = toDraft(document());
    const input = toInput({ ...draft, variableSymbol: " ", orderRef: " PO-1 ", exchangeRate: "" });
    expect(input).toMatchObject({
      docType: "invoice",
      direction: "issued",
      contactId: "c1",
      taxPointDate: "2026-10-01",
      bankAccountId: "czk-b",
      variableSymbol: null,
      orderRef: "PO-1",
      exchangeRate: null,
      lines: [{ kind: "item", description: "Work", quantity: "1", unitPrice: "1000", vatRate: "21" }],
    });
    expect(input.lines[0]).not.toHaveProperty("position");
  });

  it("compute body sends lines as entered, empty descriptions included", () => {
    const draft = { ...newDocumentDraft(ctx()), lines: [newItemLine("21"), { ...newTextLine(), description: "x" }] };
    expect(toComputeRequest(draft)).toEqual({
      lines: [
        { kind: "item", description: "", quantity: "1", unit: null, unitPrice: "0", discountPct: "0", vatRate: "21" },
        { kind: "text", description: "x" },
      ],
      vatMode: "standard",
      currency: "CZK",
      exchangeRate: null,
      roundingMode: "none",
      contactId: null,
      docType: "invoice",
      locale: "cs",
    });
  });

  it("compute body carries the advance-line context: contact, type, locale and the edited draft's id", () => {
    const draft = toDraft(document({ docType: "proforma", taxPointDate: null, locale: "en" }));
    expect(toComputeRequest(draft, null, "d1")).toMatchObject({ contactId: "c1", docType: "proforma", locale: "en", documentId: "d1" });
    expect(toComputeRequest(draft)).not.toHaveProperty("documentId");
    // PUT never converts a draft to another type.
    expect(toInput(draft).docType).toBe("proforma");
  });

  it("indicative ČNB rate goes to compute only, and only without a manual rate", () => {
    const eur = changeCurrency(newDocumentDraft(ctx()), "EUR", bankAccounts);
    expect(toComputeRequest(eur, "24.4").exchangeRate).toBe("24.4");
    expect(toInput(eur).exchangeRate).toBeNull();
    const manual = { ...eur, exchangeRate: "25,1" };
    expect(toComputeRequest(manual, "24.4").exchangeRate).toBe("25.1");
    expect(toInput(manual).exchangeRate).toBe("25.1");
  });

  it("validates header fields and line descriptions by 0-based index", () => {
    const draft = {
      ...newDocumentDraft(ctx()),
      dueDate: "2026-10-01",
      variableSymbol: "12a",
      lines: [{ ...newItemLine("21"), description: "ok" }, newItemLine("21")],
    };
    expect(validateDocument(draft)).toEqual({ dueDate: "invalid", variableSymbol: "invalid", "lines.1.description": "required" });
  });
});

describe("format", () => {
  it("formats money per locale and survives unknown currencies", () => {
    expect(formatMoney("1234.5", "CZK", "cs").replace(/\s/g, " ")).toBe("1 234,50 Kč");
    expect(formatMoney("1234.5", "EUR", "en")).toBe("€1,234.50");
    expect(formatMoney("1", "XX1", "en")).toBe("1.00 XX1");
  });

  it("remaining amount is exact and never negative", () => {
    expect(remainingAmount("1210.10", "210.20")).toBe("999.90");
    expect(remainingAmount("100.00", "150.00")).toBe("0.00");
  });
});

describe("doc types", () => {
  const advance = {
    kind: "advance" as const,
    position: 2,
    advanceDocumentId: "ddpp1",
    description: "Odpočet zálohy DP20260003",
    base: "-826.45",
    recap: [{ vatRate: "21", base: "-826.45", vat: "-173.55" }],
  };

  it("a new proforma has no tax point date and never sends one", () => {
    const draft = newDocumentDraft(ctx(), "proforma");
    expect(draft).toMatchObject({ docType: "proforma", taxPointDate: "" });
    expect(toInput({ ...draft, taxPointDate: "2026-10-08" })).toMatchObject({ docType: "proforma", taxPointDate: null, correctionReason: null });
  });

  it("debit notes and DDPP corrections require a reason too, imported corrections do not", () => {
    for (const docType of ["debit_note", "advance_credit_note"] as const) {
      const draft = { ...toDraft(document({ docType, correctionReason: null })), lines: [] };
      expect(validateDocument(draft)).toEqual({ correctionReason: "required" });
      expect(toInput({ ...draft, correctionReason: "x" })).toMatchObject({ docType, correctionReason: "x" });
      expect(validateDocument({ ...draft, imported: true, number: "EXT-1" })).toEqual({});
    }
  });

  it("a credit note requires its correction reason", () => {
    const draft = { ...toDraft(document({ docType: "credit_note", sign: -1, correctionReason: null })), lines: [] };
    expect(validateDocument(draft)).toEqual({ correctionReason: "required" });
    expect(toInput({ ...draft, correctionReason: " wrong price " })).toMatchObject({ docType: "credit_note", correctionReason: "wrong price" });
    // Other doc types never send one.
    expect(toInput({ ...draft, docType: "invoice", correctionReason: "x" }).correctionReason).toBeNull();
  });

  it("advance lines keep their display fields in the draft and go out as a bare reference", () => {
    const draft = toDraft(document({ lines: [...document().lines, advance] }));
    const line = draft.lines[1] as LineDraft;
    expect(line).toMatchObject({ kind: "advance", advanceDocumentId: "ddpp1", base: "-826.45", description: "Odpočet zálohy DP20260003" });
    // Server-generated description: no client "required" check.
    expect(validateDocument(draft)).toEqual({});
    expect(toComputeRequest(draft).lines).toEqual([
      { kind: "item", description: "Work", quantity: "1", unit: null, unitPrice: "1000", discountPct: "0", vatRate: "21" },
      { kind: "advance", advanceDocumentId: "ddpp1" },
    ]);
  });
});

describe("import mode", () => {
  it("any doc type, own number required, link only sent for imports", () => {
    const draft = newDocumentDraft(ctx(), "advance_tax_doc", true);
    expect(draft).toMatchObject({ docType: "advance_tax_doc", imported: true, number: "", taxPointDate: "2026-10-08" });
    expect(validateDocument(draft)).toEqual({ number: "required" });
    const input = toInput({ ...draft, number: " Z-1 ", relatedDocumentId: "pf1" });
    expect(input).toMatchObject({ imported: true, number: "Z-1", relatedDocumentId: "pf1" });
    const native = toInput({ ...newDocumentDraft(ctx()), relatedDocumentId: "pf1" });
    expect(native).toMatchObject({ imported: false, number: null });
    expect("relatedDocumentId" in native).toBe(false);
  });
});

