<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

const props = withDefaults(defineProps<{ token?: string; bare?: boolean }>(), { token: undefined, bare: false });
const { t } = useI18n();

/** Shown in place of the secret, which only exists right after a token is created. */
const PLACEHOLDER = "<TOKEN>";
const url = `${window.location.origin}/api/mcp`;
const headerValue = computed(() => `Bearer ${props.token ?? PLACEHOLDER}`);
const command = computed(() => `claude mcp add --transport http invoiceapp ${url} --header "Authorization: ${headerValue.value}"`);

const facts = computed(() => [
  { key: "url", label: t("mcp.facts.url"), value: url },
  { key: "transport", label: t("mcp.facts.transport"), value: "Streamable HTTP" },
  { key: "header", label: t("mcp.facts.header"), value: "Authorization" },
  { key: "value", label: t("mcp.facts.value"), value: headerValue.value },
]);

/** Key of the value copied last, for the button's feedback. */
const copied = ref<string | null>(null);
async function copy(key: string, value: string) {
  try {
    await navigator.clipboard.writeText(value);
    copied.value = key;
  } catch {
    // Clipboard blocked (permissions, insecure context): the value stays selectable (`select-all`).
    copied.value = null;
  }
}
</script>

<template>
  <section :class="['space-y-4', { card: !bare }]" data-test="mcp-connect">
    <div class="space-y-1">
      <h2 class="text-lg font-semibold">{{ t("mcp.title") }}</h2>
      <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("mcp.intro") }}</p>
    </div>

    <p v-if="!token" class="text-sm text-gray-600 dark:text-gray-400" data-test="mcp-placeholder-hint">
      {{ t("mcp.placeholder", { placeholder: PLACEHOLDER }) }}
      <RouterLink :to="{ name: 'tokens' }" class="font-medium underline" data-test="mcp-tokens-link">{{ t("mcp.tokensLink") }}</RouterLink>
    </p>

    <dl class="space-y-2">
      <div v-for="fact in facts" :key="fact.key" class="grid gap-1 sm:grid-cols-[10rem_1fr_auto] sm:items-center sm:gap-2">
        <dt class="text-sm font-medium">{{ fact.label }}</dt>
        <dd class="min-w-0">
          <code class="block select-all break-all rounded bg-gray-100 px-2 py-1 font-mono text-sm dark:bg-gray-800" :data-test="`mcp-${fact.key}`">{{ fact.value }}</code>
        </dd>
        <button type="button" class="btn btn-sm justify-self-start" :data-test="`mcp-copy-${fact.key}`" @click="copy(fact.key, fact.value)">
          {{ copied === fact.key ? t("mcp.copied") : t("mcp.copy") }}
        </button>
      </div>
    </dl>
    <p class="text-sm text-gray-600 dark:text-gray-400">{{ t("mcp.roles") }}</p>

    <div class="space-y-2">
      <h3 class="font-semibold">{{ t("mcp.claudeAi.title") }}</h3>
      <ol class="list-decimal space-y-1 pl-5 text-sm">
        <li>{{ t("mcp.claudeAi.open") }}</li>
        <li>{{ t("mcp.claudeAi.url") }}</li>
        <li>{{ t("mcp.claudeAi.header", { header: "Authorization", value: headerValue }) }}</li>
      </ol>
      <p class="text-sm text-amber-700 dark:text-amber-400" data-test="mcp-beta">{{ t("mcp.claudeAi.beta") }}</p>
    </div>

    <div class="space-y-2">
      <h3 class="font-semibold">{{ t("mcp.claudeCode.title") }}</h3>
      <p class="text-sm">{{ t("mcp.claudeCode.intro") }}</p>
      <div class="flex flex-col gap-2 sm:flex-row sm:items-start">
        <code class="block min-w-0 flex-1 select-all break-all rounded bg-gray-100 px-2 py-1 font-mono text-sm dark:bg-gray-800" data-test="mcp-command">{{ command }}</code>
        <button type="button" class="btn btn-sm self-start" data-test="mcp-copy-command" @click="copy('command', command)">
          {{ copied === "command" ? t("mcp.copied") : t("mcp.copy") }}
        </button>
      </div>
    </div>

    <div class="space-y-1">
      <h3 class="font-semibold">{{ t("mcp.generic.title") }}</h3>
      <p class="text-sm">{{ t("mcp.generic.text") }}</p>
    </div>

    <p class="rounded-md bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:bg-amber-950/50 dark:text-amber-300" data-test="mcp-security">{{ t("mcp.security") }}</p>
  </section>
</template>
