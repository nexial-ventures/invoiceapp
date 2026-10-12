import { ROLES, type Role } from "./types";

/** What the UI shows or hides per role; the server enforces the same matrix (docs/api/spaces.md). */
export type Action = "read" | "write" | "settings" | "unlockDocument" | "viewAllTokens" | "deleteSpace" | "spacePolicy";

const MIN_ROLE: Record<Action, Role> = {
  read: "accountant",
  write: "member",
  settings: "admin",
  /** Issued document back to a draft (docs/api/documents.md, Unlock). */
  unlockDocument: "admin",
  viewAllTokens: "admin",
  deleteSpace: "owner",
  /** The "require TOTP" switch (docs/api/mfa.md). */
  spacePolicy: "owner",
};

const rank = (role: Role) => ROLES.indexOf(role);

/** `null` = no space role (base host, signed out): nothing space-related is allowed. */
export function can(role: Role | null | undefined, action: Action): boolean {
  return role != null && rank(role) >= rank(MIN_ROLE[action]);
}

/** Roles a token created by `role` may carry (≤ the caller's role). */
export function rolesUpTo(role: Role): Role[] {
  return ROLES.slice(0, rank(role) + 1);
}
