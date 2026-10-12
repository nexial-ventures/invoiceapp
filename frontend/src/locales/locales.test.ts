import { describe, expect, it } from "vitest";
import { i18n } from "@/i18n";
import type { MessageTree } from "@/i18n/namespaces";
import cs from "@/i18n/messages/cs";
import en from "@/i18n/messages/en";

/** Locale JSON is precompiled: a message is an AST root `{ type: 0, body }`, a namespace a plain object. */
const isMessage = (v: unknown) => typeof v === "object" && v !== null && (v as { type?: unknown }).type === 0 && "body" in v;

function leafKeys(tree: MessageTree, prefix = ""): string[] {
  return Object.entries(tree).flatMap(([k, v]) => {
    const path = prefix ? `${prefix}.${k}` : k;
    return isMessage(v) ? [path] : leafKeys(v as MessageTree, path);
  });
}

describe("locales", () => {
  it("each locale directory has the same namespace files", () => {
    expect(Object.keys(cs).length).toBeGreaterThan(0);
    expect(Object.keys(cs).sort()).toEqual(Object.keys(en).sort());
  });

  it("cs and en have identical key sets", () => {
    expect(leafKeys(cs).length).toBeGreaterThan(100);
    expect(leafKeys(cs).sort()).toEqual(leafKeys(en).sort());
  });

  it("has no empty strings", () => {
    for (const [locale, tree] of [["cs", cs], ["en", en]] as const) {
      const empty = leafKeys(tree).filter((k) => i18n.global.t(k, {}, { locale }) === "");
      expect(empty).toEqual([]);
    }
  });
});
