// Rendering tests for the live-log screen. `EventSource` is out of scope in
// zero's in-memory web platform — "reach for them inside a test and stub them
// yourself" — so a controllable fake stands in, canned frames are dispatched,
// and the assertions are about the rendered table.
//
// `cleanup()` disposes the mounted component, but the runtime can carry an
// extra open fake across tests, so frames are broadcast to every open source:
// only the current component's source is wired to the current `el`, so exactly
// one row lands where the assertion looks.

import {
  describe, it, expect, beforeEach, afterEach,
  render, find, findAll, text, fire, cleanup,
} from "zero/test";
import type { LogEvent } from "../lib/log.ts";
import LiveLog from "./live-log.ts";

/** A controllable stand-in for the browser `EventSource`. */
type FakeSource = {
  url: string;
  onmessage: ((e: { data: string }) => void) | null;
  closed: boolean;
  close(): void;
};

const sources: FakeSource[] = [];

/**
 * `new EventSource(url)` — a constructor returning an object.
 * @param {string} url
 * @returns {FakeSource}
 */
function makeEventSource(url: string): FakeSource {
  const s: FakeSource = {
    url,
    onmessage: null,
    closed: false,
    close() {
      this.closed = true;
    },
  };
  sources.push(s);
  return s;
}

/** The still-open sources. */
function openSources(): FakeSource[] {
  return sources.filter((s) => !s.closed);
}

/**
 * Deliver one default-channel frame to every open source.
 * @param {string} data
 * @returns {void}
 */
function emitFrame(data: string): void {
  for (const s of openSources()) s.onmessage?.({ data });
}

/**
 * @param {Partial<LogEvent>} event
 * @returns {void}
 */
function emit(event: Partial<LogEvent>): void {
  emitFrame(JSON.stringify(sample(event)));
}

/**
 * @param {Partial<LogEvent>} over
 * @returns {LogEvent}
 */
function sample(over: Partial<LogEvent> = {}): LogEvent {
  return {
    id: 1,
    ts: Date.now(),
    client_id: "billing-web",
    endpoint: "/oidc/token",
    method: "POST",
    status: 200,
    duration_ms: 3,
    grant_type: "authorization_code",
    error: null,
    error_description: null,
    request: null,
    detail: null,
    issued: null,
    flaw: null,
    ...over,
  };
}

/**
 * A successful login's issuance: an access token and the ID token beside it,
 * shaped as `src/log_detail.rs` decodes them.
 * @returns {Record<string, { header: Record<string, unknown>, payload: Record<string, unknown> }>}
 */
function idTokenIssuance() {
  return {
    access_token: {
      header: { alg: "RS256", kid: "lanyard-dev-1", typ: "JWT" },
      payload: { sub: "ada", aud: "billing-api", exp: 1_757_080_991 },
    },
    id_token: {
      header: { alg: "RS256", kid: "lanyard-dev-1", typ: "JWT" },
      payload: { sub: "ada", aud: "billing-web", nonce: "n-0S6", email: "ada@example.test" },
    },
  };
}

/** Let the per-animation-frame batch flush and the rows commit. */
function flushFrames(): Promise<void> {
  return new Promise((res) =>
    requestAnimationFrame(() => requestAnimationFrame(() => res())),
  );
}

const g = globalThis as unknown as { EventSource?: unknown };
g.EventSource = makeEventSource;

