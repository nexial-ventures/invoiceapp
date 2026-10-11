import { describe, expect, it } from "vitest";
import { ApiError } from "@/api/client";
import {
  collectErrors,
  errorDetailOf,
  errorMessageKey,
  fieldErrorsOf,
  nullIfEmpty,
  reasonKey,
  textRule,
} from "./formErrors";

describe("fieldErrorsOf", () => {
  it("maps a 422 validation response to field errors", () => {
    const err = new ApiError(422, "validation", { ico: "duplicate", name: "required" });
    expect(fieldErrorsOf(err)).toEqual({ ico: "duplicate", name: "required" });
  });

  it("returns a copy, not the error's own object", () => {
    const err = new ApiError(422, "validation", { ico: "duplicate" });
    const fields = fieldErrorsOf(err)!;
    fields.ico = "x";
    expect(err.fields.ico).toBe("duplicate");
  });

  it("returns null for non-422 errors and 422 without fields", () => {
    expect(fieldErrorsOf(new ApiError(409, "conflict"))).toBeNull();
    expect(fieldErrorsOf(new ApiError(422, "validation"))).toBeNull();
    expect(fieldErrorsOf(new Error("boom"))).toBeNull();
  });
});

describe("reasonKey", () => {
  it("maps known reason codes and falls back to invalid", () => {
    expect(reasonKey("invalid_ico")).toBe("validation.invalid_ico");
    expect(reasonKey("too_long")).toBe("validation.too_long");
    expect(reasonKey("below_issued")).toBe("validation.below_issued");
    expect(reasonKey("not_set")).toBe("validation.not_set");
    expect(reasonKey("something_new")).toBe("validation.invalid");
  });
});

describe("errorMessageKey", () => {
  it("maps known codes, unreachable statuses and unknown codes", () => {
    expect(errorMessageKey(new ApiError(409, "conflict")).key).toBe("errors.conflict");
    expect(errorMessageKey(new ApiError(502, "ares_unavailable")).key).toBe("errors.aresUnavailable");
    expect(errorMessageKey(new ApiError(0, "network")).key).toBe("errors.unreachable");
    expect(errorMessageKey(new ApiError(409, "document_locked")).key).toBe("errors.documentLocked");
    expect(errorMessageKey(new ApiError(409, "invalid_state")).key).toBe("errors.invalidState");
    // 502 with a known code is that code, not the generic "unreachable".
    expect(errorMessageKey(new ApiError(502, "cnb_unavailable")).key).toBe("errors.cnbUnavailable");
    expect(errorMessageKey(new ApiError(500, "internal"))).toEqual({
      key: "errors.unexpected",
      params: { code: "internal" },
    });
  });
});

describe("helpers", () => {
  it("collectErrors drops passing checks", () => {
    expect(collectErrors({ a: null, b: "required", c: false })).toEqual({ b: "required" });
  });

  it("textRule checks required and length on trimmed input", () => {
    expect(textRule("  ", { required: true })).toBe("required");
    expect(textRule("abc", { max: 2 })).toBe("too_long");
    expect(textRule(" ab ", { required: true, max: 2 })).toBeNull();
  });

  it("nullIfEmpty trims and nulls blanks", () => {
    expect(nullIfEmpty("  ")).toBeNull();
    expect(nullIfEmpty(" x ")).toBe("x");
  });
});

describe("pdf errors", () => {
  it("maps the PDF codes before the generic unreachable statuses", () => {
    expect(errorMessageKey(new ApiError(503, "pdf_unavailable")).key).toBe("errors.pdfUnavailable");
    expect(errorMessageKey(new ApiError(503, "storage_unavailable")).key).toBe("errors.storageUnavailable");
    expect(errorMessageKey(new ApiError(502, "pdf_render_failed", {}, "typst")).key).toBe("errors.pdfRenderFailed");
  });

  it("errorDetailOf returns the server detail only", () => {
    expect(errorDetailOf(new ApiError(502, "pdf_render_failed", {}, "line 3: unknown"))).toBe("line 3: unknown");
    expect(errorDetailOf(new ApiError(503, "pdf_unavailable"))).toBeNull();
    expect(errorDetailOf(new Error("boom"))).toBeNull();
  });
});

describe("auth error messages", () => {
  it.each([
    [401, "invalid_credentials", "errors.invalidCredentials"],
    [403, "forbidden", "errors.forbidden"],
    [403, "csrf", "errors.csrf"],
    [403, "email_unverified", "errors.emailUnverified"],
    [403, "space_creation_disabled", "errors.spaceCreationDisabled"],
    [429, "rate_limited", "errors.rateLimited"],
  ])("%i %s → %s", (status, code, key) => {
    expect(errorMessageKey(new ApiError(status, code))).toEqual({ key });
  });

  it.each(["too_short", "reserved", "taken", "mismatch", "too_high"])("has a reason message for %s", (code) => {
    expect(reasonKey(code)).toBe(`validation.${code}`);
  });
});
