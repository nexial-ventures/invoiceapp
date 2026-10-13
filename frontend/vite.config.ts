import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
import VueI18nPlugin from "@intlify/unplugin-vue-i18n/vite";
import { fileURLToPath, URL } from "node:url";

// In production the SPA is embedded in the Rust binary (rust-embed) and served
// from the same origin, so the proxy only matters for `npm run dev`.
//
// Multi-tenancy in dev: the backend picks the app (base hub vs. a space) from the
// `Host` header, so the proxy must forward the browser's host unchanged
// (`changeOrigin: false`). Run the backend with `INVOICEAPP__PUBLIC_URL=http://localhost:5173`
// (the Vite origin: CSRF compares `Origin` with it, and e-mail / "Moje spaces" links
// point there), then open http://localhost:5173 for the base host and
// http://<slug>.localhost:5173 for a space. Browsers resolve `*.localhost` to
// loopback without /etc/hosts, and Vite allows `.localhost` hosts by default.
// Cookies are host-only, so every host has its own sign-in, as in production.
const BACKEND = "http://localhost:3000";

export default defineConfig({
  plugins: [
    vue(),
    tailwindcss(),
    // Locale JSON is precompiled at build time and vue-i18n is aliased to its runtime-only build:
    // no message compiler ships (smaller bundle, no `new Function`, so the CSP needs no 'unsafe-eval').
    VueI18nPlugin({ include: [fileURLToPath(new URL("./src/locales/**/*.json", import.meta.url))] }),
  ],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    proxy: {
      "/api": { target: BACKEND, changeOrigin: false },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    setupFiles: ["src/test-setup.ts"],
  },
});
