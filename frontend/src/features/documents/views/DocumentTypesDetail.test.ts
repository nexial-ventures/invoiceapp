import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { calls, mockFetchRoutes, reply } from "@/test-utils";
import { document } from "../testData";
import type { Document, Payment, RelatedDocument } from "../types";
import { mountDetail } from "./testMount";

const proforma = (overrides: Partial<Document> = {}) =>
  document({ id: "pf1", docType: "proforma", status: "issued", number: "ZF20260001", taxPointDate: null, paymentState: "unpaid", settled: false, ...overrides });
const ddppRef: RelatedDocument = { id: "dd1", docType: "advance_tax_doc", number: "DP20260001", status: "issued", payable: "500.00", currency: "CZK" };
const payment = (overrides: Partial<Payment> = {}): Payment => ({ id: "p1", date: "2026-10-08", amount: "500.00", note: null, advanceDocumentId: null, createdAt: "", ...overrides });

describe("detail per document type", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("proforma in EUR: payment with a manual rate, then links the created DDPP", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const payments: Payment[] = [];
    let related: RelatedDocument[] = [];
    const fetch = mockFetchRoutes({
      "GET /api/documents/pf1": () => proforma({ currency: "EUR", relatedDocuments: related }),
      "GET /api/documents/pf1/payments": () => payments,
      "GET /api/exchange-rates/EUR": { currency: "EUR", date: "2026-10-07", rate: "24.4" },
      "POST /api/documents/pf1/payments": (body: unknown) => {
        const p = payment({ ...(body as object), advanceDocumentId: "dd1" });
        payments.push(p);
        related = [ddppRef];
        return reply(201, p);
      },
    });
    const { w } = await mountDetail("pf1");
    expect(w.find("h1").text()).toContain("Proforma invoice");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(w.find('[data-test="payment-indicative-rate"]').text()).toBe("ČNB rate for 7 Oct 2026: 24.4");

    await w.find("#payment-amount").setValue("500");
    await w.find("#payment-rate").setValue("24,5");
    await w.find('[data-test="payments-panel"] form').trigger("submit");
    await flushPromises();

    const post = fetch.mock.calls.find(([, init]) => init?.method === "POST");
    expect(JSON.parse(String(post?.[1]?.body))).toMatchObject({ amount: "500", exchangeRate: "24.5" });
    const created = w.find('[data-test="ddpp-created"]');
    expect(created.text()).toContain("DP20260001");
    expect(created.find("a").attributes("href")).toBe("/invoices/dd1");
    expect(w.find('[data-test="payment-ddpp"]').text()).toBe("Advance payment tax document DP20260001");
    expect(w.find('[data-test="related-child"]').text()).toContain("DP20260001");
  });

  it("a CZK proforma payment sends no rate field", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/documents/pf1": proforma(),
      "GET /api/documents/pf1/payments": [],
      "POST /api/documents/pf1/payments": reply(201, payment()),
    });
    const { w } = await mountDetail("pf1");
    expect(w.find("#payment-rate").exists()).toBe(false);
    await w.find('[data-test="payments-panel"] form').trigger("submit");
    await flushPromises();
    const post = fetch.mock.calls.find(([, init]) => init?.method === "POST");
    expect(JSON.parse(String(post?.[1]?.body))).not.toHaveProperty("exchangeRate");
  });

  it("deleting a payment warns about its DDPP and explains advance_settled", async () => {
    const confirm = vi.fn(() => true);
    vi.stubGlobal("confirm", confirm);
    mockFetchRoutes({
      "GET /api/documents/pf1": proforma({ relatedDocuments: [ddppRef], paymentState: "partial", paid: "500.00" }),
      "GET /api/documents/pf1/payments": [payment({ advanceDocumentId: "dd1" })],
      "DELETE /api/documents/pf1/payments/p1": reply(409, { code: "advance_settled" }),
    });
    const { w } = await mountDetail("pf1");
    await w.find('[data-test="delete-payment"]').trigger("click");
    await flushPromises();
    expect(confirm).toHaveBeenCalledWith("Delete this payment? Its advance payment tax document DP20260001 will be cancelled too.");
    expect(w.find('[data-test="payment-error"]').text()).toBe(
      "The payment cannot be deleted: its advance payment tax document is already deducted by an issued final invoice.",
    );
  });

  it("advance_in_use links the draft final invoice that deducts the payment", async () => {
    vi.stubGlobal("confirm", () => true);
    const draftInvoice: RelatedDocument = { id: "inv9", docType: "invoice", number: null, status: "draft", payable: "0.00", currency: "CZK" };
    mockFetchRoutes({
      "GET /api/documents/pf1": proforma({ relatedDocuments: [ddppRef, draftInvoice], paymentState: "partial", paid: "500.00" }),
      "GET /api/documents/pf1/payments": [payment({ advanceDocumentId: "dd1" })],
      "DELETE /api/documents/pf1/payments/p1": reply(409, { code: "advance_in_use" }),
    });
    const { w } = await mountDetail("pf1");
    await w.find('[data-test="delete-payment"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="payment-error"]').text()).toContain("Remove the advance line from that draft");
    expect(w.find('[data-test="blocking-draft"]').attributes("href")).toBe("/invoices/inv9");
  });

  it("settling a proforma opens the new final-invoice draft", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/documents/pf1": proforma(),
      "GET /api/documents/pf1/payments": [],
      "POST /api/documents/pf1/settle": reply(201, document({ id: "inv2", relatedDocumentId: "pf1" })),
    });
    const { w, router } = await mountDetail("pf1");
    await w.find('[data-test="settle"]').trigger("click");
    await flushPromises();
    expect(calls(fetch)).toContain("POST /api/documents/pf1/settle");
    expect(router.currentRoute.value).toMatchObject({ name: "invoice-edit", params: { id: "inv2" } });
  });

  it("a settled proforma offers no settle action", async () => {
    mockFetchRoutes({ "GET /api/documents/pf1": proforma({ settled: true }), "GET /api/documents/pf1/payments": [] });
    const { w } = await mountDetail("pf1");
    expect(w.find('[data-test="settle"]').exists()).toBe(false);
    expect(w.find('[data-test="settled-badge"]').exists()).toBe(true);
  });

  it("issues a credit note from an invoice with the prompted reason", async () => {
    const prompt = vi.fn<(message?: string) => string | null>(() => null);
    vi.stubGlobal("prompt", prompt);
    const fetch = mockFetchRoutes({
      "GET /api/documents/d1": document({ status: "issued", number: "20260001", paymentState: "unpaid" }),
      "GET /api/documents/d1/payments": [],
      "POST /api/documents/d1/credit-note": reply(201, document({ id: "cn1", docType: "credit_note", sign: -1 })),
    });
    const { w, router } = await mountDetail();
    await w.find('[data-test="credit-note"]').trigger("click");
    expect(calls(fetch)).not.toContain("POST /api/documents/d1/credit-note");

    prompt.mockReturnValue(" wrong price ");
    await w.find('[data-test="credit-note"]').trigger("click");
    await flushPromises();
    const post = fetch.mock.calls.find(([, init]) => init?.method === "POST");
    expect(JSON.parse(String(post?.[1]?.body))).toEqual({ correctionReason: "wrong price" });
    expect(router.currentRoute.value).toMatchObject({ name: "invoice-edit", params: { id: "cn1" } });
  });

  it("credit note: negated totals, badge, parent invoice link and the exceeds_original reason", async () => {
    vi.stubGlobal("confirm", () => true);
    mockFetchRoutes({
      "GET /api/documents/cn1": document({
        id: "cn1",
        docType: "credit_note",
        sign: -1,
        relatedDocumentId: "d1",
        parent: { id: "d1", docType: "invoice", number: "20260001", status: "issued", payable: "1210.00", currency: "CZK" },
        correctionReason: "wrong price",
      }),
      "GET /api/settings/company": {},
      "GET /api/contacts/c1": {},
      "POST /api/documents/cn1/issue": reply(422, { code: "validation", fields: { lines: "exceeds_original" } }),
    });
    const { w } = await mountDetail("cn1");
    expect(w.find("h1").text()).toContain("Credit note");
    expect(w.find('[data-test="credit-note-badge"]').exists()).toBe(true);
    expect(w.find('[data-test="payable"]').text()).toMatch(/^-.*1,210\.00$/);
    expect(w.find('[data-test="info-correctionReason"]').text()).toBe("wrong price");
    const parent = w.find('[data-test="related-parent"]');
    expect(parent.text()).toContain("Invoice 20260001");
    expect(parent.find("a").attributes("href")).toBe("/invoices/d1");

    await w.find('[data-test="issue"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="action-error"] li').text()).toBe("Lines: Credit notes would exceed the original invoice for some VAT rate.");
  });

  it("DDPP is read-only except mark-sent and the metadata", async () => {
    const ddpp = document({
      id: "dd1",
      docType: "advance_tax_doc",
      status: "issued",
      number: "DP20260001",
      relatedDocumentId: "pf1",
      parent: { id: "pf1", docType: "proforma", number: "ZF20260001", status: "issued", payable: "1210.00", currency: "CZK" },
      paymentId: "p1",
    });
    const fetch = mockFetchRoutes({
      "GET /api/documents/dd1": ddpp,
      "POST /api/documents/dd1/mark-sent": { ...ddpp, sentAt: "2026-10-08T10:00:00Z" },
      "GET /api/settings/categories": [],
      "GET /api/settings/custom-fields": [],
    });
    const { w } = await mountDetail("dd1");
    expect(w.find("h1").text()).toContain("Advance payment tax document");
    for (const action of ["cancel", "credit-note", "settle", "edit", "issue", "delete"]) expect(w.find(`[data-test="${action}"]`).exists()).toBe(false);
    await w.find('[data-test="mark-sent"]').trigger("click");
    await flushPromises();
    expect(calls(fetch)).toContain("POST /api/documents/dd1/mark-sent");
    expect(w.find('[data-test="mark-sent"]').text()).toBe("Mark as sent again");
    expect(w.find('[data-test="payments-panel"]').exists()).toBe(false);
    expect(w.find("#meta-internalNote").exists()).toBe(true);
    expect(w.find('[data-test="related-parent"]').text()).toContain("Proforma invoice ZF20260001");
    // The parent comes with the document; only the e-mail history is fetched besides it.
    expect(calls(fetch).filter((c) => c.startsWith("GET /api/documents"))).toEqual(["GET /api/documents/dd1", "GET /api/documents/dd1/emails"]);
  });

  it("imported proforma: original PDF panel instead of ours, settle still offered, no DDPP rate, imported issue confirm", async () => {
    const draft = proforma({ status: "draft", number: "EXT-1", imported: true, paymentState: null, currency: "EUR", exchangeRate: "24.5" });
    const fetch = mockFetchRoutes({
      "GET /api/documents/pf1": draft,
      "GET /api/contacts/c1": { id: "c1", name: "Acme", street: "", city: "", zip: "", country: "CZ", ico: null, dic: null },
      "GET /api/settings/company": { name: "Me", street: "", city: "", zip: "", country: "CZ", ico: null, dic: null },
      "GET /api/settings/categories": [],
      "GET /api/settings/custom-fields": [],
      "POST /api/documents/pf1/issue": { ...draft, status: "issued", paymentState: "unpaid" },
      "GET /api/documents/pf1/payments": [],
    });
    const confirm = vi.fn(() => true);
    vi.stubGlobal("confirm", confirm);
    const { w } = await mountDetail("pf1");
    expect(w.find('[data-test="imported-badge"]').exists()).toBe(true);
    expect(w.find('[data-test="original-pdf"]').exists()).toBe(true);
    expect(w.find('[data-test="pdf-open"]').exists()).toBe(false);

    await w.find('[data-test="issue"]').trigger("click");
    await flushPromises();
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("keeps its number, no PDF is rendered"));
    expect(calls(fetch)).toContain("POST /api/documents/pf1/issue");
    expect(w.find('[data-test="settle"]').exists()).toBe(true);
    expect(w.find('[data-test="payments-panel"]').exists()).toBe(true);
    expect(w.find("#payment-rate").exists()).toBe(false);
  });
});

