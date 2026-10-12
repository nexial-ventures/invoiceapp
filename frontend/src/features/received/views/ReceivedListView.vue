<script setup lang="ts">
import { computed, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { signed } from "@/lib/money";
import { useAction } from "@/composables/useAction";
import AccountantExportButton from "@/features/csvExport/components/AccountantExportButton.vue";
import CsvExportButton from "@/features/csvExport/components/CsvExportButton.vue";
import DocTypeTabs from "@/features/documents/components/DocTypeTabs.vue";
import InvoiceFilters from "@/features/documents/components/InvoiceFilters.vue";
import ListDocMeta from "@/features/documents/components/ListDocMeta.vue";
import ListPager from "@/features/documents/components/ListPager.vue";
import StatusBadges from "@/features/documents/components/StatusBadges.vue";
import { formatDate, formatMoney } from "@/lib/format";
import { listDocTypeOf } from "@/features/documents/routes";
import { useReceivedListStore, type InvoiceFilters as Filters } from "@/features/documents/store";
import { useCategoriesStore } from "@/features/settings/stores";
import { useSessionStore } from "@/stores/session";

const { t, locale } = useI18n();
const session = useSessionStore();
const route = useRoute();
const router = useRouter();
const store = useReceivedListStore();
const categoriesStore = useCategoriesStore();
const { error, run } = useAction();

const docType = computed(() => listDocTypeOf(route.query.type));
watch(docType, (type) => void run(() => store.setDocType(type)), { immediate: true });
// Category names only decorate rows and feed a filter: the list works without them.
categoriesStore.ensure().catch(() => {});
const categories = computed(() => categoriesStore.items.filter((c) => c.kind === "expense"));

const onFilters = (f: Filters) => void run(() => store.applyFilters(f));
const filtered = computed(() => Object.values(store.filters).some(Boolean));

function open(id: string) {
  void router.push({ name: "received-detail", params: { id } });
}
</script>

<template>
  <section class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <h1 class="text-2xl font-semibold">{{ t(`received.list.title.${docType}`) }}</h1>
      <div class="flex flex-wrap gap-2">
        <CsvExportButton :query="store.query" />
        <AccountantExportButton />
        <template v-if="session.can('write')">
          <RouterLink :to="{ name: 'isdoc-import', query: { from: 'received' } }" class="btn" data-test="import-isdoc">{{ t("isdoc.import.action") }}</RouterLink>
          <RouterLink :to="{ name: 'csv-import', query: { from: 'received' } }" class="btn" data-test="import-csv">{{ t("csvImport.import.action") }}</RouterLink>
          <RouterLink :to="{ name: 'received-new', query: { docType } }" class="btn btn-primary" data-test="new-document">{{ t(`received.newTitle.${docType}`) }}</RouterLink>
        </template>
      </div>
    </div>

    <DocTypeTabs :current="docType" direction="received" label-prefix="documents.list.tab" />

    <InvoiceFilters :filters="store.filters" :categories="categories" :search-placeholder="t('received.list.searchPlaceholder')" @change="onFilters" />

    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>

    <div class="card overflow-x-auto !p-0">
      <table class="table">
        <thead>
          <tr>
            <th>{{ t("documents.fields.number") }}</th>
            <th>{{ t("documents.fields.supplier") }}</th>
            <th class="hidden sm:table-cell">{{ t("received.fields.supplierNumber") }}</th>
            <th class="hidden md:table-cell">{{ t("documents.fields.dueDate") }}</th>
            <th class="text-right">{{ t("documents.totals.payable") }}</th>
            <th class="hidden sm:table-cell">{{ t("documents.list.status") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="d in store.items"
            :key="d.id"
            class="cursor-pointer hover:bg-gray-50 dark:hover:bg-gray-800/60"
            data-test="received-row"
            @click="open(d.id)"
          >
            <td class="whitespace-nowrap">
              <RouterLink :to="{ name: 'received-detail', params: { id: d.id } }" class="font-mono font-medium hover:underline" @click.stop>
                {{ d.number }}
              </RouterLink>
              <div class="text-xs text-gray-500">{{ formatDate(d.issueDate, locale) }}</div>
            </td>
            <td>
              {{ d.customerName ?? "—" }}
              <ListDocMeta :doc="d" :categories="categories" />
              <div class="mt-1 sm:hidden"><StatusBadges :status="d.status" :payment-state="d.paymentState" :overdue="d.overdue" :sign="d.sign" :doc-type="d.docType" direction="received" /></div>
            </td>
            <td class="hidden font-mono whitespace-nowrap sm:table-cell">{{ d.supplierNumber }}</td>
            <td class="hidden whitespace-nowrap md:table-cell" :class="{ 'text-red-600 dark:text-red-400': d.overdue }">
              {{ formatDate(d.dueDate, locale) }}
            </td>
            <td class="whitespace-nowrap text-right tabular-nums">{{ formatMoney(signed(d.payable, d.sign), d.currency, locale) }}</td>
            <td class="hidden sm:table-cell">
              <StatusBadges :status="d.status" :payment-state="d.paymentState" :overdue="d.overdue" :sign="d.sign" :doc-type="d.docType" direction="received" />
            </td>
          </tr>
          <tr v-if="!store.loading && store.items.length === 0">
            <td colspan="6" class="py-8 text-center text-gray-500">
              {{ filtered ? t("documents.list.noResults") : t("received.list.empty") }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <ListPager :offset="store.offset" :count="store.items.length" :total="store.total" @go="run(() => store.goTo($event))" />
  </section>
</template>
