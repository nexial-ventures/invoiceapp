import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { i18n } from "@/i18n";
import { calls, mockFetchRoutes } from "@/test-utils";
import { newDocumentDraft, type DocumentDraft } from "../form";
import { ctx } from "../testData";
import DocumentNumberField from "./DocumentNumberField.vue";

function mountField(draft: DocumentDraft) {
  setActivePinia(createPinia());
  const w = mount(DocumentNumberField, {
    props: {
      modelValue: draft,
      "onUpdate:modelValue": (v: DocumentDraft) => void w.setProps({ modelValue: v }),
    },
    global: { plugins: [i18n] },
  });
  return w;
}

const model = (w: ReturnType<typeof mountField>) => w.props("modelValue") as DocumentDraft;
const settle = async () => {
  await vi.advanceTimersByTimeAsync(300);
  await flushPromises();
};

describe("DocumentNumberField", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("previews the expected number and refreshes it when the customer or date changes", async () => {
    const fetch = mockFetchRoutes({
      "GET /api/documents/number-preview": (() => {
        let n = 0;
        return () => (n++ === 0 ? { number: "20260008", source: "global" } : { number: "ACME-2027001", source: "client" });
      })(),
    });
    const w = mountField(newDocumentDraft(ctx()));
    await settle();
    expect(w.find('[data-test="number-preview"]').text()).toBe("Will be issued as 20260008 (global series) — expected, final at issue.");
    expect(w.find("#doc-number").attributes("placeholder")).toBe("20260008");

    await w.setProps({ modelValue: { ...model(w), contactId: "c1", issueDate: "2027-01-05" } });
    await settle();
    expect(w.find('[data-test="number-preview"] strong').text()).toBe("ACME-2027001");
    expect(w.find('[data-test="number-preview"]').text()).toContain("(client series)");
    expect(calls(fetch)).toEqual([
      "GET /api/documents/number-preview?docType=invoice&issueDate=2026-10-08",
      "GET /api/documents/number-preview?docType=invoice&issueDate=2027-01-05&contactId=c1",
    ]);
  });

  it("a typed number is touched and hides the preview; back to the sequence clears it", async () => {
    mockFetchRoutes({ "GET /api/documents/number-preview": { number: "20260008", source: "global" } });
    const w = mountField({ ...newDocumentDraft(ctx()), number: "20260001" });
    await settle();
    expect(w.find('[data-test="number-preview"]').exists()).toBe(false);
    expect(w.find('[data-test="number-manual-hint"]').exists()).toBe(true);

    await w.find("#doc-number").setValue("20260005");
    expect(model(w)).toMatchObject({ number: "20260005", numberTouched: true });

    await w.find('[data-test="number-from-sequence"]').trigger("click");
    expect(model(w)).toMatchObject({ number: "", numberTouched: true });
    await settle();
    expect(w.find('[data-test="number-preview"] strong').text()).toBe("20260008");
  });

  it("shows a field error instead of the hints", async () => {
    mockFetchRoutes({});
    const w = mountField(newDocumentDraft(ctx()));
    await w.setProps({ error: "taken" });
    expect(w.find("#doc-number-error").text()).toBe("This value is already taken.");
    expect(w.find('[data-test="number-preview"]').exists()).toBe(false);
  });
});
