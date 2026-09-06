// The live request log — lanyard's one screen with JavaScript. Subscribes to
// the SSE stream, batches incoming events per animation frame, and renders a
// dense table with a client_id filter, pause/resume, clear, and
// click-to-expand into the decoded claims.
//
// The claims panel is what the phase exists for: a login that bounced becomes
// three values side by side and a decoded token, read off a page rather than
// out of `src/`.

import { computed, each, effect, html, signal } from "zero";
import type { Signal, TemplateResult } from "zero";
import { Select } from "zero/components";
import { ALL_CLIENTS, appendCapped, clientIdsOf, matchesClient, timeAgo } from "../lib/log.ts";
import type { DecodedToken, LogEvent } from "../lib/log.ts";
import { claimsJson, clientLabel, expiryLabel, statusClass } from "../lib/format.ts";

/** Cap on retained rows. The server's ring holds 1000; this holds more. */
const MAX_ROWS = 2000;

/** Where the stream lives. Same origin — there is no other lanyard. */
const STREAM_URL = "/_/api/events";

/** Reactive state plus imperative handles for one live-log screen instance. */
type LogState = {
  events: Signal<LogEvent[]>;
  visible: { val: LogEvent[] };
  clients: Signal<string[]>;
  filter: Signal<string>;
  paused: Signal<boolean>;
  newCount: Signal<number>;
  expanded: Signal<number | null>;
  /** A coarse "current time" that ticks ~1s so the TIME cells advance. */
  now: Signal<number>;
  toggglePause: () => void;
  clear: () => void;
};

/**
 * @returns {TemplateResult}
 */
export default function LiveLog(): TemplateResult {
  const s = createLogState();
  return html`
    <section class="log-screen stack gap-0">
      ${Toolbar(s)}
      <div class="log-wrap">
        ${LogTable(s)}
        ${() =>
          s.visible.val.length === 0
            ? html`<p class="empty-state text-body text-center pad-xl">
                Waiting for requests… start a login, or run
                <code>lanyard token --as ada</code>.
              </p>`
            : ""}
      </div>
    </section>
  `;
}

/**
 * Assemble the reactive state: signals, the SSE ingest machinery, and the
 * filtered view.
 * @returns {LogState}
 */
function createLogState(): LogState {
  const events = signal<LogEvent[]>([]);
  const clients = signal<string[]>([]);
  const filter = signal(ALL_CLIENTS);
  const paused = signal(false);
  const newCount = signal(0);
  const expanded = signal<number | null>(null);

  // A shared clock, so the relative TIME labels advance without touching the
  // table body — only the per-cell bindings that read `now` re-run. Cleared
  // when the screen unmounts.
  const now = signal(Date.now());
  effect(() => {
    const id = setInterval(() => now.set(Date.now()), 1000);
    return () => clearInterval(id);
  });

  const ingest = createIngest(events, clients, paused, newCount);

  // Newest first: the freshest request sits at the top. `filter` returns a
  // fresh array, so reversing it never mutates the stored list.
  const visible = computed(() =>
    events.val.filter((e) => matchesClient(e, filter.val)).reverse(),
  );

  return {
    events,
    visible,
    clients,
    filter,
    paused,
    newCount,
    expanded,
    now,
    toggglePause: () => (paused.val ? ingest.resume() : paused.set(true)),
    clear: ingest.clear,
  };
}

/**
 * Open the SSE stream and hand each frame's `data` to `onFrame`, closing the
 * source when the screen unmounts.
 * @param {(data: string) => void} onFrame
 * @returns {void}
 */
function subscribeSse(onFrame: (data: string) => void): void {
  effect(() => {
    const es = new EventSource(STREAM_URL);
    es.onmessage = (msg) => onFrame(msg.data);
    return () => es.close();
  });
}

/**
 * Wire the stream to `events` with per-animation-frame batching and pause
 * buffering. Returns the handles the toolbar needs.
 * @returns {{ resume: () => void, clear: () => void }}
 */
