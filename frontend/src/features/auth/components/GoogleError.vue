<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { googleErrorKey } from "../google";

/** The `?google_error=` the Google flow redirected back with, as a readable message. */
const { t } = useI18n();
const route = useRoute();
const key = computed(() => googleErrorKey(route.query.google_error));
</script>

<template>
  <p v-if="key" role="alert" class="alert-error" data-test="google-error">{{ t(key) }}</p>
</template>
