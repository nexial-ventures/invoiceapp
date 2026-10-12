import type { DocLocale, Direction } from "@/api/types";
import type { Narrow, Wire } from "@/api/wire";

/**
 * `GET/PUT /api/settings/company` — singleton, seeded with name "". `default*`: the defaults a new
 * document starts from (4e).
 */
export type Company = Narrow<Wire<"Company">, { defaultLocale: DocLocale }>;

export type LocalizedNote = Wire<"LocaleNotes">;

/** `/api/settings/bank-accounts` */
export type BankAccount = Wire<"BankAccount">;
export type BankAccountInput = Omit<BankAccount, "id">;

/** `/api/settings/vat-rates` — `rate` is a decimal string, e.g. "21" or "12.5". */
export type VatRate = Wire<"VatRate">;
export type VatRateInput = Omit<VatRate, "id">;

/** `/api/settings/number-series` — `nextNumberPreview` is for the current year. */
export type NumberSeries = Wire<"NumberSeries">;

/**
 * `GET /api/pdf/design` — the live design (built-in ∪ storage keys `design/…`, sorted by path), the
 * draft and the quota. `storage`: the backend the custom files come from; a file's `source`
 * `custom` = from the storage (overrides the built-in file of the same path); sizes in bytes.
 */
export type DesignInfo = Narrow<Wire<"DesignListing">, { storage: "fs" | "s3" }>;
export type DesignFile = Wire<"DesignFile">;
/**
 * Pending changes: uploaded files and live custom files to delete, applied on activation;
 * `updatedAt` / `updatedBy` null until a change was recorded. A file's `replaced` = overrides a
 * live file of the same path.
 */
export type DesignDraft = Wire<"DraftListing">;
/** Null in `updatedBy` / `createdBy` when the account is gone. */
export type DesignAuthor = Wire<"UserRef">;
/** Bytes over live + draft + kept versions. */
export type DesignQuota = Wire<"Quota">;
/** `GET /api/pdf/design/file?source=…`. */
export type DesignFileSource = "live" | "draft" | "default";

export type DesignVersionKind = "activation" | "revert" | "default";
/** `GET /api/pdf/design/versions`, newest first; `current` = the live set. */
export type DesignVersion = Narrow<Wire<"Version">, { kind: DesignVersionKind }>;

export const CATEGORY_KINDS = ["expense", "income"] as const;
export type CategoryKind = (typeof CATEGORY_KINDS)[number];

/** `/api/settings/categories` — received documents take `expense`, issued `income`. */
export type Category = Narrow<Wire<"Category">, { kind: CategoryKind }>;
export type CategoryInput = Omit<Category, "id">;

export const CUSTOM_FIELD_TYPES = ["text", "number", "date", "bool", "select"] as const;
export type CustomFieldType = (typeof CUSTOM_FIELD_TYPES)[number];
export const CUSTOM_FIELD_SCOPES = ["issued", "received", "both"] as const;
export type CustomFieldScope = (typeof CUSTOM_FIELD_SCOPES)[number];

/** `/api/settings/custom-fields` — `key` and `type` are immutable after create; `options`: `select` only. */
export type CustomField = Narrow<Wire<"CustomField">, { type: CustomFieldType; appliesTo: CustomFieldScope }>;
export type CustomFieldInput = Omit<CustomField, "id">;

/** The document types exported to accounting programs (no proformas), in display order. */
export const ACCOUNTING_DOC_TYPES = ["invoice", "credit_note", "debit_note", "advance_tax_doc", "advance_credit_note", "simplified"] as const;
export type AccountingDocType = (typeof ACCOUNTING_DOC_TYPES)[number];
export const ACCOUNTING_DIRECTIONS = ["issued", "received"] as const satisfies readonly Direction[];

/** Accounting programs with their own codes in Settings → Accounting, in display order. */
export const ACCOUNTING_PROGRAMS = ["pohoda", "money"] as const;
export type AccountingProgram = (typeof ACCOUNTING_PROGRAMS)[number];

/**
 * One direction × doc type row of an accounting program's codes; null = the element is not
 * written. `accounting` = předkontace (Pohoda `typ:ids`, Money `PredKontac`), `classificationVat` =
 * členění DPH (Pohoda `typ:ids`, Money `KodDPH`), `numberSeries` = číselná řada (Pohoda `typ:ids`,
 * Money `Rada`), `classificationVatNonDeductible` = členění DPH of a received document without VAT
 * deduction (`vatDeductible` false; received rows only).
 */
export type AccountingCodes = Narrow<Wire<"CodeRow">, { direction: Direction; docType: AccountingDocType }>;

/** One program's section: the accounting unit override (Pohoda `dataPack/@ico`, Money `MoneyData/@ICAgendy`; null → the company IČO) and every direction × doc type row. */
export type AccountingProgramSettings = Narrow<Wire<"ProgramSettings">, { codes: AccountingCodes[] }>;

/** `GET/PUT /api/settings/accounting` — GET returns both sections with every direction × doc type row. */
export type AccountingSettings = Narrow<Wire<"AccountingSettings">, Record<AccountingProgram, AccountingProgramSettings>>;