function createIngest(
  events: Signal<LogEvent[]>,
  clients: Signal<string[]>,
  paused: Signal<boolean>,
  newCount: Signal<number>,
): { resume: () => void; clear: () => void } {
  let incoming: LogEvent[] = [];
  let scheduled = false;

  // Empty the local view: the retained rows, any buffered batch, and the
  // paused count. Driven both by the toolbar's Clear and by a server `clear`
  // frame, so one tab clearing empties every open tab.
  const clearView = () => {
    incoming = [];
    events.set([]);
    newCount.set(0);
  };

  const append = (batch: LogEvent[]) => {
    if (batch.length === 0) return;
    events.update((prev) => appendCapped(prev, batch, MAX_ROWS));
    // The filter offers what has actually been seen, and only grows — a
    // client id that scrolled out of the ring is still a thing you looked at.
    // The `> 0` is a skip, not a correctness guard: recomputing the same list
    // would render identically, just more often.
    const merged = clientIdsOf(batch).filter((id) => !clients.val.includes(id));
    if (merged.length > 0) clients.set(clientIdsOf(events.val));
  };

  const flush = () => {
    scheduled = false;
    if (incoming.length === 0) return;
    if (paused.val) {
      newCount.set(incoming.length);
      return; // stays buffered; resume flushes it
    }
    const batch = incoming;
    incoming = [];
    append(batch);
  };

  // A latch, not a correctness guard: without it a burst of frames would
  // queue one `requestAnimationFrame` each and flush the same batch.
  const schedule = () => {
    if (!scheduled) {
      scheduled = true;
      requestAnimationFrame(flush);
    }
  };

  subscribeSse((data) => {
    const parsed = parseFrame(data);
    if (parsed === "clear") {
      clearView();
      return;
    }
    if (parsed !== null) {
      incoming.push(parsed);
      schedule();
    }
  });

  return {
    clear: () => {
      void fetch("/_/api/events/clear", { method: "POST" }).catch(() => {});
      clearView();
    },
    resume: () => {
      paused.set(false);
      newCount.set(0);
      const batch = incoming;
      incoming = [];
      append(batch);
    },
  };
}

/**
 * One frame's payload: an event, the `clear` directive, or nothing at all — a
 * keep-alive, a `dropped` marker, or a line that is not JSON.
 * @param {string} data
 * @returns {LogEvent | "clear" | null}
 */
function parseFrame(data: string): LogEvent | "clear" | null {
  try {
    const parsed = JSON.parse(data) as Partial<LogEvent> & { clear?: boolean };
    if (parsed.clear === true) return "clear";
    return typeof parsed.id === "number" ? (parsed as LogEvent) : null;
  } catch {
    return null;
  }
}

/**
 * The toolbar: the client_id filter, the count, clear, and pause/resume.
 * @param {LogState} s
 * @returns {TemplateResult}
 */
function Toolbar(s: LogState): TemplateResult {
  const options = () => [
    { value: ALL_CLIENTS, label: "All applications" },
    ...s.clients.val.map((id) => ({ value: id, label: id })),
  ];
  return html`
    <div class="toolbar split align-center pad-md border-b">
      <div class="cluster align-center gap-md">
        ${() => Select({ value: s.filter, options: options(), size: "sm" })}
        <span class="count text-small"
          >${() => `${s.visible.val.length} / ${s.events.val.length}`}</span
        >
      </div>
      <div class="cluster align-center gap-sm">
        <button class="clear-btn pad-sm border" @click=${s.clear}>Clear</button>
        <button class="pause-btn pad-sm border" @click=${s.toggglePause}>
          ${() => (s.paused.val ? "Resume" : "Pause")}
          ${() =>
            s.paused.val && s.newCount.val > 0
              ? html`<span class="pause-count">${s.newCount}</span>`
              : ""}
        </button>
      </div>
    </div>
  `;
}

/**
 * The log table. Rows are keyed by event id; a click expands the detail row.
 * @param {LogState} s
 * @returns {TemplateResult}
 */
function LogTable(s: LogState): TemplateResult {
  return html`
    <table class="log-table">
      <thead>
        <tr>
          <th class="c-time text-start">TIME</th>
          <th class="c-client text-start">CLIENT</th>
          <th class="c-method text-start">METHOD</th>
          <th class="c-endpoint text-start">ENDPOINT</th>
          <th class="c-status text-start">STATUS</th>
          <th class="c-dur text-start">DUR</th>
          <th class="c-detail text-start">WHAT HAPPENED</th>
        </tr>
      </thead>
      <tbody>
        ${each(
          s.visible as unknown as Signal<LogEvent[]>,
          (e) => Row(e, s.expanded, s.now),
          (e) => e.id,
        )}
      </tbody>
    </table>
  `;
}

/**
 * One log row plus its conditionally rendered detail row.
 * @returns {TemplateResult}
 */
function Row(e: LogEvent, expanded: Signal<number | null>, now: Signal<number>): TemplateResult {
  const toggle = () => expanded.update((id) => (id === e.id ? null : e.id));
  return html`
    <tr class="log-row" @click=${toggle}>
      <td class="c-time">${() => timeAgo(e.ts, now.val)}</td>
      <td class="c-client">${clientLabel(e.client_id)}</td>
      <td class="c-method"><span class=${"method m-" + e.method.toLowerCase()}>${e.method}</span></td>
      <td class="c-endpoint">${e.endpoint}</td>
      <td class="c-status">
        <span class=${"pill s-" + statusClass(e.status)}>${e.status}</span>
      </td>
      <td class="c-dur">${e.duration_ms} ms</td>
      <td class="c-detail">${summary(e)}</td>
    </tr>
    <tr class=${() => "log-detail-row" + (expanded.val === e.id ? " open" : "")}>
      ${() => (expanded.val === e.id ? Detail(e, now) : "")}
    </tr>
  `;
}

