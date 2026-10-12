import type { RouteLocationRaw } from "vue-router";
import type { Direction } from "@/api/types";
import { LIST_DOC_TYPES, type ListDocType } from "./store";

const ROUTES = {
  issued: { list: "invoices", detail: "invoice-detail", edit: "invoice-edit" },
  received: { list: "received", detail: "received-detail", edit: "received-edit" },
} as const;

/** The list tab is the `?type=` query (absent = invoices), so links can open a tab. */
export function listLocation(docType: string, direction: Direction = "issued"): RouteLocationRaw {
  return { name: ROUTES[direction].list, query: docType === "invoice" ? {} : { type: docType } };
}

export function listDocTypeOf(query: unknown): ListDocType {
  return (LIST_DOC_TYPES as readonly unknown[]).includes(query) ? (query as ListDocType) : "invoice";
}

export const detailLocation = (id: string, direction: Direction = "issued"): RouteLocationRaw => ({
  name: ROUTES[direction].detail,
  params: { id },
});
export const editLocation = (id: string, direction: Direction = "issued"): RouteLocationRaw => ({
  name: ROUTES[direction].edit,
  params: { id },
});
