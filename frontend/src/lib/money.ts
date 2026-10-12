/**
 * Money arithmetic on decimal strings in integer cents (BigInt), so sums, differences and
 * comparisons stay exact — never `Number(x) * 100`. Display formatting lives in `format.ts`.
 */

const AMOUNT = /^-?\d+([.,]\d{1,2})?$/;
const DECIMAL = /^([+-]?)(\d*)(?:[.,](\d*))?$/;

/** A user-entered amount (≤ 2 dp, `.` or `,`) → cents; null when not such an amount. */
export function toCents(value: string): bigint | null {
  const v = value.trim();
  return AMOUNT.test(v) ? parseCents(v) : null;
}

/**
 * Any decimal string (e.g. a server amount of another scale) → cents, rounded half away from
 * zero; null when not a number.
 */
export function parseCents(value: string | null | undefined): bigint | null {
  const m = DECIMAL.exec((value ?? "").trim());
  if (!m || (m[2] === "" && !m[3])) return null;
  const [, sign, whole, frac = ""] = m;
  let cents = BigInt(whole || "0") * 100n + BigInt(frac.slice(0, 2).padEnd(2, "0"));
  if (frac.length > 2 && Number(frac[2]) >= 5) cents += 1n;
  return sign === "-" ? -cents : cents;
}

/** Cents → a 2-dp decimal string ("-0.05"). */
export function fromCents(cents: bigint): string {
  const abs = cents < 0n ? -cents : cents;
  const frac = String(abs % 100n).padStart(2, "0");
  return `${cents < 0n ? "-" : ""}${abs / 100n}.${frac}`;
}

/** True when two amount strings denote the same cents ("210" = "210.00"). */
export function sameAmount(a: string | null | undefined, b: string | null | undefined): boolean {
  const ca = a == null ? null : toCents(a);
  return ca !== null && ca === (b == null ? null : toCents(b));
}

/** Wire value of an entered amount: dot decimal, trimmed. */
export const normalizeAmount = (v: string) => v.trim().replace(",", ".");

/** `payable − paid`, never below zero, as a 2-dp string. */
export function remainingAmount(payable: string, paid: string): string {
  const rest = (parseCents(payable) ?? 0n) - (parseCents(paid) ?? 0n);
  return fromCents(rest > 0n ? rest : 0n);
}

/** True for an amount below zero (at cent precision). */
export const isNegative = (amount: string | null | undefined): boolean => (parseCents(amount) ?? 0n) < 0n;

/** Negates a decimal string exactly, keeping its scale; "0.00" stays unsigned. */
export function negate(amount: string): string {
  const v = amount.trim();
  if (v.startsWith("-")) return v.slice(1);
  return /^[0.]*$/.test(v.replace(/^\+/, "")) ? v : `-${v.replace(/^\+/, "")}`;
}

/** Applies a document's sign (credit notes are stored positive, shown negative). */
export const signed = (amount: string, sign: 1 | -1): string => (sign === -1 ? negate(amount) : amount);
