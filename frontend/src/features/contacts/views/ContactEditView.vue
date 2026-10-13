<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { useErrorText } from "@/composables/useAction";
import ContactForm from "../components/ContactForm.vue";
import ContactNumberCounters from "../components/ContactNumberCounters.vue";
import { useContactsStore } from "../store";
import type { Contact } from "../types";
import { useSessionStore } from "@/stores/session";

const { t } = useI18n();
const session = useSessionStore();
const route = useRoute();
const router = useRouter();
const store = useContactsStore();

const id = computed(() => (typeof route.params.id === "string" ? route.params.id : undefined));
const contact = ref<Contact | undefined>();
const loading = ref(false);
const error = ref<string | null>(null);

const errorText = useErrorText();
const setError = (err: unknown) => (error.value = errorText(err));

watch(
  id,
  async (value) => {
    contact.value = undefined;
    error.value = null;
    if (!value) return;
    loading.value = true;
    try {
      contact.value = await store.get(value);
    } catch (err) {
      setError(err);
    } finally {
      loading.value = false;
    }
  },
  { immediate: true },
);

async function onSaved() {
  await router.push({ name: "contacts" });
}

async function onDelete() {
  if (!contact.value || !window.confirm(t("contacts.confirmDelete", { name: contact.value.name }))) return;
  try {
    await store.remove(contact.value.id);
    await router.push({ name: "contacts" });
  } catch (err) {
    setError(err);
  }
}
</script>

<template>
  <section class="space-y-4">
    <div class="flex items-center gap-3">
      <RouterLink :to="{ name: 'contacts' }" class="text-sm text-blue-600 hover:underline dark:text-blue-400">
        ← {{ t("contacts.backToList") }}
      </RouterLink>
    </div>
    <h1 class="text-2xl font-semibold">
      {{ id ? t("contacts.editTitle") : t("contacts.newTitle") }}
    </h1>

    <p v-if="error" role="alert" class="alert-error">{{ error }}</p>
    <p v-if="loading" class="text-sm text-gray-500">{{ t("common.loading") }}</p>

    <ContactForm v-if="!id || contact" :key="contact?.id ?? 'new'" :contact="contact" :readonly="!session.can('write')" @saved="onSaved">
      <template #actions>
        <RouterLink :to="{ name: 'contacts' }" class="btn">{{ t("common.cancel") }}</RouterLink>
        <button v-if="contact && session.can('write')" type="button" class="btn btn-danger ml-auto" @click="onDelete">
          {{ t("common.delete") }}
        </button>
      </template>
    </ContactForm>

    <ContactNumberCounters v-if="contact" :key="contact.id" :contact-id="contact.id" :has-code="!!contact.numberCode" />
  </section>
</template>
