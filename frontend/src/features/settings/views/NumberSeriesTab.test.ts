import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { DOC_TYPES, ISSUED_SERIES, RECEIVED_SERIES } from "@/api/types";
import { i18n } from "@/i18n";
import { mockFetch } from "@/test-utils";
import type { NumberSeries } from "../types";
import NumberSeriesTab from "./NumberSeriesTab.vue";

const series = (docType: NumberSeries["docType"]): NumberSeries => ({ docType, pattern: `${docType}{YYYY}{NNNN}`, counters: [], nextNumberPreview: "" });

describe("NumberSeriesTab", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => vi.unstubAllGlobals());

  it("lists all fourteen series grouped issued / received, in display order", async () => {
    expect(DOC_TYPES).toHaveLength(14);
    // The server's order does not matter.
    mockFetch(200, [...DOC_TYPES].reverse().map(series));
    setActivePinia(createPinia());
    const w = mount(NumberSeriesTab, { global: { plugins: [i18n] } });
    await flushPromises();

    const keys = (group: string) =>
      w.findAll(`[data-test="series-group-${group}"] [data-test^="series-"]`).map((e) => e.attributes("data-test")?.replace("series-", ""));
    expect(keys("issued")).toEqual([...ISSUED_SERIES]);
    expect(keys("received")).toEqual([...RECEIVED_SERIES]);
    expect(w.find('[data-test="series-group-issued"] h2').text()).toBe("Issued documents");
    // Headings name the series in the plural in both groups.
    expect(w.find('[data-test="series-debit_note"] h2').text()).toBe("Debit notes");
    expect(w.find('[data-test="series-advance_tax_doc"] h2').text()).toBe("Advance payment tax documents");
    expect(w.find('[data-test="series-received_advance_tax_doc"] h2').text()).toBe("Received advance payment tax documents");
    expect(w.find('[data-test="series-received_simplified"] h2').text()).toBe("Received simplified tax documents");
  });
});
