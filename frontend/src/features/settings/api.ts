import { request, requestBlob } from "@/api/client";
import type { DocLocale } from "@/api/types";
import type {
  AccountingSettings,
  BankAccount,
  BankAccountInput,
  Category,
  CategoryInput,
  Company,
  CustomField,
  CustomFieldInput,
  DesignFileSource,
  DesignInfo,
  DesignVersion,
  DocType,
  NumberSeries,
  VatRate,
  VatRateInput,
} from "./types";

const BASE = "/api/settings";

export interface CrudApi<T, I> {
  list(): Promise<T[]>;
  create(input: I): Promise<T>;
  update(id: string, input: I): Promise<T>;
  remove(id: string): Promise<void>;
}

function crudApi<T, I>(path: string): CrudApi<T, I> {
  const url = `${BASE}/${path}`;
  const item = (id: string) => `${url}/${encodeURIComponent(id)}`;
  return {
    list: () => request<T[]>(url),
    create: (input) => request<T>(url, { method: "POST", body: input }),
    update: (id, input) => request<T>(item(id), { method: "PUT", body: input }),
    remove: (id) => request<void>(item(id), { method: "DELETE" }),
  };
}

export const companyApi = {
  get: () => request<Company>(`${BASE}/company`),
  update: (company: Company) => request<Company>(`${BASE}/company`, { method: "PUT", body: company }),
};

export const accountingApi = {
  get: () => request<AccountingSettings>(`${BASE}/accounting`),
  update: (settings: AccountingSettings) => request<AccountingSettings>(`${BASE}/accounting`, { method: "PUT", body: settings }),
};

export const bankAccountsApi = crudApi<BankAccount, BankAccountInput>("bank-accounts");
export const vatRatesApi = crudApi<VatRate, VatRateInput>("vat-rates");
export const categoriesApi = crudApi<Category, CategoryInput>("categories");
export const customFieldsApi = crudApi<CustomField, CustomFieldInput>("custom-fields");

export const numberSeriesApi = {
  list: () => request<NumberSeries[]>(`${BASE}/number-series`),
  updatePattern: (docType: DocType, pattern: string) =>
    request<NumberSeries>(`${BASE}/number-series/${docType}`, { method: "PUT", body: { pattern } }),
  setCounter: (docType: DocType, year: number, lastNumber: number) =>
    request<NumberSeries>(`${BASE}/number-series/${docType}/counters/${year}`, {
      method: "PUT",
      body: { lastNumber },
    }),
};

const DESIGN = "/api/pdf/design";
const query = (params: Record<string, string>) => new URLSearchParams(params).toString();

export const pdfApi = {
  design: () => request<DesignInfo>(DESIGN),
  /** Sample invoice rendered with the current design; never stored. */
  preview: (locale: DocLocale) => requestBlob(`/api/pdf/preview?${query({ locale })}`),
  designFile: (path: string, source: DesignFileSource) => requestBlob(`${DESIGN}/file?${query({ path, source })}`),
  // The listing in the PUT / DELETE answers is not relied on: the store reloads the design (the quota moves too).
  uploadDraftFile: (path: string, file: Blob) =>
    request<unknown>(`${DESIGN}/draft/file?${query({ path })}`, { method: "PUT", body: file }),
  removeDraftFile: (path: string) => request<unknown>(`${DESIGN}/draft/file?${query({ path })}`, { method: "DELETE" }),
  discardDraft: () => request<void>(`${DESIGN}/draft`, { method: "DELETE" }),
  /** Sample invoice rendered with live ∪ draft. */
  draftPreview: (locale: DocLocale) => requestBlob(`${DESIGN}/draft/preview?${query({ locale })}`),
  activate: () => request<unknown>(`${DESIGN}/activate`, { method: "POST" }),
  versions: () => request<DesignVersion[]>(`${DESIGN}/versions`),
  restore: (id: string) => request<unknown>(`${DESIGN}/versions/${encodeURIComponent(id)}/restore`, { method: "POST" }),
  reset: () => request<unknown>(`${DESIGN}/reset`, { method: "POST" }),
};
