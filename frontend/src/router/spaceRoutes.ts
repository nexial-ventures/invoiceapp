import type { RouteRecordRaw } from "vue-router";
import { accountRoutes } from "./authRoutes";

/** The app on a space host: every business view of phases 1–3 + login, account and API tokens. */
export const spaceRoutes: RouteRecordRaw[] = [
  ...accountRoutes,
  // The space's landing page: the period overview (every role).
  {
    path: "/",
    name: "home",
    component: () => import("@/features/overview/views/OverviewView.vue"),
  },
  {
    // Reached from the invitation e-mail, signed out or as anyone: the page signs the invitee in.
    path: "/invite",
    name: "invite",
    component: () => import("@/features/members/views/InviteAcceptView.vue"),
    meta: { public: true },
  },
  {
    path: "/tokens",
    name: "tokens",
    component: () => import("@/features/tokens/views/TokensView.vue"),
  },

  {
    path: "/invoices",
    name: "invoices",
    component: () => import("@/features/documents/views/InvoicesListView.vue"),
  },
  {
    path: "/invoices/new",
    name: "invoice-new",
    component: () => import("@/features/documents/views/InvoiceEditView.vue"),
    meta: { can: "write" },
  },
  {
    path: "/invoices/:id",
    name: "invoice-detail",
    component: () => import("@/features/documents/views/InvoiceDetailView.vue"),
  },
  {
    path: "/invoices/:id/edit",
    name: "invoice-edit",
    component: () => import("@/features/documents/views/InvoiceEditView.vue"),
    meta: { can: "write" },
  },
  {
    path: "/received",
    name: "received",
    component: () => import("@/features/received/views/ReceivedListView.vue"),
  },
  {
    path: "/received/new",
    name: "received-new",
    component: () => import("@/features/received/views/ReceivedEditView.vue"),
    meta: { can: "write" },
  },
  {
    path: "/received/:id",
    name: "received-detail",
    component: () => import("@/features/received/views/ReceivedDetailView.vue"),
  },
  {
    path: "/received/:id/edit",
    name: "received-edit",
    component: () => import("@/features/received/views/ReceivedEditView.vue"),
    meta: { can: "write" },
  },
  {
    path: "/import/isdoc",
    name: "isdoc-import",
    component: () => import("@/features/isdoc/views/IsdocImportView.vue"),
    meta: { can: "write" },
  },
  {
    path: "/import/csv",
    name: "csv-import",
    component: () => import("@/features/csvImport/views/CsvImportView.vue"),
    meta: { can: "write" },
  },
  {
    path: "/contacts",
    name: "contacts",
    component: () => import("@/features/contacts/views/ContactsListView.vue"),
  },
  {
    path: "/contacts/new",
    name: "contact-new",
    component: () => import("@/features/contacts/views/ContactEditView.vue"),
    meta: { can: "write" },
  },
  {
    path: "/contacts/:id",
    name: "contact-edit",
    component: () => import("@/features/contacts/views/ContactEditView.vue"),
  },
  {
    path: "/catalog",
    component: () => import("@/features/catalog/views/CatalogView.vue"),
    children: [
      { path: "", redirect: { name: "catalog-items" } },
      {
        path: "items",
        name: "catalog-items",
        component: () => import("@/features/catalog/views/CatalogItemsTab.vue"),
      },
      {
        path: "groups",
        name: "catalog-groups",
        component: () => import("@/features/catalog/views/CatalogGroupsTab.vue"),
      },
    ],
  },
  {
    path: "/settings",
    component: () => import("@/features/settings/views/SettingsView.vue"),
    meta: { can: "settings" },
    children: [
      { path: "", redirect: { name: "settings-company" } },
      {
        path: "company",
        name: "settings-company",
        component: () => import("@/features/settings/views/CompanyTab.vue"),
      },
      {
        path: "bank-accounts",
        name: "settings-bank-accounts",
        component: () => import("@/features/settings/views/BankAccountsTab.vue"),
      },
      {
        path: "vat-rates",
        name: "settings-vat-rates",
        component: () => import("@/features/settings/views/VatRatesTab.vue"),
      },
      {
        path: "number-series",
        name: "settings-number-series",
        component: () => import("@/features/settings/views/NumberSeriesTab.vue"),
      },
      {
        path: "categories",
        name: "settings-categories",
        component: () => import("@/features/settings/views/CategoriesTab.vue"),
      },
      {
        path: "custom-fields",
        name: "settings-custom-fields",
        component: () => import("@/features/settings/views/CustomFieldsTab.vue"),
      },
      {
        path: "design",
        name: "settings-design",
        component: () => import("@/features/settings/views/DesignTab.vue"),
      },
      {
        path: "email",
        name: "settings-email",
        component: () => import("@/features/email/views/EmailTab.vue"),
      },
      {
        path: "accounting",
        name: "settings-accounting",
        component: () => import("@/features/settings/views/AccountingTab.vue"),
      },
      {
        path: "members",
        name: "settings-members",
        component: () => import("@/features/members/views/MembersTab.vue"),
      },
      {
        path: "space",
        name: "settings-space",
        component: () => import("@/features/spaces/views/SpaceTab.vue"),
      },
    ],
  },
  { path: "/:pathMatch(.*)*", redirect: "/" },
];
