import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { calls, mockFetchRoutes, reply } from "@/test-utils";
import { company, contact, document } from "../testData";
import { mountDetail } from "./testMount";

const issued = document({ status: "issued", number: "20260001", paymentState: "unpaid", customer: { ...contact(), registration: null, vatPayer: null, web: null } });
const unlocked = document({
  number: "20260001",
  paid: "500.00",
  unlockedAt: "2026-10-12T09:30:00Z",
  unlockedBy: { email: "admin@example.com", displayName: "Alice Admin" },
  unlockReason: "Wrong VAT rate",
});
const issuedRoutes = (doc = issued) => ({
  "GET /api/documents/d1": doc,
  "GET /api/documents/d1/payments": [],
  "GET /api/documents/d1/emails": [],
  "GET /api/settings/company": company,
});

describe("unlocking an issued document", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("is offered to admins only", async () => {
    mockFetchRoutes(issuedRoutes());
    const member = (await mountDetail("d1", "member")).w;
    expect(member.find('[data-test="unlock"]').exists()).toBe(false);

    mockFetchRoutes(issuedRoutes());
    const admin = (await mountDetail("d1", "admin")).w;
    expect(admin.find('[data-test="unlock"]').exists()).toBe(true);

    // A native DDPP belongs to its payment.
    mockFetchRoutes(issuedRoutes({ ...issued, docType: "advance_tax_doc", paymentState: null }));
    const ddpp = (await mountDetail("d1", "owner")).w;
    expect(ddpp.find('[data-test="unlock"]').exists()).toBe(false);
  });

  it("keeps a note of the unlock on the re-issued document", async () => {
    mockFetchRoutes(issuedRoutes({ ...issued, unlockedAt: unlocked.unlockedAt, unlockedBy: unlocked.unlockedBy, unlockReason: "Wrong VAT rate" }));
    const w = (await mountDetail("d1", "accountant")).w;
    expect(w.find('[data-test="unlocked-banner"]').exists()).toBe(false);
    const note = w.find('[data-test="unlocked-note"]').text();
    expect(note).toContain("Alice Admin");
    expect(note).toContain("Wrong VAT rate");
  });

  it("explains why an unlocked draft cannot be deleted or re-issued", async () => {
    vi.stubGlobal("confirm", () => true);
    mockFetchRoutes({
      ...issuedRoutes(unlocked),
      "GET /api/contacts/c1": contact(),
      "DELETE /api/documents/d1": reply(409, { code: "related_in_use" }),
      "POST /api/documents/d1/issue": reply(409, { code: "exceeds_original" }),
    });
    const w = (await mountDetail("d1", "owner")).w;
    await w.find('[data-test="delete"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="action-error"]').text()).toContain("other documents refer to it");
    await w.find('[data-test="issue"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="action-error"]').text()).toContain("credit notes would exceed");
  });

  it("warns about what stays, requires a reason and shows the draft with its banner", async () => {
    const sent = { ...issued, sentAt: "2026-10-02T08:00:00Z", paid: "500.00", relatedDocuments: [{ id: "cn1", docType: "credit_note" as const, number: "D20260001", status: "issued" as const, payable: "100.00", currency: "CZK" }] };
    const fetch = mockFetchRoutes({ ...issuedRoutes(sent), "GET /api/contacts/c1": contact(), "POST /api/documents/d1/unlock": unlocked });
    const w = (await mountDetail("d1", "owner")).w;

    await w.find('[data-test="unlock"]').trigger("click");
    for (const warn of ["sent", "payments", "related"]) {
      expect(w.find(`[data-test="unlock-warn-${warn}"]`).exists(), warn).toBe(true);
    }
    await w.find('[data-test="unlock-reason"]').setValue("   ");
    await w.find('[data-test="unlock-confirm"]').trigger("click");
    await flushPromises();
    expect(w.find('[data-test="unlock-reason-error"]').exists()).toBe(true);
    expect(calls(fetch)).not.toContain("POST /api/documents/d1/unlock");

    await w.find('[data-test="unlock-reason"]').setValue("  Wrong VAT rate ");
    await w.find('[data-test="unlock-confirm"]').trigger("click");
    await flushPromises();
    const post = fetch.mock.calls.find(([, init]) => init?.method === "POST");
    expect(JSON.parse(String(post?.[1]?.body))).toEqual({ reason: "Wrong VAT rate" });
    const banner = w.find('[data-test="unlocked-banner"]');
    expect(banner.text()).toContain("Alice Admin");
    expect(banner.text()).toContain("Wrong VAT rate");
    expect(w.find('[data-test="kept-number"]').text()).toContain("20260001");
    expect(w.find("h1").text()).toContain("20260001");
    expect(w.find('[data-test="issue"]').exists()).toBe(true);
    expect(w.find('[data-test="unlock"]').exists()).toBe(false);
  });

  it("hides the warnings that do not apply", async () => {
    mockFetchRoutes(issuedRoutes());
    const w = (await mountDetail("d1", "admin")).w;
    await w.find('[data-test="unlock"]').trigger("click");
    expect(w.find('[data-test="unlock-dialog"]').exists()).toBe(true);
    for (const warn of ["sent", "payments", "related"]) {
      expect(w.find(`[data-test="unlock-warn-${warn}"]`).exists(), warn).toBe(false);
    }
  });

  it("warns that deleting an unlocked draft deletes its payments", async () => {
    const asked: string[] = [];
    vi.stubGlobal("confirm", (q: string) => {
      asked.push(q);
      return false;
    });
    mockFetchRoutes({ ...issuedRoutes(unlocked), "GET /api/contacts/c1": contact() });
    const w = (await mountDetail("d1", "owner")).w;
    await w.find('[data-test="delete"]').trigger("click");
    expect(asked[0]).toContain("20260001");
    expect(asked[0]).toContain("payments");
  });

  it("shows when, who and why in Czech too, and a deleted user", async () => {
    i18n.global.locale.value = "cs";
    mockFetchRoutes({ ...issuedRoutes({ ...unlocked, unlockedBy: null }), "GET /api/contacts/c1": contact() });
    const w = (await mountDetail("d1", "owner")).w;
    const banner = w.find('[data-test="unlocked-banner"]').text();
    expect(banner).toContain("Odemčeno");
    expect(banner).toContain("smazaný uživatel");
  });
});
