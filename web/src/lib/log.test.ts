import { describe, it, expect } from "zero/test";
import { ALL_CLIENTS, appendCapped, clientIdsOf, matchesClient, timeAgo } from "./log.ts";
import type { LogEvent } from "./log.ts";

/**
 * A LogEvent with sensible defaults, overridden per test.
 * @param {Partial<LogEvent>} over
 * @returns {LogEvent}
 */
function ev(over: Partial<LogEvent> = {}): LogEvent {
  return {
    id: 1,
    ts: 0,
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

describe("ALL_CLIENTS", () => {
  it("is the empty string, which is the filter select's `everything` value", () => {
    // The `<option value="">` the toolbar renders has to be this exact value,
    // or "All applications" selects a client id nobody has.
    expect(ALL_CLIENTS).toBe("");
  });
});

describe("matchesClient", () => {
  it("passes everything with no filter", () => {
    expect(matchesClient(ev(), ALL_CLIENTS)).toBe(true);
    expect(matchesClient(ev({ client_id: null }), ALL_CLIENTS)).toBe(true);
  });

  it("hides the other application's rows and only those", () => {
    expect(matchesClient(ev({ client_id: "billing-web" }), "billing-web")).toBe(true);
    expect(matchesClient(ev({ client_id: "php-demo" }), "billing-web")).toBe(false);
  });

  it("hides an event that names no application when one is asked for", () => {
    expect(matchesClient(ev({ client_id: null }), "billing-web")).toBe(false);
  });
});

describe("clientIdsOf", () => {
  it("lists each application once, sorted", () => {
    const events = [
      ev({ client_id: "php-demo" }),
      ev({ client_id: "billing-web" }),
      ev({ client_id: "php-demo" }),
    ];
    expect(clientIdsOf(events)).toEqual(["billing-web", "php-demo"]);
  });

  it("offers nothing for events that name no application", () => {
    expect(clientIdsOf([ev({ client_id: null })])).toEqual([]);
    expect(clientIdsOf([])).toEqual([]);
  });
});

describe("appendCapped", () => {
  it("appends in order", () => {
    const out = appendCapped([ev({ id: 1 })], [ev({ id: 2 }), ev({ id: 3 })], 10);
    expect(out.map((e) => e.id)).toEqual([1, 2, 3]);
  });

  it("keeps only the last `max` events", () => {
    const prev = [ev({ id: 1 }), ev({ id: 2 })];
    const out = appendCapped(prev, [ev({ id: 3 }), ev({ id: 4 })], 3);
    expect(out.map((e) => e.id)).toEqual([2, 3, 4]);
  });

  it("keeps a list that lands exactly on `max`", () => {
    const out = appendCapped([ev({ id: 1 }), ev({ id: 2 })], [ev({ id: 3 })], 3);
    expect(out.map((e) => e.id)).toEqual([1, 2, 3]);
  });

  it("returns the same list for an empty batch", () => {
    const prev = [ev({ id: 1 })];
    expect(appendCapped(prev, [], 10)).toBe(prev);
  });
});

describe("timeAgo", () => {
  const now = 1_700_000_000_000;

  it("shows `now` under a second", () => {
    expect(timeAgo(now, now)).toBe("now");
    expect(timeAgo(now - 999, now)).toBe("now");
  });

  it("shows seconds, minutes, hours and days at each threshold", () => {
    expect(timeAgo(now - 1_000, now)).toBe("1s");
    expect(timeAgo(now - 59_000, now)).toBe("59s");
    expect(timeAgo(now - 60_000, now)).toBe("1m");
    expect(timeAgo(now - 60 * 60_000, now)).toBe("1h");
    expect(timeAgo(now - 24 * 60 * 60_000, now)).toBe("1d");
    expect(timeAgo(now - 3 * 24 * 60 * 60_000, now)).toBe("3d");
  });

  it("clamps a future timestamp to `now`", () => {
    expect(timeAgo(now + 5_000, now)).toBe("now");
  });
});
