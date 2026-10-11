import type { RoundingMode, VatMode } from "@/features/documents/types";

/**
 * VAT recap rows of a received document, entered as printed on the supplier's document.
 * Amounts are handled as integer cents (BigInt) so sums and the VAT suggestion stay exact.
 */
export interface RecapRowDraft {
  /** Client-only identity for `v-for`. */
  key: number;
  rate: string;
  base: string;
  vat: string;
  /** VAT follows round2(base × rate / 100) until the user types their own. */
  vatAuto: boolean;
}

let lastKey = 0;
export const newRecapRow = (rate: string): RecapRowDraft => ({ key: ++lastKey, rate, base: "", vat: "", vatAuto: true });

const AMOUNT = /^-?\d+([.,]\d{1,2})?$/;
const RATE = /^\d{1,3}([.,]\d{1,2})?$/;

/** 2-dp decimal string → cents; null when not a valid amount. */
export function toCents(value: string): bigint | null {
  const v = value.trim();
  if (!AMOUNT.test(v)) return null;
  const negative = v.startsWith("-");
  const [whole = "0", frac = ""] = v.replace("-", "").split(/[.,]/);
  const cents = BigInt(whole) * 100n + BigInt(frac.padEnd(2, "0"));
  return negative ? -cents : cents;
}

export function fromCents(cents: bigint): string {
  const abs = cents < 0n ? -cents : cents;
  const frac = String(abs % 100n).padStart(2, "0");
  return `${cents < 0n ? "-" : ""}${abs / 100n}.${frac}`;
}

/** Rate in hundredths of a percent (21 → 2100), null unless 0..100 with ≤ 2 dp. */
export function rateHundredths(value: string): bigint | null {
  const v = value.trim();
  if (!RATE.test(v)) return null;
  const [whole = "0", frac = ""] = v.split(/[.,]/);
  const h = BigInt(whole) * 100n + BigInt(frac.padEnd(2, "0"));
  return h <= 10000n ? h : null;
}

/** round2(base × rate / 100), half away from zero; "" while base or rate is incomplete. */
export function suggestVat(base: string, rate: string): string {
  const cents = toCents(base);
  const h = rateHundredths(rate);
  if (cents === null || h === null) return "";
  const product = cents * h;
  const abs = product < 0n ? -product : product;
  let q = abs / 10000n;
  if ((abs % 10000n) * 2n >= 10000n) q += 1n;
  return fromCents(product < 0n ? -q : q);
}

/** Only the standard VAT mode carries VAT; any other mode forces 0. */
export const vatAllowed = (mode: VatMode) => mode === "standard";

/** Applies an edit; typing a VAT amount stops the suggestion, clearing it resumes it. */
export function updateRow(row: RecapRowDraft, patch: Partial<Pick<RecapRowDraft, "rate" | "base" | "vat">>, mode: VatMode): RecapRowDraft {
  const next = { ...row, ...patch };
  if (patch.vat !== undefined) next.vatAuto = patch.vat.trim() === "";
  if (!vatAllowed(mode)) return { ...next, vat: "0" };
  return next.vatAuto ? { ...next, vat: suggestVat(next.base, next.rate) } : next;
}

/** Re-applies the mode rule to every row (VAT 0 outside standard; suggestions back in standard). */
export function enforceRecapVatMode(rows: RecapRowDraft[], mode: VatMode): RecapRowDraft[] {
  return rows.map((r) => {
    if (!vatAllowed(mode)) return { ...r, vat: "0", vatAuto: true };
    return r.vatAuto ? { ...r, vat: suggestVat(r.base, r.rate) } : r;
  });
}

/** Σ(base + vat) + rounding as a 2-dp string; null while any amount is not valid. */
export function recapTotal(rows: { base: string; vat: string }[], rounding: string): string | null {
  let sum = toCents(rounding.trim() === "" ? "0" : rounding);
  if (sum === null) return null;
  for (const r of rows) {
    const base = toCents(r.base);
    const vat = toCents(r.vat.trim() === "" ? "0" : r.vat);
    if (base === null || vat === null) return null;
    sum += base + vat;
  }
  return fromCents(sum);
}

/**
 * Mode the server derives on save from the entered amounts (informational for received documents):
 * no rounding amount → none; Σ(base + vat) + rounding on a whole / 0.50 multiple → unit / half;
 * any other ("odd") rounding → none. null while an amount is not valid.
 */
export function derivedRoundingMode(rows: { base: string; vat: string }[], rounding: string): RoundingMode | null {
  const amount = toCents(rounding.trim() === "" ? "0" : rounding);
  const total = recapTotal(rows, rounding);
  if (amount === null || total === null) return null;
  const cents = toCents(total) ?? 0n;
  if (amount === 0n) return "none";
  if (cents % 100n === 0n) return "unit";
  return cents % 50n === 0n ? "half" : "none";
}

/** True when two amount strings denote the same cents ("210" = "210.00"). */
export function sameAmount(a: string | null | undefined, b: string | null | undefined): boolean {
  const ca = a == null ? null : toCents(a);
  return ca !== null && ca === (b == null ? null : toCents(b));
}

/** 422 keys for row fields, `vatRecap.N.rate|base|vat` (N 0-based) — mirrored by the client checks. */
export function validateRecap(rows: RecapRowDraft[], mode: VatMode): Record<string, string> {
  const errors: Record<string, string> = {};
  if (rows.length === 0) errors.vatRecap = "required";
  const seen = new Set<string>();
  rows.forEach((r, i) => {
    const h = rateHundredths(r.rate);
    if (r.rate.trim() === "") errors[`vatRecap.${i}.rate`] = "required";
    else if (h === null) errors[`vatRecap.${i}.rate`] = "invalid";
    else if (seen.has(String(h))) errors[`vatRecap.${i}.rate`] = "duplicate";
    else seen.add(String(h));
    if (r.base.trim() === "") errors[`vatRecap.${i}.base`] = "required";
    else if (toCents(r.base) === null) errors[`vatRecap.${i}.base`] = "invalid";
    const vat = toCents(r.vat.trim() === "" ? "0" : r.vat);
    if (vat === null || (!vatAllowed(mode) && vat !== 0n)) errors[`vatRecap.${i}.vat`] = "invalid";
  });
  return errors;
}

/** Wire value of an amount: dot decimal, trimmed. */
export const normalizeAmount = (v: string) => v.trim().replace(",", ".");