/**
 * The one-line "what happened" cell — the whole `error_description` on a
 * failure, because the sentence is the point, and the grant on a success.
 * @param {LogEvent} e
 * @returns {string}
 */
function summary(e: LogEvent): string {
  if (e.error !== null) {
    const failure =
      e.error_description === null ? e.error : `${e.error} — ${e.error_description}`;
    return [failure, ...warningTail(e)].join("  ");
  }
  const parts = [e.grant_type, e.flaw === null ? null : `flaw=${e.flaw}`];
  return parts.filter((part) => part !== null).concat(warningTail(e)).join("  ");
}

/**
 * The warnings as they read on the row, matching the `warning: …` tail
 * `lanyard serve` prints. Last and always — on a success and on a failure
 * alike, because it is not what this request did but what is wrong while it was
 * being answered.
 * @param {LogEvent} e
 * @returns {string[]}
 */
function warningTail(e: LogEvent): string[] {
  return (e.warnings ?? []).map((w) => `warning: ${w}`);
}

/**
 * The expanded panel: the decoded claims of every token minted, and — for a
 * PKCE failure — the three values side by side.
 * @returns {TemplateResult}
 */
function Detail(e: LogEvent, now: Signal<number>): TemplateResult {
  const issued = e.issued ?? {};
  return html`
    <td class="detail-cell" colspan="7">
      <div class="stack gap-md pad-md">
        ${WarningsPanel(e)}
        ${PkcePanel(e)}
        ${Object.keys(issued).map((name) => TokenPanel(name, issued[name]!, now))}
        ${RequestPanel(e)}
      </div>
    </td>
  `;
}

/**
 * Every warning, in full. The row's cell carries the same sentences, but a
 * shadowed id names two file paths and that does not fit in a table cell.
 * @param {LogEvent} e
 * @returns {TemplateResult | string}
 */
function WarningsPanel(e: LogEvent): TemplateResult | string {
  const warnings = e.warnings ?? [];
  if (warnings.length === 0) return "";
  return html`
    <section class="warnings-panel stack gap-xs">
      <h3 class="text-h4">persona sources</h3>
      ${warnings.map((w) => html`<p class="warning-line text-small">${w}</p>`)}
    </section>
  `;
}

/**
 * **Which parameter mismatched.** The verifier presented, the challenge
 * computed from it, and the challenge recorded at `/authorize` — stacked, so a
 * reader sees which two were meant to be equal without being told.
 * @param {LogEvent} e
 * @returns {TemplateResult | string}
 */
function PkcePanel(e: LogEvent): TemplateResult | string {
  const pkce = e.detail?.["pkce"] as Record<string, string> | undefined;
  if (pkce === undefined) return "";
  const row = (label: string, key: string) =>
    html`<div class="pkce-row flank gap-md">
      <span class="pkce-label text-small">${label}</span>
      <code class="pkce-value">${pkce[key] ?? "—"}</code>
    </div>`;
  return html`
    <section class="pkce-panel stack gap-xs">
      <h3 class="text-h4">PKCE (${pkce["method"] ?? "—"})</h3>
      ${row("code_verifier presented", "verifier_presented")}
      ${row("challenge computed from it", "challenge_computed")}
      ${row("code_challenge recorded", "challenge_recorded")}
    </section>
  `;
}

/**
 * One minted token, decoded: the header that was emitted and the payload, with
 * `exp` read both ways.
 * @returns {TemplateResult}
 */
function TokenPanel(name: string, token: DecodedToken, now: Signal<number>): TemplateResult {
  return html`
    <section class=${"token-panel stack gap-xs t-" + name}>
      <h3 class="text-h4">${name}</h3>
      ${() => Expiry(token, now.val)}
      <div class="claims cluster gap-md align-stretch">
        <pre class="claims-header">${claimsJson(token.header)}</pre>
        <pre class="claims-payload">${claimsJson(token.payload)}</pre>
      </div>
    </section>
  `;
}

/**
 * `exp` as both the epoch integer and a human time, saying plainly whether the
 * token is already dead.
 * @returns {TemplateResult | string}
 */
function Expiry(token: DecodedToken, now: number): TemplateResult | string {
  const label = expiryLabel(token.payload["exp"], now);
  if (label === null) return "";
  return html`<div class=${"expiry" + (label.expired ? " expired" : "")}>
    <span class="expiry-label text-small">exp</span>
    <code class="expiry-epoch">${label.epoch}</code>
    <span class="expiry-human">${label.human}</span>
    <span class="expiry-state">${label.expired ? "already expired" : "still valid"}</span>
  </div>`;
}

/**
 * The decoded parameters, so "which one did I send" is answerable without a
 * network tab.
 * @param {LogEvent} e
 * @returns {TemplateResult | string}
 */
function RequestPanel(e: LogEvent): TemplateResult | string {
  if (e.request === null) return "";
  return html`
    <section class="request-panel stack gap-xs">
      <h3 class="text-h4">request</h3>
      <pre class="request-json">${claimsJson(e.request)}</pre>
    </section>
  `;
}
