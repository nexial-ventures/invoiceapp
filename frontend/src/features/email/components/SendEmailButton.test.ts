import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { useToastStore } from "@/stores/toast";
import { calls, mockFetchRoutes, reply } from "@/test-utils";
import { company, contact, document } from "@/features/documents/testData";
import { mountDetail } from "@/features/documents/views/testMount";
import { emailSettings, logEntry, prefill } from "../testData";

const issued = document({ status: "issued", number: "20260001", paymentState: "unpaid", customer: { ...contact(), registration: null, vatPayer: null, web: null } });

describe("e-mail on the document detail", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("is not offered on a draft", async () => {
    const fetch = mockFetchRoutes({ "GET /api/documents/d1": document(), "GET /api/settings/company": company, "GET /api/contacts/c1": contact() });
    const { w } = await mountDetail();
    expect(w.find('[data-test="send-email"]').exists()).toBe(false);
    expect(w.find('[data-test="email-history"]').exists()).toBe(false);
    expect(calls(fetch).some((c) => c.includes("email"))).toBe(false);
  });

  it("is disabled with a hint when SMTP is not configured", async () => {
    mockFetchRoutes({
      "GET /api/documents/d1": { ...issued, status: "cancelled" },
      "GET /api/documents/d1/payments": [],
      "GET /api/documents/d1/emails": [],
      "GET /api/settings/email": emailSettings({ configured: false }),
    });
    const { w } = await mountDetail();
    expect(w.find('[data-test="send-email"]').attributes("disabled")).toBeDefined();
    expect(w.find('[data-test="send-email-hint"]').text()).toBe("E-mail sending is not configured on the server.");
  });

  it("is disabled with a hint while e-mail is disabled", async () => {
    mockFetchRoutes({
      "GET /api/documents/d1": issued,
      "GET /api/documents/d1/payments": [],
      "GET /api/documents/d1/emails": [],
      "GET /api/settings/email": emailSettings({ disabled: true }),
    });
    const { w } = await mountDetail();
    expect(w.find('[data-test="send-email"]').attributes("disabled")).toBeDefined();
    expect(w.find('[data-test="send-email-hint"]').text()).toBe("Sending e-mails is disabled on this instance.");
  });

  it("sends, then reloads the document (sentAt) and the history", async () => {
    let sentAt: string | null = null;
    const history = [logEntry({ id: "old", ok: false, error: "timeout" })];
    const fetch = mockFetchRoutes({
      "GET /api/documents/d1": () => ({ ...issued, sentAt }),
      "GET /api/documents/d1/payments": [],
      "GET /api/documents/d1/emails": () => [...history],
      "GET /api/settings/email": emailSettings(),
      "GET /api/documents/d1/email": prefill(),
      "POST /api/documents/d1/email": () => {
        sentAt = "2026-10-09T08:00:00Z";
        history.unshift(logEntry({ id: "new" }));
        return history[0];
      },
    });
    const { w } = await mountDetail();
    expect(w.findAll('[data-test="email-entry"]')).toHaveLength(1);
    expect(w.find('[data-test="mark-sent"]').text()).toBe("Mark as sent");

    await w.find('[data-test="send-email"]').trigger("click");
    await flushPromises();
    await w.find('[data-test="email-send"]').trigger("click");
    await flushPromises();

    expect(w.find('[data-test="send-email-dialog"]').exists()).toBe(false);
    expect(useToastStore().message).toBe("E-mail sent to buyer@acme.cz.");
    expect(w.find('[data-test="mark-sent"]').text()).toBe("Mark as sent again");
    expect(w.findAll('[data-test="email-entry"]')).toHaveLength(2);
    expect(calls(fetch).filter((c) => c === "GET /api/documents/d1/emails")).toHaveLength(2);
  });

  it("reloads the history after an SMTP failure (logged by the server) and on close", async () => {
    const history = [logEntry({ id: "old" })];
    const fetch = mockFetchRoutes({
      "GET /api/documents/d1": issued,
      "GET /api/documents/d1/payments": [],
      "GET /api/documents/d1/emails": () => [...history],
      "GET /api/settings/email": emailSettings(),
      "GET /api/documents/d1/email": prefill(),
      "POST /api/documents/d1/email": () => {
        history.unshift(logEntry({ id: "failed", ok: false, error: "550 rejected" }));
        return reply(502, { code: "smtp_failed", detail: "550 rejected" });
      },
    });
    const { w } = await mountDetail();
    await w.find('[data-test="send-email"]').trigger("click");
    await flushPromises();
    await w.find('[data-test="email-send"]').trigger("click");
    await flushPromises();

    expect(w.find('[data-test="send-email-dialog"]').exists()).toBe(true);
    expect(w.findAll('[data-test="email-failed"]')).toHaveLength(1);
    await w.find('[data-test="email-cancel"]').trigger("click");
    await flushPromises();
    expect(calls(fetch).filter((c) => c === "GET /api/documents/d1/emails")).toHaveLength(3);
  });

  it("does not reload the history after an error before SMTP", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/documents/d1": issued,
      "GET /api/documents/d1/payments": [],
      "GET /api/documents/d1/emails": [],
      "GET /api/settings/email": emailSettings(),
      "GET /api/documents/d1/email": prefill(),
      "POST /api/documents/d1/email": reply(503, { code: "storage_unavailable" }),
    });
    const { w } = await mountDetail();
    await w.find('[data-test="send-email"]').trigger("click");
    await flushPromises();
    await w.find('[data-test="email-send"]').trigger("click");
    await flushPromises();
    expect(calls(fetch).filter((c) => c === "GET /api/documents/d1/emails")).toHaveLength(1);
  });

  it("closes the dialog on Escape without sending", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/documents/d1": issued,
      "GET /api/documents/d1/payments": [],
      "GET /api/documents/d1/emails": [],
      "GET /api/settings/email": emailSettings(),
      "GET /api/documents/d1/email": prefill(),
    });
    const { w } = await mountDetail();
    await w.find('[data-test="send-email"]').trigger("click");
    await flushPromises();
    await w.find('[data-test="send-email-dialog"]').trigger("keydown", { key: "Escape" });
    expect(w.find('[data-test="send-email-dialog"]').exists()).toBe(false);
    expect(calls(fetch)).not.toContain("POST /api/documents/d1/email");
  });
});
