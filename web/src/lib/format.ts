// Rendering helpers for the live log: how a status reads, how an `exp` reads,
// and how a decoded claim blob is laid out. No DOM, no signals — the view
// imports these and the tests call them directly.

/** Milliseconds in a second, for the epoch-seconds claims JWTs carry. */
const MS = 1000;

/**
 * The status class a row is coloured by: `ok`, `redirect`, `warn` (4xx) or
 * `error` (5xx). Anything outside the four hundreds is `warn` only if it is a
 * client error, so a `302` in the middle of a login does not read as a problem.
 * @param {number} status
 * @returns {string}
 */
export function statusClass(status: number): string {
  if (status >= 500) return "error";
  if (status >= 400) return "warn";
  if (status >= 300) return "redirect";
  return "ok";
}

/** How an `exp` claim reads — both ways, because neither alone is enough. */
export type ExpiryLabel = {
  /** The epoch integer, verbatim, because that is what is in the token. */
  epoch: number;
  /** The same instant a person can read. */
  human: string;
  /** Whether it is already past, relative to the moment asked about. */
  expired: boolean;
};

/**
 * `exp` as **both** the epoch integer and a human time, plus whether it has
 * already passed.
 *
 * Both, because a developer comparing a token against a server's clock needs
 * the integer, and a developer asking "is this dead yet" needs the time — and a
 * token minted with `--expired` has to read as already expired at the moment it
 * was issued.
 * @param {unknown} exp
 * @param {number} now
 * @returns {ExpiryLabel | null}
 */
export function expiryLabel(exp: unknown, now: number): ExpiryLabel | null {
  if (typeof exp !== "number" || !Number.isFinite(exp)) return null;
  const at = exp * MS;
  return {
    epoch: exp,
    human: new Date(at).toISOString().replace("T", " ").replace(".000Z", " UTC"),
    // At the moment `exp` names, the token is already dead: RFC 7519 §4.1.4
    // says processing MUST be refused on or after it.
    expired: at <= now,
  };
}

/**
 * A decoded header or payload, pretty-printed for the claims viewer. Stable
 * two-space JSON — the claims are the diagnosis and they are read, not parsed.
 * @param {unknown} value
 * @returns {string}
 */
export function claimsJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2) ?? "null";
  } catch {
    // A claim blob comes off the wire; a viewer that threw would take the
    // whole row with it.
    return "(these claims could not be shown)";
  }
}

/**
 * The `client_id` column's text. An event that names no application says so
 * rather than rendering an empty cell that reads as a layout bug.
 * @param {string | null} clientId
 * @returns {string}
 */
export function clientLabel(clientId: string | null): string {
  return clientId ?? "—";
}
