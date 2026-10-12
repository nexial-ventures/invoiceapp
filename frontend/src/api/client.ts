import type { ApiErrorBody, FieldErrors } from "./types";

/**
 * `status` 0 means the request never got a response (network down, CORS, abort).
 * `fields` maps camelCase wire field names to reason codes (422 `validation`).
 * `detail` is a human-readable diagnostic (502 `pdf_render_failed`: the typst error).
 */
export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    public readonly fields: FieldErrors = {},
    public readonly detail: string | null = null,
    /** `Retry-After` of a 429, in seconds. */
    public readonly retryAfter: number | null = null,
  ) {
    super(`API error ${status}: ${code}`);
    this.name = "ApiError";
  }
}

export interface RequestOptions extends Omit<RequestInit, "body"> {
  /**
   * Serialized as JSON; a `FormData` goes out as multipart (the browser sets the boundary), a `Blob` / `File`
   * as the raw body (its own type as `Content-Type`).
   */
  body?: unknown;
  /**
   * A 401 is an expected answer here (the session probe, the login itself), so it must
   * not trigger the "go to login" handler.
   */
  quiet401?: boolean;
}

export interface AuthHandlers {
  /** Any 401 except `quiet401` requests: the session is gone. */
  unauthorized: () => void;
  /** 403 `email_unverified`: the user has to verify the e-mail first. */
  emailUnverified: () => void;
}

// Registered by main.ts with the router. Kept as a hook rather than importing
// the router here to avoid a client → router → views → client import cycle.
let handlers: AuthHandlers = { unauthorized: () => {}, emailUnverified: () => {} };

export function setAuthHandlers(next: Partial<AuthHandlers>): void {
  handlers = { unauthorized: () => {}, emailUnverified: () => {}, ...next };
}

async function errorBody(res: Response): Promise<ApiErrorBody> {
  try {
    const body = (await res.json()) as Partial<ApiErrorBody>;
    if (typeof body.code === "string") {
      const fields = body.fields && typeof body.fields === "object" ? body.fields : undefined;
      const detail = typeof body.detail === "string" ? body.detail : undefined;
      return { code: body.code, fields, detail };
    }
  } catch {
    // Non-JSON error (e.g. proxy HTML page) — fall through to a generic code.
  }
  return { code: `http_${res.status}` };
}

/**
 * Sends the request with the session cookie (same origin only); any non-2xx becomes an `ApiError`.
 * The browser adds `Origin` to mutations itself, which the server's CSRF check needs.
 */
async function send(path: string, options: RequestOptions, accept: string): Promise<Response> {
  const { body, quiet401, headers: extraHeaders, ...init } = options;

  const headers = new Headers(extraHeaders);
  headers.set("Accept", accept);
  const raw = body instanceof FormData || body instanceof Blob;
  if (body !== undefined && !raw) headers.set("Content-Type", "application/json");

  let res: Response;
  try {
    res = await fetch(path, {
      ...init,
      credentials: "same-origin",
      headers,
      body: body === undefined ? undefined : raw ? body : JSON.stringify(body),
    });
  } catch {
    throw new ApiError(0, "network");
  }

  if (!res.ok) {
    const { code, fields, detail } = await errorBody(res);
    if (res.status === 401 && !quiet401) handlers.unauthorized();
    if (res.status === 403 && code === "email_unverified") handlers.emailUnverified();
    const retry = Number(res.headers.get("Retry-After") ?? "");
    throw new ApiError(res.status, code, fields, detail ?? null, Number.isFinite(retry) && retry > 0 ? retry : null);
  }
  return res;
}

export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const res = await send(path, options, "application/json");
  // 202 / 204 carry no body (register, password reset, …); any other empty 2xx is treated the same.
  if (res.status === 202 || res.status === 204) return undefined as T;
  const text = await res.text();
  return (text === "" ? undefined : JSON.parse(text)) as T;
}

export interface BlobResponse {
  blob: Blob;
  /** From `Content-Disposition`; null when the header carries none. */
  filename: string | null;
}

/**
 * File download (a PDF, an ISDOC / ZIP, a CSV), fetched here and handed to the page as a blob so an
 * error stays an `ApiError` with a readable message instead of a broken tab.
 */
export async function requestBlob(path: string, options: RequestOptions = {}): Promise<BlobResponse> {
  const res = await send(path, options, "application/pdf, application/zip, application/xml, text/csv, application/octet-stream, application/json");
  return { blob: await res.blob(), filename: filenameFromDisposition(res.headers.get("Content-Disposition")) };
}

/** `filename*=UTF-8''…` (RFC 5987) wins over `filename="…"`. */
export function filenameFromDisposition(header: string | null): string | null {
  if (!header) return null;
  const extended = /filename\*\s*=\s*(?:UTF-8|utf-8)''([^;]+)/.exec(header);
  if (extended?.[1]) {
    try {
      return decodeURIComponent(extended[1].trim());
    } catch {
      // Malformed escape — fall back to the plain parameter.
    }
  }
  const plain = /filename\s*=\s*(?:"([^"]*)"|([^;]+))/.exec(header);
  const name = (plain?.[1] ?? plain?.[2])?.trim();
  return name || null;
}
