<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { FieldErrors } from "@/api/types";
import { reasonKey } from "@/lib/formErrors";
import { formatMoney } from "@/lib/format";
import type { MemberDraft } from "../form";
import type { CatalogItem } from "../types";

/** Ordered members of a catalog group: pick items, set quantities, reorder. */
const members = defineModel<MemberDraft[]>({ required: true });
const props = defineProps<{ items: CatalogItem[]; errors: FieldErrors }>();

const { t, locale } = useI18n();
const picked = ref("");

/** Items not yet in the group (a member may appear once). */
const available = computed(() => props.items.filter((i) => i.active && !members.value.some((m) => m.item.id === i.id)));

function add() {
  const item = props.items.find((i) => i.id === picked.value);
  if (!item) return;
  members.value = [...members.value, { item, quantity: "1" }];
  picked.value = "";
}

function move(index: number, delta: -1 | 1) {
  const next = [...members.value];
  const [m] = next.splice(index, 1);
  next.splice(index + delta, 0, m as MemberDraft);
  members.value = next;
}

const remove = (index: number) => (members.value = members.value.filter((_, i) => i !== index));
const setQuantity = (index: number, quantity: string) =>
  (members.value = members.value.map((m, i) => (i === index ? { ...m, quantity } : m)));
const qtyError = (i: number) => props.errors[`members.${i}.quantity`];
</script>

<template>
  <fieldset class="space-y-3">
    <legend class="text-sm font-medium">{{ t("catalog.groups.members") }}</legend>
    <ol v-if="members.length" class="space-y-2">
      <li
        v-for="(m, i) in members"
        :key="m.item.id"
        class="grid grid-cols-[1fr_auto] items-center gap-2 rounded-md border border-gray-200 p-2 sm:grid-cols-[1fr_8rem_auto] dark:border-gray-700"
        data-test="member-row"
      >
        <div class="text-sm">
          <span class="font-medium">{{ m.item.name }}</span>
          <span class="ml-2 text-xs text-gray-500">{{ formatMoney(m.item.unitPrice, m.item.currency, locale) }} · {{ m.item.vatRate }} %</span>
        </div>
        <div class="order-last col-span-2 sm:order-none sm:col-span-1">
          <label :for="`member-${i}-quantity`" class="sr-only">{{ t("documents.line.quantity") }}</label>
          <input
            :id="`member-${i}-quantity`"
            :value="m.quantity"
            inputmode="decimal"
            class="input"
            :class="{ 'input-error': qtyError(i) }"
            :placeholder="t('documents.line.quantity')"
            @input="setQuantity(i, ($event.target as HTMLInputElement).value)"
          />
          <p v-if="qtyError(i)" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ t(reasonKey(qtyError(i) as string)) }}</p>
        </div>
        <div class="flex gap-1">
          <button type="button" class="btn btn-sm" :disabled="i === 0" :aria-label="t('documents.line.moveUp', { n: i + 1 })" @click="move(i, -1)">↑</button>
          <button type="button" class="btn btn-sm" :disabled="i === members.length - 1" :aria-label="t('documents.line.moveDown', { n: i + 1 })" @click="move(i, 1)">↓</button>
          <button type="button" class="btn btn-sm btn-danger" :aria-label="t('documents.line.remove', { n: i + 1 })" data-test="remove-member" @click="remove(i)">✕</button>
        </div>
      </li>
    </ol>
    <p v-else class="text-sm text-gray-500">{{ t("catalog.groups.noMembers") }}</p>
    <p v-if="errors.members" class="text-xs text-red-600 dark:text-red-400" data-test="members-error">
      {{ t(reasonKey(errors.members)) }}
    </p>

    <div class="flex flex-wrap gap-2">
      <label for="member-pick" class="sr-only">{{ t("catalog.groups.pickItem") }}</label>
      <select id="member-pick" v-model="picked" class="input sm:max-w-sm">
        <option value="">{{ available.length ? t("catalog.groups.pickItem") : t("catalog.groups.noMoreItems") }}</option>
        <option v-for="i in available" :key="i.id" :value="i.id">{{ i.name }} ({{ i.vatRate }} %)</option>
      </select>
      <button type="button" class="btn" :disabled="!picked" data-test="add-member" @click="add">{{ t("common.add") }}</button>
    </div>
  </fieldset>
</template>