describe("LiveLog", () => {
  beforeEach(() => {
    sources.length = 0;
  });
  afterEach(cleanup);

  it("says it is waiting before any traffic", () => {
    const el = render(LiveLog());
    expect(text(el, ".empty-state")).toContain("Waiting for requests");
    expect(findAll(el, ".log-row").length).toBe(0);
  });

  it("subscribes to the event stream on mount", () => {
    render(LiveLog());
    expect(openSources().some((s) => s.url === "/_/api/events")).toBe(true);
  });

  it("renders a row when an event arrives", async () => {
    const el = render(LiveLog());
    emit({});
    await flushFrames();
    const rows = findAll(el, ".log-row");
    expect(rows.length).toBe(1);
    expect(text(rows[0]!, ".c-client")).toContain("billing-web");
    expect(text(rows[0]!, ".c-method")).toContain("POST");
    expect(text(rows[0]!, ".c-endpoint")).toContain("/oidc/token");
    expect(text(rows[0]!, ".c-status")).toContain("200");
  });

  it("keeps appending across separate animation frames", async () => {
    // The batcher latches a scheduled frame; a latch that is never released
    // would show the first event and silently swallow every one after it.
    const el = render(LiveLog());
    emit({ id: 1 });
    await flushFrames();
    emit({ id: 2 });
    await flushFrames();
    expect(findAll(el, ".log-row").length).toBe(2);
  });

  it("colours the method and the status from the event itself", async () => {
    const el = render(LiveLog());
    emit({ id: 1, method: "POST", status: 400, error: "invalid_grant" });
    await flushFrames();
    expect(find(el, ".method.m-post")).not.toBe(null);
    expect(find(el, ".pill.s-warn")).not.toBe(null);
    expect(find(el, ".pill.s-ok")).toBe(null);
  });

  it("puts the newest request at the top", async () => {
    const el = render(LiveLog());
    emit({ id: 1, endpoint: "/oidc/authorize" });
    emit({ id: 2, endpoint: "/oidc/token" });
    await flushFrames();
    const rows = findAll(el, ".log-row");
    expect(rows.length).toBe(2);
    expect(text(rows[0]!, ".c-endpoint")).toContain("/oidc/token");
    expect(text(rows[1]!, ".c-endpoint")).toContain("/oidc/authorize");
  });

  it("names the failure's whole description on the row", async () => {
    const el = render(LiveLog());
    emit({
      status: 400,
      error: "invalid_grant",
      error_description:
        "the code_verifier does not match the S256 code_challenge this code was issued against",
    });
    await flushFrames();
    const row = find(el, ".log-row")!;
    expect(text(row, ".c-status")).toContain("400");
    expect(text(row, ".c-detail")).toContain("invalid_grant");
    expect(text(row, ".c-detail")).toContain("does not match the S256 code_challenge");
  });

  // Criterion 13: two projects at once, filtered to one.
  it("filters to one client_id, hiding the other's rows and only those", async () => {
    const el = render(LiveLog());
    emit({ id: 1, client_id: "billing-web" });
    emit({ id: 2, client_id: "php-demo" });
    await flushFrames();
    expect(findAll(el, ".log-row").length).toBe(2);

    const select = find(el, "select")!;
    (select as HTMLSelectElement).value = "php-demo";
    fire(select, "change");
    await flushFrames();

    const rows = findAll(el, ".log-row");
    expect(rows.length).toBe(1);
    expect(text(rows[0]!, ".c-client")).toContain("php-demo");
  });

  it("offers every client_id it has seen in the filter", async () => {
    const el = render(LiveLog());
    emit({ id: 1, client_id: "php-demo" });
    emit({ id: 2, client_id: "billing-web" });
    await flushFrames();
    const labels = findAll(el, "option").map((o) => o.textContent);
    expect(labels).toContain("billing-web");
    expect(labels).toContain("php-demo");
  });

  it("buffers while paused and releases on resume", async () => {
    const el = render(LiveLog());
    expect(text(el, ".pause-btn")).toContain("Pause");
    expect(find(el, ".pause-count")).toBe(null);

    fire(find(el, ".pause-btn")!, "click");
    await flushFrames();
    // The button says what it will do next, and there is nothing buffered yet.
    expect(text(el, ".pause-btn")).toContain("Resume");
    expect(find(el, ".pause-count")).toBe(null);

    emit({ id: 1 });
    await flushFrames();
    expect(findAll(el, ".log-row").length).toBe(0);
    expect(text(el, ".pause-count")).toContain("1");

    fire(find(el, ".pause-btn")!, "click");
    await flushFrames();
    expect(findAll(el, ".log-row").length).toBe(1);
    // Really resumed: the label flipped back, and the count is gone.
    expect(text(el, ".pause-btn")).toContain("Pause");
    expect(find(el, ".pause-count")).toBe(null);
  });

  it("keeps following once resumed", async () => {
    const el = render(LiveLog());
    fire(find(el, ".pause-btn")!, "click");
    await flushFrames();
    fire(find(el, ".pause-btn")!, "click");
    await flushFrames();
    emit({ id: 9 });
    await flushFrames();
    expect(findAll(el, ".log-row").length).toBe(1);
  });

  it("empties the view when the toolbar clears it", async () => {
    const el = render(LiveLog());
    emit({ id: 1 });
    await flushFrames();
    expect(findAll(el, ".log-row").length).toBe(1);

    fire(find(el, ".clear-btn")!, "click");
    await flushFrames();
    expect(findAll(el, ".log-row").length).toBe(0);
  });

  // The server's clear rides the default data channel, so one tab clearing
  // empties every open tab.
  it("empties the view when the server says the ring was cleared", async () => {
    const el = render(LiveLog());
    emit({ id: 1 });
    await flushFrames();
    emitFrame('{"clear":true}');
    await flushFrames();
    expect(findAll(el, ".log-row").length).toBe(0);
  });

  // ------------------------------------------------ the claims viewer --

  it("shows nothing expanded until a row is clicked", async () => {
    const el = render(LiveLog());
    emit({ issued: idTokenIssuance() });
    await flushFrames();
    expect(findAll(el, ".detail-cell").length).toBe(0);
  });

  // Criterion 14: the decoded header and payload of the ID token.
  it("expands a row into the decoded header and payload of every token minted", async () => {
    const el = render(LiveLog());
    emit({ issued: idTokenIssuance() });
    await flushFrames();

    fire(find(el, ".log-row")!, "click");
    await flushFrames();

    // Only the clicked row opens.
    expect(findAll(el, ".log-detail-row.open").length).toBe(1);

    const detail = find(el, ".detail-cell")!;
    expect(text(detail, ".t-access_token .claims-header")).toContain('"alg": "RS256"');
    expect(text(detail, ".t-access_token .claims-header")).toContain("lanyard-dev-1");
    expect(text(detail, ".t-id_token .claims-payload")).toContain('"email": "ada@example.test"');
    expect(text(detail, ".t-id_token .claims-payload")).toContain('"nonce": "n-0S6"');
  });

  it("collapses again on a second click", async () => {
    const el = render(LiveLog());
    emit({ issued: idTokenIssuance() });
    await flushFrames();
    fire(find(el, ".log-row")!, "click");
    await flushFrames();
    fire(find(el, ".log-row")!, "click");
    await flushFrames();
    expect(findAll(el, ".detail-cell").length).toBe(0);
    expect(findAll(el, ".log-detail-row.open").length).toBe(0);
  });

  // Criterion 15: the flaw is visible in the header that was actually emitted.
  it("shows an alg-none header as `none`", async () => {
    const el = render(LiveLog());
    emit({
      flaw: "alg-none",
      issued: {
        access_token: { header: { alg: "none", typ: "JWT" }, payload: { sub: "ada" } },
      },
    });
    await flushFrames();
    // The grant and the flaw, both readable, and not run together.
    expect(text(find(el, ".log-row")!, ".c-detail")).toBe("authorization_code  flaw=alg-none");

    fire(find(el, ".log-row")!, "click");
    await flushFrames();
    expect(text(el, ".claims-header")).toContain('"alg": "none"');
  });

  // Criterion 16: exp both ways, and an --expired token reads as already dead
  // at the moment it was issued.
  it("reads exp as both the epoch integer and a human time", async () => {
    const el = render(LiveLog());
    const nowSeconds = Math.floor(Date.now() / 1000);
    emit({
      issued: {
        access_token: {
          header: { alg: "RS256" },
          payload: { sub: "ada", iat: nowSeconds, exp: nowSeconds + 60 },
        },
      },
    });
    await flushFrames();
    fire(find(el, ".log-row")!, "click");
    await flushFrames();

    expect(text(el, ".expiry-epoch")).toContain(String(nowSeconds + 60));
    expect(text(el, ".expiry-human")).toContain("UTC");
    expect(text(el, ".expiry-state")).toContain("still valid");
  });

  it("reads a token minted with --expired as already expired", async () => {
    const el = render(LiveLog());
    const nowSeconds = Math.floor(Date.now() / 1000);
    emit({
      flaw: "expired",
      issued: {
        access_token: {
          header: { alg: "RS256" },
          payload: { sub: "ada", iat: nowSeconds, exp: nowSeconds - 3600 },
        },
      },
    });
    await flushFrames();
    fire(find(el, ".log-row")!, "click");
    await flushFrames();

    expect(text(el, ".expiry-state")).toContain("already expired");
    expect(find(el, ".expiry.expired")).not.toBe(null);
  });

  // Criterion 10: the three values, stacked, and the mismatched pair visibly
  // the one that differs.
  it("stacks the three PKCE values on a verifier mismatch", async () => {
    const el = render(LiveLog());
    emit({
      status: 400,
      error: "invalid_grant",
      error_description:
        "the code_verifier does not match the S256 code_challenge this code was issued against",
      detail: {
        pkce: {
          method: "S256",
          verifier_presented: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
          challenge_computed: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
          challenge_recorded: "K2-ltc83acc4h0c9w6ESC_rEMTJ3F50BXVuGJSstw-cM",
        },
      },
    });
    await flushFrames();
    fire(find(el, ".log-row")!, "click");
    await flushFrames();

    const rows = findAll(el, ".pkce-row");
    expect(rows.length).toBe(3);
    expect(text(rows[0]!, ".pkce-value")).toContain("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
    expect(text(rows[1]!, ".pkce-value")).toContain("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    expect(text(rows[2]!, ".pkce-value")).toContain("K2-ltc83acc4h0c9w6ESC_rEMTJ3F50BXVuGJSstw-cM");
    // The pair that was meant to be equal, and is not.
    expect(text(rows[1]!, ".pkce-value")).not.toBe(text(rows[2]!, ".pkce-value"));
    expect(text(el, ".pkce-panel h3")).toContain("S256");
  });

  it("says so when a PKCE value is missing rather than rendering a blank", async () => {
    const el = render(LiveLog());
    emit({ status: 400, error: "invalid_grant", detail: { pkce: { method: "plain" } } });
    await flushFrames();
    fire(find(el, ".log-row")!, "click");
    await flushFrames();
    const values = findAll(el, ".pkce-value").map((v) => v.textContent);
    expect(values).toEqual(["—", "—", "—"]);
  });

  it("shows the decoded request parameters", async () => {
    const el = render(LiveLog());
    emit({ request: { scope: ["openid", "email"], code_challenge_method: "S256" } });
    await flushFrames();
    fire(find(el, ".log-row")!, "click");
    await flushFrames();
    expect(text(el, ".request-json")).toContain('"code_challenge_method": "S256"');
    expect(text(el, ".request-json")).toContain('"openid"');
  });

  // A warning is not what this request did — it is the state of the persona
  // sources it was answered in. `/_/` emits no event, so a broken project file
  // reaches this page on the next protocol request.
  it("puts a source warning on the row that carried it", async () => {
    const el = render(LiveLog());
    emit({ warnings: ["/code/billing: linked directory does not exist"] });
    await flushFrames();
    const cell = find(el, "td.c-detail")!.textContent ?? "";
    expect(cell).toContain("warning");
    expect(cell).toContain("/code/billing");
  });

  // A warning rides on a refusal too — it is not what the request did, so a
  // failure does not displace it — and the two are separated on the one line.
  it("keeps the refusal and the warning apart on a failed row", async () => {
    const el = render(LiveLog());
    emit({
      error: "invalid_grant",
      error_description: "no such code",
      warnings: ["/code/billing: linked directory does not exist"],
    });
    await flushFrames();
    expect(find(el, "td.c-detail")!.textContent ?? "").toContain(
      "invalid_grant — no such code  warning: /code/billing",
    );
  });

  it("shows a single warning in the expanded panel", async () => {
    const el = render(LiveLog());
    emit({ warnings: ["/code/billing: linked directory does not exist"] });
    await flushFrames();
    fire(find(el, ".log-row")!, "click");
    await flushFrames();
    expect(findAll(el, ".warning-line").length).toBe(1);
  });

  it("shows every warning in the expanded panel", async () => {
    const el = render(LiveLog());
    emit({
      warnings: [
        "/code/billing: linked directory does not exist",
        'persona "ada" is defined in A and B; A wins',
      ],
    });
    await flushFrames();
    fire(find(el, ".log-row")!, "click");
    await flushFrames();
    const lines = findAll(el, ".warning-line").map((w) => w.textContent);
    expect(lines.length).toBe(2);
    expect(lines[1]).toContain("A wins");
  });

  it("renders no warning panel when the sources are healthy", async () => {
    const el = render(LiveLog());
    emit({});
    await flushFrames();
    fire(find(el, ".log-row")!, "click");
    await flushFrames();
    expect(findAll(el, ".warning-line").length).toBe(0);
    expect(find(el, "td.c-detail")!.textContent ?? "").not.toContain("warning");
  });

  it("ignores a frame that is not an event", async () => {
    const el = render(LiveLog());
    emitFrame("not json");
    emitFrame('{"dropped":12}');
    await flushFrames();
    expect(findAll(el, ".log-row").length).toBe(0);
    expect(text(el, ".empty-state")).toContain("Waiting for requests");
  });
});
