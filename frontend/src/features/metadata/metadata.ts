import type { FieldErrors, Direction } from "@/api/types";
import { nullIfEmpty, textRule } from "@/lib/formErrors";
import type { CustomFieldValues, MetadataInput } from "@/features/documents/types";
import type { Category, CategoryKind, CustomField } from "@/features/settings/types";

/**
 * Category, custom fields and internal note of a document — shared by the issued editor, the
 * received form and the detail "Metadata" panel. `defs` below are always the *applicable* field
 * definitions (see `applicableFields`); stored values of other keys travel through untouched.
 */

export const CUSTOM_FIELD_ERROR_PREFIX = "customFields.";
const TEXT_MAX = 500;

/** Active definitions for the document's direction, in display order. */
export function applicableFields(defs: CustomField[], direction: Direction): CustomField[] {
  return defs
    .filter((d) => d.active && (d.appliesTo === "both" || d.appliesTo === direction))
    .sort((a, b) => a.position - b.position || a.label.localeCompare(b.label));
}

export const categoryKindOf = (direction: Direction): CategoryKind => (direction === "received" ? "expense" : "income");

/** Assignable categories of the kind, plus the current one even if it was deactivated since. */
export function categoryOptions(categories: Category[], kind: CategoryKind, currentId: string | null): Category[] {
  return categories
    .filter((c) => c.kind === kind && (c.active || c.id === currentId))
    .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name));
}

const textOf = (value: string | boolean | null | undefined): string =>
  typeof value === "string" ? value.trim() : value == null ? "" : String(value);

/** Editor values: the stored ones plus an entry per applicable field, so every input has a key to bind. */
export function toCustomFieldDraft(values: CustomFieldValues | null | undefined, defs: CustomField[]): CustomFieldValues {
  const out: CustomFieldValues = { ...(values ?? {}) };
  for (const def of defs) {
    const value = out[def.key];
    if (def.type === "bool") out[def.key] = value === true;
    else out[def.key] = value == null ? "" : String(value);
  }
  return out;
}

/** Wire values: "" → null, decimal comma → dot; keys without an applicable definition go back unchanged. */
export function serializeCustomFields(values: CustomFieldValues, defs: CustomField[]): CustomFieldValues {
  const byKey = new Map(defs.map((d) => [d.key, d]));
  const out: CustomFieldValues = {};
  for (const [key, value] of Object.entries(values)) {
    const def = byKey.get(key);
    if (!def) out[key] = value;
    else if (def.type === "bool") out[key] = value === true;
    else {
      const text = textOf(value);
      out[key] = text === "" ? null : def.type === "number" ? text.replace(",", ".") : text;
    }
  }
  return out;
}

function fieldReason(def: CustomField, value: string | boolean | null | undefined): string | null {
  // A checkbox always holds a value (false included).
  if (def.type === "bool") return null;
  const text = textOf(value);
  if (text === "") return def.required ? "required" : null;
  switch (def.type) {
    case "text":
      return text.length > TEXT_MAX ? "too_long" : null;
    case "number":
      return /^-?\d+([.,]\d+)?$/.test(text) ? null : "invalid";
    case "date":
      return /^\d{4}-\d{2}-\d{2}$/.test(text) ? null : "invalid";
    case "select":
      return def.options.includes(text) ? null : "invalid";
  }
}

/** Client mirror of the server rules, keyed like its 422: `customFields.<key>`. */
export function validateCustomFields(values: CustomFieldValues, defs: CustomField[]): FieldErrors {
  const errors: FieldErrors = {};
  for (const def of defs) {
    const reason = fieldReason(def, values[def.key]);
    if (reason) errors[`${CUSTOM_FIELD_ERROR_PREFIX}${def.key}`] = reason;
  }
  return errors;
}

/** `customFields.<key>` errors by key. */
export function customFieldErrors(errors: FieldErrors): Record<string, string> {
  return Object.fromEntries(
    Object.entries(errors)
      .filter(([k]) => k.startsWith(CUSTOM_FIELD_ERROR_PREFIX))
      .map(([k, v]) => [k.slice(CUSTOM_FIELD_ERROR_PREFIX.length), v]),
  );
}

/** Form state of the three metadata fields; nullable strings are "". */
export interface MetadataDraft {
  categoryId: string;
  customFields: CustomFieldValues;
  internalNote: string;
}

export function toMetadataDraft(
  doc: { categoryId: string | null; customFields: CustomFieldValues | null; internalNote: string | null },
  defs: CustomField[],
): MetadataDraft {
  return {
    categoryId: doc.categoryId ?? "",
    customFields: toCustomFieldDraft(doc.customFields, defs),
    internalNote: doc.internalNote ?? "",
  };
}

export function validateMetadata(d: MetadataDraft, defs: CustomField[]): FieldErrors {
  const errors = validateCustomFields(d.customFields, defs);
  const note = textRule(d.internalNote, { max: 2000 });
  if (note) errors.internalNote = note;
  return errors;
}

export function toMetadataInput(d: MetadataDraft, defs: CustomField[]): MetadataInput {
  return {
    categoryId: d.categoryId || null,
    customFields: serializeCustomFields(d.customFields, defs),
    internalNote: nullIfEmpty(d.internalNote),
  };
}
