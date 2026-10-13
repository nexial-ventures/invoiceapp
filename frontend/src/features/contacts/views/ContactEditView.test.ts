import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
import { contact } from "@/features/documents/testData";
import type { Role } from "@/features/spaces/types";
import { i18n } from "@/i18n";
import { calls, mockFetchRoutes, reply, signIn } from "@/test-utils";
import ContactEditView from "./ContactEditView.vue";

async function mountContact(role: Role) {
  const pinia = createPinia();
  setActivePinia(pinia);
  signIn(role);
  const stub = { template: "<div />" };
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/contacts", name: "contacts", component: stub },
      { path: "/contacts/new", name: "contact-new", component: stub },
      { path: "/contacts/:id", name: "contact-edit", component: ContactEditView },
    ],
  });
  await router.push("/contacts/c1");
  const w = mount({ template: "<RouterView />" }, { global: { plugins: [pinia, i18n, router] } });
  await flushPromises();
  return w;
}

describe("ContactEditView roles", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("is read-only for an accountant: disabled fields, no Save, no Delete", async () => {
    const fetch = mockFetchRoutes({ "GET /api/contacts/c1": contact(), "GET /api/contacts/c1/number-counters": [] });
    const w = await mountContact("accountant");
    expect(w.find('[data-test="contact-fields"]').attributes("disabled")).toBeDefined();
    expect(w.find('[data-test="contact-save"]').exists()).toBe(false);
    expect(w.find(".btn-danger").exists()).toBe(false);
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(calls(fetch)).toEqual(["GET /api/contacts/c1", "GET /api/contacts/c1/number-counters"]);
  });

  it("is editable for a member", async () => {
    mockFetchRoutes({ "GET /api/contacts/c1": contact(), "GET /api/contacts/c1/number-counters": [] });
    const w = await mountContact("member");
    expect(w.find('[data-test="contact-fields"]').attributes("disabled")).toBeUndefined();
    expect(w.find('[data-test="contact-save"]').exists()).toBe(true);
    expect(w.find(".btn-danger").exists()).toBe(true);
  });
});

const clientSeries = (lastNumber: number, preview: string | null = "ACME-2026008") => ({
  docType: "invoice" as const,
  clientPattern: "{C}-{YYYY}{NNN}",
  counters: [{ year: 2026, lastNumber }],
  nextNumberPreview: preview,
});

describe("ContactEditView numbering", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("sends the number code with every save (PUT replaces all fields)", async () => {
    let sent: Record<string, unknown> | undefined;
    mockFetchRoutes({
      "GET /api/contacts/c1": contact({ numberCode: "ACME" }),
      "GET /api/contacts/c1/number-counters": [],
      "PUT /api/contacts/c1": (body: unknown) => {
        sent = body as Record<string, unknown>;
        return contact({ numberCode: "ACME" });
      },
    });
    const w = await mountContact("member");
    expect((w.find("#contact-numberCode").element as HTMLInputElement).value).toBe("ACME");
    await w.find("#contact-note").setValue("Changed");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(sent).toMatchObject({ note: "Changed", numberCode: "ACME" });
  });

  it("maps a duplicate number code to the field", async () => {
    mockFetchRoutes({
      "GET /api/contacts/c1": contact(),
      "GET /api/contacts/c1/number-counters": [],
      "PUT /api/contacts/c1": reply(422, { code: "validation", fields: { numberCode: "duplicate" } }),
    });
    const w = await mountContact("member");
    await w.find("#contact-numberCode").setValue("beta");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(w.find("#contact-numberCode-error").text()).toBe("This value already exists.");
  });

  it("lists the client sequences with the next number; an admin edits a counter", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/contacts/c1": contact({ numberCode: "ACME" }),
      "GET /api/contacts/c1/number-counters": [clientSeries(7)],
      "PUT /api/contacts/c1/number-counters/invoice/2026": clientSeries(3, "ACME-2026008"),
    });
    const w = await mountContact("admin");
    const card = w.find('[data-test="contact-series-invoice"]');
    expect(card.find("h3").text()).toBe("Invoices");
    expect(card.find('[data-test="contact-next-number"]').text()).toBe("ACME-2026008");
    await card.find('input[aria-label="Last number for 2026"]').setValue("3");
    await card.find("tbody tr button").trigger("click");
    await flushPromises();
    const put = fetch.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(put?.[0]).toBe("/api/contacts/c1/number-counters/invoice/2026");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({ lastNumber: 3 });
    expect(w.find('[data-test="contact-series-invoice"] input').exists()).toBe(true);
  });

  it("is read-only below admin and explains a missing code or client pattern", async () => {
    mockFetchRoutes({ "GET /api/contacts/c1": contact(), "GET /api/contacts/c1/number-counters": [clientSeries(7, null)] });
    const w = await mountContact("member");
    expect(w.find('[data-test="contact-series-invoice"] input').exists()).toBe(false);
    expect(w.find('[data-test="contact-next-number"]').text()).toBe("—");
    expect(w.find('[data-test="no-code"]').exists()).toBe(true);

    mockFetchRoutes({ "GET /api/contacts/c1": contact(), "GET /api/contacts/c1/number-counters": [] });
    const w2 = await mountContact("member");
    expect(w2.find('[data-test="no-client-series"]').exists()).toBe(true);
  });
});
