/** A locale's messages; leaves are precompiled message ASTs (see `vite.config.ts`). */
export type MessageTree = { [key: string]: unknown };

const namespaceOf = (path: string) => path.slice(path.lastIndexOf("/") + 1, -".json".length);

/**
 * Locale files are split per top-level namespace (`locales/<locale>/<namespace>.json`, file content
 * = that namespace) to keep each file small; merged here so runtime keys stay `namespace.key`.
 */
export const mergeNamespaces = (files: Record<string, MessageTree>): MessageTree =>
  Object.fromEntries(Object.entries(files).map(([path, tree]) => [namespaceOf(path), tree]));
