import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { i18n } from "@/i18n";
import { mockFetch, sentRequest } from "@/test-utils";
import { useNumberSeriesStore } from "../stores";
import type { NumberSeries } from "../types";
import CountersEditor from "./CountersEditor.vue";
import NumberSeriesCard from "./NumberSeriesCard.vue";

const series = (counters: NumberSeries["counters"], pattern = "{YYYY}{NNNN}"): NumberSeries => ({
  docType: "invoice",
  pattern,
  clientPattern: null,
  counters,
  nextNumberPreview: "",
});

function plugins() {
  const pinia = createPinia();
  setActivePinia(pinia);
  return { global: { plugins: [pinia, i18n] } };
}

function mountEditor(s: NumberSeries, readonly = false) {
  const opts = plugins();
  const store = useNumberSeriesStore();
  const saveCounter = (year: number, lastNumber: number) => store.setCounter(s.docType, year, lastNumber);
  return mount(CountersEditor, { props: { counters: s.counters, currentYear: 2026, saveCounter, readonly }, ...opts });
}
const mountCard = (s: NumberSeries) => mount(NumberSeriesCard, { props: { series: s, currentYear: 2026 }, ...plugins() });

const input = (w: ReturnType<typeof mountEditor>, label: string) => w.find(`input[aria-label="${label}"]`);

describe("CountersEditor", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("saving one row keeps unsaved edits in other rows and the new-year row", async () => {
    const initial = series([
      { year: 2026, lastNumber: 41 },
      { year: 2025, lastNumber: 120 },
    ]);
    const w = mountEditor(initial);
    await input(w, "Last number for 2025").setValue("130");
    await input(w, "Year").setValue("2030");
    await input(w, "Last number for 2026").setValue("50");

    const saved = series([
      { year: 2026, lastNumber: 50 },
      { year: 2025, lastNumber: 120 },
    ]);
    const fetch = mockFetch(200, saved);
    const row2026 = w.findAll("tbody tr")[0]!;
    await row2026.find("button").trigger("click");
    await flushPromises();
    await w.setProps({ counters: saved.counters });

    expect(sentRequest(fetch)).toEqual({
      url: "/api/settings/number-series/invoice/counters/2026",
      method: "PUT",
      body: { lastNumber: 50 },
    });
    expect((input(w, "Last number for 2026").element as HTMLInputElement).value).toBe("50");
    expect((input(w, "Last number for 2025").element as HTMLInputElement).value).toBe("130");
    expect((input(w, "Year").element as HTMLInputElement).value).toBe("2030");
  });
});

describe("CountersEditor errors", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("lowers a counter (numbers already issued are skipped) and shows a field reason inline", async () => {
    const w = mountEditor(series([{ year: 2026, lastNumber: 41 }]));
    expect(w.find('[data-test="counter-hint"]').text()).toBe("A lower value is allowed: numbers already issued are skipped.");
    await input(w, "Last number for 2026").setValue("10");
    const fetch = mockFetch(422, { code: "validation", fields: { lastNumber: "invalid" } });
    await w.findAll("tbody tr")[0]!.find("button").trigger("click");
    await flushPromises();
    expect(sentRequest(fetch).body).toEqual({ lastNumber: 10 });
    expect(w.find('[data-test="counter-error"]').text()).toBe("Invalid value.");
    expect((input(w, "Last number for 2026").element as HTMLInputElement).value).toBe("10");
  });

  it("read-only lists the values without inputs", () => {
    const w = mountEditor(series([{ year: 2026, lastNumber: 41 }]), true);
    expect(w.find("input").exists()).toBe(false);
    expect(w.find('[data-test="counter-value"]').text()).toBe("41");
  });
});

describe("NumberSeriesCard", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("flags a pattern without a year token client-side", async () => {
    const w = mountCard(series([]));
    await w.find("#pattern-invoice").setValue("F{NNNN}");
    expect(w.find('[data-test="client-preview"]').text()).toBe("Invalid number pattern.");
  });

  it("renders a server duplicate-pattern 422 inline", async () => {
    const w = mountCard(series([]));
    await w.find("#pattern-invoice").setValue("D{YYYY}{NNNN}");
    mockFetch(422, { code: "validation", fields: { pattern: "duplicate" } });
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(w.find("#pattern-invoice-error").text()).toBe("This value already exists.");
  });
});

describe("ClientPatternForm", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("previews and saves a client pattern; empty clears it", async () => {
    const w = mountCard(series([]));
    await w.find("#client-pattern-invoice").setValue(" {C}-{YYYY}{NNN} ");
    expect(w.find('[data-test="client-pattern-value"]').text()).toBe("ACME-2026001");
    const fetch = mockFetch(200, { ...series([]), clientPattern: "{C}-{YYYY}{NNN}" });
    await w.find('[data-test="client-pattern-form"]').trigger("submit");
    await flushPromises();
    expect(sentRequest(fetch)).toEqual({
      url: "/api/settings/number-series/invoice/client-pattern",
      method: "PUT",
      body: { clientPattern: "{C}-{YYYY}{NNN}" },
    });

    await w.setProps({ series: { ...series([]), clientPattern: "{C}-{YYYY}{NNN}" } });
    await w.find("#client-pattern-invoice").setValue("");
    const clear = mockFetch(200, series([]));
    await w.find('[data-test="client-pattern-form"]').trigger("submit");
    await flushPromises();
    expect(sentRequest(clear).body).toEqual({ clientPattern: null });
  });

  it("flags a pattern without {C} client-side and shows a server reason inline", async () => {
    const w = mountCard(series([]));
    await w.find("#client-pattern-invoice").setValue("{YYYY}{NNN}");
    expect(w.find('[data-test="client-pattern-preview"]').text()).toContain("Invalid number pattern.");
    const fetch = mockFetch(422, { code: "validation", fields: { clientPattern: "not_issued" } });
    await w.find('[data-test="client-pattern-form"]').trigger("submit");
    expect(fetch).not.toHaveBeenCalled();
    expect(w.find("#client-pattern-invoice-error").text()).toBe("Invalid number pattern.");

    await w.find("#client-pattern-invoice").setValue("{C}{YYYY}{NNN}");
    await w.find('[data-test="client-pattern-form"]').trigger("submit");
    await flushPromises();
    expect(w.find("#client-pattern-invoice-error").text()).toBe("Only issued document series have this.");
  });

  it("is not offered on a received series", () => {
    const w = mountCard({ ...series([]), docType: "received" });
    expect(w.find('[data-test="client-pattern-form"]').exists()).toBe(false);
  });
});
