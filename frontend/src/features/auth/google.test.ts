import { describe, expect, it } from "vitest";
import { i18n } from "@/i18n";
import { GOOGLE_ERRORS, googleErrorKey, googleStartUrl } from "./google";

describe("googleStartUrl", () => {
  it("encodes the purpose, the return path and the invitation token", () => {
    expect(googleStartUrl("login", { returnTo: "/invoices?status=paid&page=2" })).toBe(
      "/api/auth/google/start?purpose=login&return=%2Finvoices%3Fstatus%3Dpaid%26page%3D2",
    );
    expect(googleStartUrl("invite", { invite: "a+b/c=" })).toBe("/api/auth/google/start?purpose=invite&invite=a%2Bb%2Fc%3D");
  });
});

describe("googleErrorKey", () => {
  it("has a cs and en message for every contract code", () => {
    for (const code of GOOGLE_ERRORS) {
      const key = googleErrorKey(code)!;
      expect(key).toBe(`google.errors.${code}`);
      for (const locale of ["cs", "en"] as const) expect(i18n.global.te(key, locale)).toBe(true);
    }
  });

  it("falls back to a generic message and ignores a missing value", () => {
    expect(googleErrorKey("weird")).toBe("google.errors.unknown");
    expect(googleErrorKey(undefined)).toBeNull();
    expect(googleErrorKey("")).toBeNull();
    expect(googleErrorKey(["invalid"])).toBeNull();
  });
});
