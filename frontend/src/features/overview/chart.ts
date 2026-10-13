import { parseCents } from "@/lib/money";
import type { OverviewMonth } from "./types";

/** Chart box in SVG units (scaled to the container width). */
export const WIDTH = 640;
export const HEIGHT = 220;
const TOP = 8;
const LABELS = 22;
/** Rounded data end and the surface gap between a month's two bars. */
const RADIUS = 4;
const GAP = 2;

export interface Column {
  month: string;
  inPeriod: boolean;
  income: string;
  expense: string;
  /** SVG path of each bar (empty for a zero amount). */
  incomePath: string;
  expensePath: string;
  /** Centre of the month's slot (label x). */
  center: number;
}

/** A bar from the baseline `zero` to `y`, its data end rounded (up for positive, down for negative). */
export function barPath(x: number, w: number, zero: number, y: number): string {
  const h = Math.abs(zero - y);
  if (h < 0.5) return "";
  const r = Math.min(RADIUS, w / 2, h);
  const f = (n: number) => Math.round(n * 100) / 100;
  if (y < zero) {
    return `M${f(x)},${f(zero)}V${f(y + r)}Q${f(x)},${f(y)} ${f(x + r)},${f(y)}H${f(x + w - r)}Q${f(x + w)},${f(y)} ${f(x + w)},${f(y + r)}V${f(zero)}Z`;
  }
  return `M${f(x)},${f(zero)}V${f(y - r)}Q${f(x)},${f(y)} ${f(x + r)},${f(y)}H${f(x + w - r)}Q${f(x + w)},${f(y)} ${f(x + w)},${f(y - r)}V${f(zero)}Z`;
}

/** Income / expense bars per month on one shared scale that always includes zero (credit notes can go below). */
export function layout(months: OverviewMonth[]): { columns: Column[]; zero: number } {
  // Display geometry only: the float conversion of exact cents is harmless here.
  const num = (v: string) => Number(parseCents(v) ?? 0n) / 100;
  const values = months.flatMap((m) => [num(m.income), num(m.expense)]);
  const max = Math.max(0, ...values);
  const min = Math.min(0, ...values);
  const span = max - min || 1;
  const bottom = HEIGHT - LABELS;
  const y = (v: number) => TOP + ((max - v) / span) * (bottom - TOP);
  const zero = y(0);
  const slot = WIDTH / Math.max(months.length, 1);
  const bar = Math.min(28, slot * 0.34);
  const columns = months.map((m, i) => {
    const center = slot * i + slot / 2;
    return {
      month: m.month,
      inPeriod: m.inPeriod,
      income: m.income,
      expense: m.expense,
      incomePath: barPath(center - bar - GAP / 2, bar, zero, y(num(m.income))),
      expensePath: barPath(center + GAP / 2, bar, zero, y(num(m.expense))),
      center,
    };
  });
  return { columns, zero };
}
