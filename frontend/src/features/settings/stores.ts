import { defineStore } from "pinia";
import { ref, type Ref } from "vue";
import type { DocType } from "@/api/types";
import { loadOnce } from "@/lib/loadOnce";
import { accountingApi, bankAccountsApi, categoriesApi, companyApi, customFieldsApi, numberSeriesApi, pdfApi, vatRatesApi, type CrudApi } from "./api";
import type { AccountingResponse, AccountingSettings, BankAccount, BankAccountInput, Category, CategoryInput, Company, CustomField, CustomFieldInput, DesignInfo, DesignVersion, NumberSeries, VatRate, VatRateInput } from "./types";

export const useCompanyStore = defineStore("settings/company", () => {
  const company = ref<Company | null>(null);

  async function load(): Promise<void> {
    company.value = await companyApi.get();
  }

  const ensure = loadOnce(load, () => company.value !== null);

  async function save(input: Company): Promise<void> {
    company.value = await companyApi.update(input);
  }

  return { company, load, ensure, save };
});

export const useAccountingStore = defineStore("settings/accounting", () => {
  const settings = ref<AccountingResponse | null>(null);

  async function load(): Promise<void> {
    settings.value = await accountingApi.get();
  }

  async function save(input: AccountingSettings): Promise<void> {
    settings.value = await accountingApi.update(input);
  }

  return { settings, load, save };
});

/**
 * List settings whose default flag is owned by the server: saving one default
 * unsets the others (per currency for bank accounts, globally for VAT rates)
 * and the first bank account of a currency becomes default. Rather than mirror
 * those rules, every mutation reloads the list.
 */
function defineCrudStore<T extends { id: string }, I>(id: string, api: CrudApi<T, I>) {
  return defineStore(id, () => {
    const items = ref([]) as Ref<T[]>;
    const loaded = ref(false);

    async function load(): Promise<void> {
      items.value = await api.list();
      loaded.value = true;
    }

    const ensure = loadOnce(load, () => loaded.value);

    async function save(input: I, itemId?: string): Promise<void> {
      if (itemId) await api.update(itemId, input);
      else await api.create(input);
      await load();
    }

    async function remove(itemId: string): Promise<void> {
      await api.remove(itemId);
      await load();
    }

    return { items, loaded, load, ensure, save, remove };
  });
}

export const useBankAccountsStore = defineCrudStore<BankAccount, BankAccountInput>(
  "settings/bankAccounts",
  bankAccountsApi,
);

export const useVatRatesStore = defineCrudStore<VatRate, VatRateInput>("settings/vatRates", vatRatesApi);
export const useCategoriesStore = defineCrudStore<Category, CategoryInput>("settings/categories", categoriesApi);
export const useCustomFieldsStore = defineCrudStore<CustomField, CustomFieldInput>("settings/customFields", customFieldsApi);

export const useNumberSeriesStore = defineStore("settings/numberSeries", () => {
  const series = ref<NumberSeries[]>([]);

  async function load(): Promise<void> {
    series.value = await numberSeriesApi.list();
  }

  function replace(saved: NumberSeries): void {
    series.value = series.value.map((s) => (s.docType === saved.docType ? saved : s));
  }

  async function savePattern(docType: DocType, pattern: string): Promise<void> {
    replace(await numberSeriesApi.updatePattern(docType, pattern));
  }

  async function setCounter(docType: DocType, year: number, lastNumber: number): Promise<void> {
    replace(await numberSeriesApi.setCounter(docType, year, lastNumber));
  }

  return { series, load, savePattern, setCounter };
});

/**
 * The design is reloaded on each visit (`invoiceapp design push` edits live behind the UI's back) and after
 * every change: the server's answer is not merged, the quota and the version list move with it.
 */
export const useDesignStore = defineStore("settings/design", () => {
  const design = ref<DesignInfo | null>(null);
  const versions = ref<DesignVersion[]>([]);

  async function load(): Promise<void> {
    [design.value, versions.value] = await Promise.all([pdfApi.design(), pdfApi.versions()]);
  }

  async function reloadDesign(): Promise<void> {
    design.value = await pdfApi.design();
  }

  // Draft changes leave live and the history alone; activation, restore and reset replace live and add a version.
  async function upload(path: string, file: Blob): Promise<void> {
    await pdfApi.uploadDraftFile(path, file);
    await reloadDesign();
  }
  async function removeDraftFile(path: string): Promise<void> {
    await pdfApi.removeDraftFile(path);
    await reloadDesign();
  }
  async function discard(): Promise<void> {
    await pdfApi.discardDraft();
    await reloadDesign();
  }
  async function activate(): Promise<void> {
    await pdfApi.activate();
    await load();
  }
  async function restore(id: string): Promise<void> {
    await pdfApi.restore(id);
    await load();
  }
  async function reset(): Promise<void> {
    await pdfApi.reset();
    await load();
  }

  return {
    design,
    versions,
    load,
    upload,
    removeDraftFile,
    discard,
    activate,
    restore,
    reset,
  };
});
