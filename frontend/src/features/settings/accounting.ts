import type { FieldErrors, Direction } from "@/api/types";
import { nullIfEmpty, textRule } from "@/lib/formErrors";
import {
  ACCOUNTING_DIRECTIONS,
  ACCOUNTING_DOC_TYPES,
  ACCOUNTING_PROGRAMS,
  type AccountingCodes,
  type AccountingLimits,
  type AccountingProgram,
  type AccountingProgramSettings,
  type AccountingSettings,
} from "./types";

/** The code columns of the codes table, in display order. */
export const CODE_FIELDS = ["accounting", "classificationVat", "classificationVatNonDeductible", "numberSeries"] as const;
export type CodeField = (typeof CODE_FIELDS)[number];

/** Whether the column has an input in a row of that direction (VAT deduction exists only for received documents). */
export const codeApplies = (field: CodeField, direction: Direction): boolean =>
  field !== "classificationVatNonDeductible" || direction === "received";

/** The server's limit for `field` of `program` (after trim), from the response's `limits`. */
export const codeMaxLength = (limits: AccountingLimits, program: AccountingProgram, field: CodeField): number =>
  field === "numberSeries" ? limits[program].numberSeries : limits[program].code;

export type CodeRowDraft = Pick<AccountingCodes, "direction" | "docType"> & Record<CodeField, string>;

export interface ProgramDraft {
  ico: string;
  /** Every direction × doc type in display order; the index is the `N` of the 422 keys `<program>.codes.N.*`. */
  rows: CodeRowDraft[];
}

export type AccountingDraft = Record<AccountingProgram, ProgramDraft>;

/** Rows are rebuilt in display order rather than taken as sent, so a missing row still gets its inputs. */
function toProgramDraft(section: AccountingProgramSettings | undefined): ProgramDraft {
  const codes = section?.codes ?? [];
  const rows = ACCOUNTING_DIRECTIONS.flatMap((direction) =>
    ACCOUNTING_DOC_TYPES.map((docType) => {
      const row = codes.find((c) => c.direction === direction && c.docType === docType);
      return {
        direction,
        docType,
        accounting: row?.accounting ?? "",
        classificationVat: row?.classificationVat ?? "",
        numberSeries: row?.numberSeries ?? "",
        classificationVatNonDeductible: row?.classificationVatNonDeductible ?? "",
      };
    }),
  );
  return { ico: section?.ico ?? "", rows };
}

export function toAccountingDraft(settings: AccountingSettings | null): AccountingDraft {
  return { pohoda: toProgramDraft(settings?.pohoda), money: toProgramDraft(settings?.money) };
}

/** `""` → null. */
function toProgramSettings(draft: ProgramDraft): AccountingProgramSettings {
  return {
    ico: nullIfEmpty(draft.ico),
    codes: draft.rows.map((r) => ({
      direction: r.direction,
      docType: r.docType,
      accounting: nullIfEmpty(r.accounting),
      classificationVat: nullIfEmpty(r.classificationVat),
      numberSeries: nullIfEmpty(r.numberSeries),
      classificationVatNonDeductible: codeApplies("classificationVatNonDeductible", r.direction)
        ? nullIfEmpty(r.classificationVatNonDeductible)
        : null,
    })),
  };
}

/** Always both sections: the server keeps a section absent from the PUT, so sending both makes Save authoritative. */
export function toAccountingSettings(draft: AccountingDraft): AccountingSettings {
  return { pohoda: toProgramSettings(draft.pohoda), money: toProgramSettings(draft.money) };
}

/** Client mirror of the server's length checks (the `limits` it returned), keyed like its 422 (`<program>.codes.N.<field>`). */
export function validateAccounting(draft: AccountingDraft, limits: AccountingLimits): FieldErrors {
  const errors: FieldErrors = {};
  for (const program of ACCOUNTING_PROGRAMS) {
    draft[program].rows.forEach((row, i) => {
      for (const field of CODE_FIELDS) {
        const reason = textRule(row[field], { max: codeMaxLength(limits, program, field) });
        if (reason) errors[`${program}.codes.${i}.${field}`] = reason;
      }
    });
  }
  return errors;
}
