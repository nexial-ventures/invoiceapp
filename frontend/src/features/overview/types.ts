import type { Wire } from "@/api/wire";

/** `GET /api/overview`: the period's figures and today's open documents, every amount in CZK. */
export type Overview = Wire<"Overview">;
export type OverviewTotals = Wire<"OverviewTotals">;
export type OverviewVat = Wire<"OverviewVat">;
export type OverviewOpen = Wire<"OverviewOpen">;
export type OverviewMonth = Wire<"OverviewMonth">;
export type OverviewCategory = Wire<"OverviewCategoryAmount">;
