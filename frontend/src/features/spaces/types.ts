/** Ordered from the least to the most privileged; each role may do everything the previous one may. */
export const ROLES = ["accountant", "member", "admin", "owner"] as const;
export type Role = (typeof ROLES)[number];

/** `GET /api/context` — which app the current host serves. */
export interface AppContext {
  kind: "base" | "space";
  space: { slug: string; name: string } | null;
  /** Public registration is enabled on the base host. */
  registration: boolean;
  /** Sign in with Google is configured (the Google buttons are shown). */
  google: boolean;
  /** `INVOICE__PUBLIC_URL`, e.g. `https://invoiceapp.cz`. */
  baseUrl: string;
}

/** `GET /api/spaces` row, `POST /api/spaces` and `GET|PUT /api/space` response. */
export interface Space {
  slug: string;
  name: string;
  role: Role;
  /** Absolute URL of the space host. */
  url: string;
  /** Members must use TOTP to sign in (from their next login). */
  requireMfa: boolean;
}

/** `PUT /api/space`; `requireMfa` is owner only. */
export interface UpdateSpaceBody {
  name?: string;
  requireMfa?: boolean;
  /** Step-up, required with `requireMfa` (the owner's TOTP / recovery code). */
  code?: string;
}

export interface CreateSpaceBody {
  slug: string;
  name: string;
}

export interface DeleteSpaceBody {
  slug: string;
  password: string;
  /** Step-up, only for a user with TOTP. */
  code?: string;
}
