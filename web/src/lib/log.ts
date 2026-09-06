// The live log's pure logic, kept out of the view so it is unit-testable with
// no DOM: the client_id filter predicate, the capped ring append, and the
// relative-time label.

/** A JWT as the server decoded it — the header and payload that were emitted. */
export type DecodedToken = {
  header: Record<string, unknown>;
  payload: Record<string, unknown>;
};

/**
 * One captured request, exactly as `src/events.rs` serializes it. Every field
 * is present on every event; the ones that did not happen are `null`.
 */
export type LogEvent = {
  id: number;
  ts: number;
  client_id: string | null;
  endpoint: string;
  method: string;
  status: number;
  duration_ms: number;
  grant_type: string | null;
  error: string | null;
  error_description: string | null;
  request: Record<string, unknown> | null;
  detail: Record<string, unknown> | null;
  issued: Record<string, DecodedToken> | null;
  flaw: string | null;
};

/** The filter value that means "every application". */
export const ALL_CLIENTS = "";

/**
 * Whether an event passes the `client_id` filter. `ALL_CLIENTS` passes
 * everything; anything else is an exact match, so filtering to one application
 * hides the other's rows **and only those**.
 * @param {LogEvent} e
 * @param {string} clientId
 * @returns {boolean}
 */
export function matchesClient(e: LogEvent, clientId: string): boolean {
  if (clientId === ALL_CLIENTS) return true;
  return e.client_id === clientId;
}

/**
 * The `client_id`s present in `events`, sorted, for the filter control. An
 * event with none contributes nothing — there is no row to offer.
 * @param {LogEvent[]} events
 * @returns {string[]}
 */
export function clientIdsOf(events: LogEvent[]): string[] {
  const seen = new Set<string>();
  for (const e of events) {
    if (e.client_id !== null) seen.add(e.client_id);
  }
  return [...seen].sort();
}

/**
 * Append `batch` to `prev`, retaining only the most recent `max` events. An
 * empty batch returns `prev` unchanged (same reference), so a frame that
 * carried nothing cannot churn the rendered list.
 * @param {LogEvent[]} prev
 * @param {LogEvent[]} batch
 * @param {number} max
 * @returns {LogEvent[]}
 */
export function appendCapped(prev: LogEvent[], batch: LogEvent[], max: number): LogEvent[] {
  if (batch.length === 0) return prev;
  const next = prev.concat(batch);
  // `>` vs `>=` is an equivalent mutation here: at exactly `max` the slice
  // starts at 0 and returns the same elements. Left as `>` because "trim only
  // when over" is what the sentence means.
  return next.length > max ? next.slice(next.length - max) : next;
}

/**
 * A human "time-ago" label for an event's wall-clock `ts` relative to `now`
 * (both Unix ms): `now` under a second, then `5s`, `2m`, `1h`, `3d`. Coarse and
 * monotonic — it stays readable as the log ages, unlike seconds-since-first.
 * @param {number} ts
 * @param {number} now
 * @returns {string}
 */
export function timeAgo(ts: number, now: number): string {
  const secs = Math.max(0, Math.floor((now - ts) / 1000));
  if (secs < 1) return "now";
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}
