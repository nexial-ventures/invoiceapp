import { request } from "@/api/client";
import type { Overview } from "./types";

export const overviewApi = {
  /** The tax-date period `from..=to` (`YYYY-MM-DD`, at most 366 days apart). */
  get: (from: string, to: string) => request<Overview>(`/api/overview?${new URLSearchParams({ from, to }).toString()}`),
};
