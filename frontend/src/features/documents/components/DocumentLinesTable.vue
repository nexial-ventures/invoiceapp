<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { formatMoney, formatNumber } from "@/lib/format";
import type { ComputedLine } from "../types";

const props = defineProps<{ lines: ComputedLine[]; currency: string }>();
const { t, locale } = useI18n();

/** position → positions of the subtotals that include it, for the grouping marker. */
const memberOf = computed(() => {
  const map = new Map<number, number[]>();
  for (const line of props.lines) {
    if (line.kind !== "subtotal") continue;
    for (const ref of line.refs) map.set(ref, [...(map.get(ref) ?? []), line.position]);
  }
  return map;
});

const money = (v: string) => formatMoney(v, props.currency, locale.value);
const num = (v: string) => formatNumber(v, locale.value);
</script>

<template>
  <section class="card overflow-x-auto !p-0">
    <table class="table">
      <thead>
        <tr>
          <th class="w-0">#</th>
          <th>{{ t("documents.line.description") }}</th>
          <th class="hidden text-right sm:table-cell">{{ t("documents.line.quantity") }}</th>
          <th class="hidden text-right sm:table-cell">{{ t("documents.line.unitPrice") }}</th>
          <th class="hidden text-right md:table-cell">{{ t("documents.line.discountPct") }}</th>
          <th class="hidden text-right md:table-cell">{{ t("documents.line.vatRate") }}</th>
          <th class="text-right">{{ t("documents.totals.base") }}</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="line in lines"
          :key="line.position"
          :class="{ 'bg-gray-50 font-medium dark:bg-gray-800/40': line.kind === 'subtotal' }"
          :data-test="`detail-line-${line.position}`"
        >
          <td class="font-mono text-xs whitespace-nowrap text-gray-500">
            {{ line.position }}
            <span v-if="memberOf.get(line.position)" class="ml-1" :title="t('documents.line.memberOf', { positions: memberOf.get(line.position)!.join(', ') })">
              ↳{{ memberOf.get(line.position)!.join(",") }}
            </span>
          </td>
          <td>
            <span class="whitespace-pre-line" :class="{ 'text-gray-600 italic dark:text-gray-400': line.kind === 'text' }">{{ line.description }}</span>
            <div v-if="line.kind === 'subtotal'" class="text-xs font-normal text-gray-500">
              Σ {{ line.refs.join(", ") }}
              <span v-if="line.collapse" class="badge ml-1">{{ t("documents.line.collapsed") }}</span>
            </div>
            <ul v-if="line.kind === 'advance'" class="text-xs text-gray-500" data-test="advance-recap">
              <li v-for="r in line.recap" :key="r.vatRate">
                {{ t("documents.line.advanceRecap", { rate: num(r.vatRate), base: money(r.base), vat: money(r.vat) }) }}
              </li>
            </ul>
            <div v-if="line.kind === 'item'" class="text-xs text-gray-500 sm:hidden">
              {{ num(line.quantity) }} {{ line.unit }} × {{ money(line.unitPrice) }} · {{ num(line.vatRate) }} %
            </div>
          </td>
          <template v-if="line.kind === 'item'">
            <td class="hidden text-right whitespace-nowrap sm:table-cell">{{ num(line.quantity) }} {{ line.unit }}</td>
            <td class="hidden text-right whitespace-nowrap tabular-nums sm:table-cell">{{ money(line.unitPrice) }}</td>
            <td class="hidden text-right md:table-cell">{{ Number(line.discountPct) ? `${num(line.discountPct)} %` : "" }}</td>
            <td class="hidden text-right md:table-cell">{{ num(line.vatRate) }} %</td>
          </template>
          <template v-else>
            <td class="hidden sm:table-cell" />
            <td class="hidden sm:table-cell" />
            <td class="hidden md:table-cell" />
            <td class="hidden text-right md:table-cell">{{ line.kind === "subtotal" && line.vatRate ? `${num(line.vatRate)} %` : "" }}</td>
          </template>
          <td class="text-right whitespace-nowrap tabular-nums">{{ "base" in line ? money(line.base) : "" }}</td>
        </tr>
      </tbody>
    </table>
  </section>
</template>
