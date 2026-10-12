import { createApp } from "vue";
import { createPinia } from "pinia";
import { createWebHistory } from "vue-router";
import App from "./App.vue";
import { loadContext, type Boot } from "./boot";
import { createAppRouter } from "./router";
import { i18n, loadInitialLocale } from "./i18n";
import { setAuthHandlers } from "./api/client";
import { useSessionStore } from "./stores/session";
import "./style.css";

async function main() {
  const pinia = createPinia();
  const [context, messages] = await Promise.all([loadContext(), loadInitialLocale()]);
  const boot: Boot = messages ? context : { status: "error" };
  const session = useSessionStore(pinia);
  session.applyBoot(boot);

  const router = createAppRouter(createWebHistory(), boot);

  setAuthHandlers({
    unauthorized: () => {
      session.clear();
      const current = router.currentRoute.value;
      if (current.meta.public) return;
      void router.push({ name: "login", query: { return: current.fullPath } });
    },
    emailUnverified: () => {
      if (router.hasRoute("verify")) void router.push({ name: "verify" });
    },
  });

  document.documentElement.lang = i18n.global.locale.value;
  createApp(App).use(pinia).use(router).use(i18n).mount("#app");
}

void main();
