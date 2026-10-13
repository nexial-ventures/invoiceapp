import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { i18n } from "@/i18n";
import { mountView } from "@/testMount";
import McpConnectCard from "./McpConnectCard.vue";

const MCP_URL = `${window.location.origin}/api/mcp`;

describe("McpConnectCard", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });
  afterEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  const text = (w: Pick<VueWrapper, "find">, test: string) => w.find(`[data-test="${test}"]`).text();

  it("shows a placeholder token and links to the API tokens page", async () => {
    const { w } = await mountView(McpConnectCard, { path: "/settings/space", role: "admin" });
    expect(text(w, "mcp-url")).toBe(MCP_URL);
    expect(text(w, "mcp-transport")).toBe("Streamable HTTP");
    expect(text(w, "mcp-header")).toBe("Authorization");
    expect(text(w, "mcp-value")).toBe("Bearer <TOKEN>");
    expect(text(w, "mcp-command")).toBe(`claude mcp add --transport http invoiceapp ${MCP_URL} --header "Authorization: Bearer <TOKEN>"`);
    expect(w.find('[data-test="mcp-placeholder-hint"]').exists()).toBe(true);
    expect(w.find('[data-test="mcp-tokens-link"]').attributes("href")).toBe("/tokens");
    expect(w.find('[data-test="mcp-beta"]').exists()).toBe(true);
    expect(w.find('[data-test="mcp-security"]').exists()).toBe(true);
  });

  it("fills in a given token and hides the placeholder hint", () => {
    const w = mount(McpConnectCard, { props: { token: "inv_abcd_SECRET" }, global: { plugins: [i18n] } });
    expect(text(w, "mcp-value")).toBe("Bearer inv_abcd_SECRET");
    expect(text(w, "mcp-command")).toContain('--header "Authorization: Bearer inv_abcd_SECRET"');
    expect(w.text()).not.toContain("<TOKEN>");
    expect(w.find('[data-test="mcp-placeholder-hint"]').exists()).toBe(false);
  });

  it("copies a value and confirms on its button", async () => {
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const w = mount(McpConnectCard, { props: { token: "inv_x" }, global: { plugins: [i18n] } });
    await w.find('[data-test="mcp-copy-url"]').trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith(MCP_URL);
    expect(text(w, "mcp-copy-url")).toBe("Copied");
    await w.find('[data-test="mcp-copy-command"]').trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenLastCalledWith(`claude mcp add --transport http invoiceapp ${MCP_URL} --header "Authorization: Bearer inv_x"`);
    expect(text(w, "mcp-copy-url")).toBe("Copy");
    expect(text(w, "mcp-copy-command")).toBe("Copied");
  });

  it("keeps the value visible when the clipboard is blocked", async () => {
    Object.defineProperty(navigator, "clipboard", { value: { writeText: vi.fn(async () => Promise.reject(new Error("denied"))) }, configurable: true });
    const w = mount(McpConnectCard, { props: { token: "inv_x" }, global: { plugins: [i18n] } });
    await w.find('[data-test="mcp-copy-value"]').trigger("click");
    await flushPromises();
    expect(text(w, "mcp-copy-value")).toBe("Copy");
    expect(w.find('[data-test="mcp-value"]').classes()).toContain("select-all");
  });
});
