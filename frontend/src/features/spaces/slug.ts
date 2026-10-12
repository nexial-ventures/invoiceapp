// Same rules as the server (docs/api/spaces.md); the server stays authoritative (`taken` only it knows).
const SLUG = /^[a-z0-9](?:[a-z0-9-]{1,28})[a-z0-9]$/;

export const RESERVED_SLUGS = [
  "www", "api", "app", "admin", "mail", "smtp", "static", "assets", "cdn", "status", "docs", "help", "support", "blog",
  "login", "signin", "signup", "register", "account", "accounts", "auth", "oauth", "sso", "secure", "security",
  "verify", "verification", "password", "reset", "billing", "pay", "payment", "payments", "paypal", "bank", "banking",
  "google", "microsoft", "apple", "root", "system", "email", "webmail", "imap", "ns", "dns", "invoice", "invoices",
  "faktura", "faktury", "test", "demo",
] as const;

/** Reason code for a slug, or null when it is acceptable. */
export function slugError(slug: string): string | null {
  if (slug === "") return "required";
  if (!SLUG.test(slug)) return "invalid";
  if ((RESERVED_SLUGS as readonly string[]).includes(slug)) return "reserved";
  return null;
}

/** `host[:port]` of the base URL; a space lives at `{slug}.{host}`. */
export function baseHost(baseUrl: string): string {
  try {
    return new URL(baseUrl).host;
  } catch {
    return baseUrl;
  }
}
