import type { LocationQuery } from "vue-router";
import { todayIso } from "@/features/documents/form";

export const PERIOD_KINDS = ["month", "quarter", "year", "custom"] as const;
export type PeriodKind = (typeof PERIOD_KINDS)[number];
export type PresetKind = Exclude<PeriodKind, "custom">;

/** A tax-date period, both ends inclusive (`YYYY-MM-DD`). */
export interface Period {
  kind: PeriodKind;
  from: string;
  to: string;
}

const MONTHS: Record<PresetKind, number> = { month: 1, quarter: 3, year: 12 };

/** A real calendar date `YYYY-MM-DD` → its local Date; anything else → null. */
function parseIso(value: unknown): Date | null {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return null;
  const [y, m, d] = value.split("-").map(Number) as [number, number, number];
  const date = new Date(y, m - 1, d);
  return todayIso(date) === value ? date : null;
}

/** The month / quarter / year containing `day`. */
export function presetOf(kind: PresetKind, day: Date): Period {
  const y = day.getFullYear();
  const months = MONTHS[kind];
  const first = day.getMonth() - (day.getMonth() % months);
  // Day 0 of the following month is the last day of the period; Date rolls the year over.
  return { kind, from: todayIso(new Date(y, first, 1)), to: todayIso(new Date(y, first + months, 0)) };
}

/** The default: the current calendar month. */
export const currentMonth = (now = new Date()): Period => presetOf("month", now);

/** The preset period `step` periods later (negative: earlier); a custom period stays. */
export function shift(period: Period, step: number): Period {
  const from = parseIso(period.from);
  if (period.kind === "custom" || !from) return period;
  return presetOf(period.kind, new Date(from.getFullYear(), from.getMonth() + step * MONTHS[period.kind], 1));
}

/** The quarter (1–4) a preset quarter starts in. */
export const quarterOf = (period: Period): number => Math.floor(Number(period.from.slice(5, 7)) / 3) + 1;

/**
 * The period kept in the URL (`period`, `from`, `to`). A preset is normalized to the one containing `from`;
 * a custom period needs `from ≤ to`. Anything unusable → the current month (the 366-day cap is the server's).
 */
export function fromQuery(query: LocationQuery, now = new Date()): Period {
  const kind = PERIOD_KINDS.find((k) => k === query.period);
  const from = parseIso(query.from);
  if (!kind || !from) return currentMonth(now);
  if (kind !== "custom") return presetOf(kind, from);
  const to = parseIso(query.to);
  return to && to >= from ? { kind, from: String(query.from), to: String(query.to) } : currentMonth(now);
}

export const toQuery = (period: Period): Record<string, string> => ({ period: period.kind, from: period.from, to: period.to });

export const samePeriod = (a: Period, b: Period): boolean => a.kind === b.kind && a.from === b.from && a.to === b.to;

/** The period of another kind around the current one's start (custom keeps the dates). */
export function switchKind(period: Period, kind: PeriodKind): Period {
  if (kind === "custom") return { ...period, kind };
  return presetOf(kind, parseIso(period.from) ?? new Date());
}
