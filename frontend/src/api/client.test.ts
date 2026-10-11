import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, filenameFromDisposition, request, requestBlob, setAuthHandlers } from "./client";
import { mockFetch, mockFetchRoutes, pdfReply, reply, sentHeaders } from "@/test-utils";

describe("api client", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    setAuthHandlers({});
  });

  it("relies on the same-origin session cookie, never an Authorization header", async () => {
    localStorage.setItem("invoice.token", "legacy");
    const fetch = mockFetch(200, { status: "ok", version: "1.0.0" });

    const res = await request<{ version: string }>("/api/health");

    expect(res.version).toBe("1.0.0");
    expect(fetch.mock.calls[0]?.[1]?.credentials).toBe("same-origin");
    expect(sentHeaders(fetch).has("Authorization")).toBe(false);
  });

  it("returns undefined on 204", async () => {
    mockFetch(204);
    await expect(request("/api/x")).resolves.toBeUndefined();
  });

  it.each([202, 200, 201])("returns undefined for an empty %i body (e.g. 202 from register)", async (status) => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response("", { status, headers: { "Content-Length": "0" } })));
    await expect(request("/api/auth/register", { method: "POST", body: {} })).resolves.toBeUndefined();
  });

  it("sends a Blob as the raw body", async () => {
    const fetch = mockFetch(200, {});
    const file = new Blob(["<svg/>"], { type: "image/svg+xml" });
    await request("/api/x", { method: "PUT", body: file });
    expect(fetch.mock.calls[0]?.[1]?.body).toBe(file);
    expect(sentHeaders(fetch).has("Content-Type")).toBe(false);
  });

  it("serializes a JSON body", async () => {
    const fetch = mockFetch(200, {});
    await request("/api/x", { method: "POST", body: { a: 1 } });
    expect(fetch.mock.calls[0]?.[1]?.body).toBe('{"a":1}');
    expect(sentHeaders(fetch).get("Content-Type")).toBe("application/json");
  });

  it("parses the error code into ApiError", async () => {
    mockFetch(404, { code: "not_found" });
    await expect(request("/api/x")).rejects.toEqual(new ApiError(404, "not_found"));
  });

  it("carries 422 validation fields", async () => {
    mockFetch(422, { code: "validation", fields: { ico: "invalid_ico", name: "required" } });
    await expect(request("/api/x")).rejects.toMatchObject({
      status: 422,
      code: "validation",
      fields: { ico: "invalid_ico", name: "required" },
    });
  });

  it("defaults fields to an empty object", async () => {
    mockFetch(409, { code: "conflict" });
    await expect(request("/api/x")).rejects.toMatchObject({ fields: {} });
  });

  it("falls back to http_<status> for non-JSON errors", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response("<html>", { status: 502 })));
    await expect(request("/api/x")).rejects.toMatchObject({ status: 502, code: "http_502" });
  });

  it("maps a network failure to status 0", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => Promise.reject(new TypeError("Failed to fetch"))));
    await expect(request("/api/x")).rejects.toMatchObject({ status: 0, code: "network" });
  });

  it("on 401 calls the unauthorized handler", async () => {
    const unauthorized = vi.fn();
    const emailUnverified = vi.fn();
    setAuthHandlers({ unauthorized, emailUnverified });
    mockFetch(401, { code: "unauthorized" });

    await expect(request("/api/x")).rejects.toMatchObject({ status: 401, code: "unauthorized" });
    expect(unauthorized).toHaveBeenCalledOnce();
    expect(emailUnverified).not.toHaveBeenCalled();
  });

  it("on 401 of a quiet401 request (session probe, login) leaves the handler alone", async () => {
    const unauthorized = vi.fn();
    setAuthHandlers({ unauthorized });
    mockFetch(401, { code: "invalid_credentials" });

    await expect(request("/api/auth/login", { method: "POST", quiet401: true })).rejects.toMatchObject({ code: "invalid_credentials" });
    expect(unauthorized).not.toHaveBeenCalled();
  });

  it("on 403 email_unverified calls the verify handler", async () => {
    const unauthorized = vi.fn();
    const emailUnverified = vi.fn();
    setAuthHandlers({ unauthorized, emailUnverified });
    mockFetch(403, { code: "email_unverified" });

    await expect(request("/api/spaces", { method: "POST", body: {} })).rejects.toMatchObject({ status: 403, code: "email_unverified" });
    expect(emailUnverified).toHaveBeenCalledOnce();
    expect(unauthorized).not.toHaveBeenCalled();
  });

  it.each(["forbidden", "csrf"])("on 403 %s only throws (the view shows the message)", async (code) => {
    const unauthorized = vi.fn();
    const emailUnverified = vi.fn();
    setAuthHandlers({ unauthorized, emailUnverified });
    mockFetch(403, { code });

    await expect(request("/api/x", { method: "PUT", body: {} })).rejects.toEqual(new ApiError(403, code));
    expect(unauthorized).not.toHaveBeenCalled();
    expect(emailUnverified).not.toHaveBeenCalled();
  });

  describe("requestBlob", () => {
    it("returns the blob with the filename", async () => {
      const fetch = mockFetchRoutes({ "GET /api/documents/d1/pdf": pdfReply("20260001.pdf", "attachment") });

      const res = await requestBlob("/api/documents/d1/pdf?download=1");

      expect(fetch.mock.calls[0]?.[0]).toBe("/api/documents/d1/pdf?download=1");
      expect(fetch.mock.calls[0]?.[1]?.credentials).toBe("same-origin");
      expect(sentHeaders(fetch).get("Accept")).toContain("application/pdf");
      expect(res.blob.type).toBe("application/pdf");
      expect(await res.blob.text()).toBe("%PDF-1.7");
      expect(res.filename).toBe("20260001.pdf");
    });

    it("has a null filename without Content-Disposition", async () => {
      mockFetchRoutes({ "GET /api/pdf/preview": pdfReply() });
      expect((await requestBlob("/api/pdf/preview?locale=cs")).filename).toBeNull();
    });

    it("turns a JSON error into ApiError with its detail", async () => {
      mockFetchRoutes({ "GET /api/pdf/preview": reply(502, { code: "pdf_render_failed", detail: "error: unknown variable: foo" }) });
      await expect(requestBlob("/api/pdf/preview")).rejects.toEqual(
        new ApiError(502, "pdf_render_failed", {}, "error: unknown variable: foo"),
      );
    });

    it("leaves detail null when the body has none", async () => {
      mockFetchRoutes({ "GET /api/pdf/preview": reply(503, { code: "pdf_unavailable" }) });
      await expect(requestBlob("/api/pdf/preview")).rejects.toMatchObject({ status: 503, code: "pdf_unavailable", detail: null });
    });
  });

  it("filenameFromDisposition prefers filename* and tolerates missing parts", () => {
    expect(filenameFromDisposition(null)).toBeNull();
    expect(filenameFromDisposition("inline")).toBeNull();
    expect(filenameFromDisposition('attachment; filename="a b.pdf"')).toBe("a b.pdf");
    expect(filenameFromDisposition("attachment; filename=plain.pdf")).toBe("plain.pdf");
    expect(filenameFromDisposition("attachment; filename=\"x.pdf\"; filename*=UTF-8''%C5%BE.pdf")).toBe("ž.pdf");
  });
});
