import type { DesignAuthor } from "./types";

/** Display name, else e-mail; null for a deleted account (the caller shows its own fallback). */
export const authorName = (author: DesignAuthor | null): string | null => (author ? author.displayName || author.email : null);
