import { describe, expect, it } from "vitest";
import { ApiError } from "@/api/client";
import { i18n } from "@/i18n";
import { entry, isdocFile } from "./testData";
import { defaultSelection } from "@/features/imports/selection";
import { totalBytes } from "@/features/imports/upload";
import {
  ENTRY_ERRORS,
  entryErrorKey,
  exportErrorKey,
  partitionFiles,
  uploadErrorKey,
  warningKey,
  WARNINGS,
} from "./upload";

describe("isdoc upload helpers", () => {
  it("keeps .isdoc / .isdocx / .zip (any case) and names the rest", () => {
    const { files, ignored } = partitionFiles([isdocFile("a.isdoc"), isdocFile("B.ISDOCX"), isdocFile("c.zip"), isdocFile("d.pdf")]);
    expect(files.map((f) => f.name)).toEqual(["a.isdoc", "B.ISDOCX", "c.zip"]);
    expect(ignored).toEqual(["d.pdf"]);
    expect(totalBytes([isdocFile("a.isdoc", 10), isdocFile("b.zip", 5)])).toBe(15);
  });

  it("preselects the ok keys; duplicates and errors stay out", () => {
    const entries = [
      entry({ key: "a" }),
      entry({ key: "a#2", status: "duplicate" }),
      entry({ key: "b", status: "error", error: "foreign" }),
      entry({ key: "c" }),
    ];
    expect(defaultSelection(entries)).toEqual(["a", "c"]);
  });

  it("maps known entry codes and falls back for unknown ones; every key exists", () => {
    expect(entryErrorKey("foreign")).toBe("isdoc.entryError.foreign");
    expect(entryErrorKey("quota_exceeded")).toBe("isdoc.entryError.quota_exceeded");
    expect(entryErrorKey("new_thing")).toBe("isdoc.entryError.unknown");
    expect(warningKey("pdf_skipped")).toBe("isdoc.warning.pdf_skipped");
    expect(warningKey("other")).toBe("isdoc.warning.unknown");
    for (const locale of ["cs", "en"] as const) {
      for (const code of [...ENTRY_ERRORS, "x"]) expect(i18n.global.te(entryErrorKey(code), locale)).toBe(true);
      for (const code of [...WARNINGS, "x"]) expect(i18n.global.te(warningKey(code), locale)).toBe(true);
    }
  });

  it("maps the upload limits and the export cap", () => {
    expect(uploadErrorKey(new ApiError(413, "too_large"))).toBe("isdoc.upload.tooLarge");
    expect(uploadErrorKey(new ApiError(422, "validation", { files: "too_many" }))).toBe("isdoc.upload.tooMany");
    expect(uploadErrorKey(new ApiError(422, "validation", { files: "too_large" }))).toBe("isdoc.upload.unpackedTooLarge");
    expect(uploadErrorKey(new ApiError(422, "validation", { categoryId: "inactive" }))).toBe("isdoc.upload.category");
    expect(uploadErrorKey(new ApiError(500, "internal"))).toBeNull();
    expect(uploadErrorKey(new Error("x"))).toBeNull();
    expect(exportErrorKey(new ApiError(422, "validation", { filter: "too_many" }))).toBe("isdoc.export.tooMany");
    expect(exportErrorKey(new ApiError(422, "validation", { from: "invalid" }))).toBeNull();
  });
});
