import { vi } from "vitest";
import type { Me, MfaStatus } from "@/features/auth/types";
import type { AppContext, Role } from "@/features/spaces/types";
import { useSessionStore } from "@/stores/session";

/** Stubs global fetch with a single canned response; returns the mock for assertions. */
export function mockFetch(status: number, body?: unknown) {
  const fn = vi.fn<typeof fetch>(async () =>
    new Response(body === undefined ? null : JSON.stringify(body), {
      status,
      headers: { "Content-Type": "application/json" },
    }),
  );
  vi.stubGlobal("fetch", fn);
  return fn;
}

/** Headers sent on the n-th fetch call. */
export function sentHeaders(fn: ReturnType<typeof mockFetch>, call = 0): Headers {
  return new Headers(fn.mock.calls[call]?.[1]?.headers);
}

/** URL, method and parsed JSON body of the n-th fetch call. */
export function sentRequest(fn: ReturnType<typeof mockFetch>, call = 0) {
  const [url, init] = fn.mock.calls[call] ?? [];
  return {
    url,
    method: init?.method ?? "GET",
    body: typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined,
  };
}

/** Stubs fetch with a queue of JSON bodies, one per call; `undefined` answers 204. */
export function mockFetchSequence(...bodies: unknown[]) {
  const fn = vi.fn<typeof fetch>(async () => {
    const body = bodies.shift();
    return body === undefined
      ? new Response(null, { status: 204 })
      : new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } });
  });
  vi.stubGlobal("fetch", fn);
  return fn;
}

type RouteReply = unknown | ((body: unknown) => unknown);

/**
 * Stubs fetch by `"METHOD /path"` (query string ignored). A reply is a JSON body
 * (200), `{ status, body }` via `reply()`, a ready `Response` (binary, e.g. `pdfReply()`),
 * or a function of the parsed request body.
 * Unknown routes answer 404 so a missing stub fails loudly.
 */
export function mockFetchRoutes(routes: Record<string, RouteReply>) {
  const fn = vi.fn<typeof fetch>(async (input, init) => {
    const path = String(input).split("?")[0];
    const key = `${init?.method ?? "GET"} ${path}`;
    if (!(key in routes)) return new Response(JSON.stringify({ code: "not_found" }), { status: 404 });
    const route = routes[key];
    const body = typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined;
    const value = typeof route === "function" ? (route as (b: unknown) => unknown)(body) : route;
    if (value instanceof Response) return value;
    const { status, body: payload } = value instanceof Reply ? value : { status: 200, body: value };
    return status === 204
      ? new Response(null, { status })
      : new Response(JSON.stringify(payload), { status, headers: { "Content-Type": "application/json" } });
  });
  vi.stubGlobal("fetch", fn);
  return fn;
}

class Reply {
  constructor(
    public readonly status: number,
    public readonly body?: unknown,
  ) {}
}

/** A non-200 reply for `mockFetchRoutes`. */
export const reply = (status: number, body?: unknown) => new Reply(status, body);

/** `"METHOD url"` of every fetch call, in order. */
export const calls = (fn: ReturnType<typeof vi.fn<typeof fetch>>) =>
  fn.mock.calls.map(([url, init]) => `${init?.method ?? "GET"} ${String(url)}`);

/** A `200 application/pdf` response; pass a filename to send `Content-Disposition`. */
export const pdfReply = (filename?: string, disposition = "inline") =>
  new Response("%PDF-1.7", {
    status: 200,
    headers: {
      "Content-Type": "application/pdf",
      ...(filename ? { "Content-Disposition": `${disposition}; filename="${filename}"` } : {}),
    },
  });

export const BASE_CONTEXT: AppContext = { kind: "base", space: null, registration: true, google: false, baseUrl: "http://localhost:3000" };
export const SPACE_CONTEXT: AppContext = {
  kind: "space",
  space: { slug: "firma", name: "Firma s.r.o." },
  registration: false,
  google: false,
  baseUrl: "http://localhost:3000",
};

/** `GET /api/account/mfa` of a user without TOTP (the account page loads it). */
export const MFA_OFF: MfaStatus = { enabled: false, recoveryCodesLeft: 0, requiredBy: [] };

/** `GET /api/auth/me` body; `role` null = a base-host session. */
export function meFixture(role: Role | null = "owner", emailVerified = true, mfaEnabled = false): Me {
  return {
    user: { id: "u1", email: "jana@example.cz", displayName: "Jana", emailVerified, mfaEnabled, hasPassword: true, googleLinked: false },
    space: role ? { slug: "firma", name: "Firma s.r.o.", role } : null,
  };
}

/**
 * Signs the active pinia's session in as a space member (`role`), or as a base-host user
 * (`null`). Call after `setActivePinia` and pass the same pinia to `mount`.
 */
export function signIn(role: Role | null = "owner", context: AppContext = role ? SPACE_CONTEXT : BASE_CONTEXT) {
  const session = useSessionStore();
  session.applyBoot({ status: "ok", context: structuredClone(context) });
  session.me = meFixture(role);
  session.loaded = true;
  return session;
}
