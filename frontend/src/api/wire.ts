import type { components } from "./schema";

/**
 * Helpers over the generated `schema.d.ts` (`make frontend-api-types`). Response types alias the
 * generated schemas; request bodies stay hand-written in the features, because the server parses
 * them leniently (most fields `Option`) and the client sends a stricter shape than it accepts.
 */
export type Schemas = components["schemas"];

/**
 * serde writes every field of a response (an `Option` as `null`), so nothing the schema marks
 * optional is ever absent: strip the `?` all the way down. Not for DTOs with a
 * `skip_serializing_if` field (absent, not null): describe those by hand.
 */
export type Present<T> = T extends readonly (infer U)[]
  ? Present<U>[]
  : T extends object
    ? { [K in keyof T]-?: Present<Exclude<T[K], undefined>> }
    : T;

/** A response schema as received. */
export type Wire<K extends keyof Schemas> = Present<Schemas[K]>;

/**
 * `T` with some fields narrowed to the client's unions (the backend types them as plain strings).
 * Every narrowed field must exist on `T` and stay assignable to it, so a renamed or retyped wire
 * field fails the type-check instead of drifting.
 */
export type Narrow<T, N extends Partial<T> & Record<Exclude<keyof N, keyof T>, never>> = Omit<T, keyof N> & N;

/** The tuple must list every member of `T` exactly (order is the caller's display order). */
export const allOf =
  <T extends string>() =>
  <const A extends readonly T[]>(values: A & ([T] extends [A[number]] ? unknown : never)): A =>
    values;
