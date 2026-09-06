import { describe, it, expect } from "zero/test";
import { claimsJson, clientLabel, expiryLabel, statusClass } from "./format.ts";

describe("statusClass", () => {
  it("reads a redirect as a redirect, not a problem", () => {
    // Half of a browser login is a 302; colouring it as a warning would make
    // every successful login look like a failure.
    expect(statusClass(300)).toBe("redirect");
    expect(statusClass(302)).toBe("redirect");
    expect(statusClass(200)).toBe("ok");
    expect(statusClass(204)).toBe("ok");
  });

  it("separates a refusal from a fault", () => {
    expect(statusClass(400)).toBe("warn");
    expect(statusClass(401)).toBe("warn");
    expect(statusClass(499)).toBe("warn");
    expect(statusClass(500)).toBe("error");
  });
});

describe("expiryLabel", () => {
  const now = 1_757_080_931_000;
  const nowSeconds = Math.floor(now / 1000);

  it("gives the epoch integer and a human time for the same instant", () => {
    // 1757080991 is the `exp` in the spec's worked example.
    const label = expiryLabel(1_757_080_991, now);
    expect(label?.epoch).toBe(1_757_080_991);
    expect(label?.human).toBe("2025-09-05 14:03:11 UTC");
    expect(label?.expired).toBe(false);
  });

  it("reads a token minted with --expired as already expired", () => {
    const label = expiryLabel(nowSeconds - 3600, now);
    expect(label?.expired).toBe(true);
    expect(label?.epoch).toBe(nowSeconds - 3600);
  });

  it("treats the exact moment of expiry as expired", () => {
    expect(expiryLabel(nowSeconds, now)?.expired).toBe(true);
  });

  it("has nothing to say about a claim that is not a usable number", () => {
    expect(expiryLabel(undefined, now)).toBe(null);
    expect(expiryLabel("soon", now)).toBe(null);
    expect(expiryLabel(null, now)).toBe(null);
    // A number that is not a moment. `new Date(NaN)` throws on `toISOString`,
    // so this guard is what keeps a malformed claim from taking the row down.
    expect(expiryLabel(Number.NaN, now)).toBe(null);
    expect(expiryLabel(Number.POSITIVE_INFINITY, now)).toBe(null);
  });
});

describe("claimsJson", () => {
  it("pretty-prints with two spaces so the claims are read, not parsed", () => {
    expect(claimsJson({ alg: "RS256", kid: "k" })).toBe('{\n  "alg": "RS256",\n  "kid": "k"\n}');
  });

  it("renders an absent blob as `null` rather than as nothing", () => {
    expect(claimsJson(undefined)).toBe("null");
  });

  it("survives a value it cannot stringify", () => {
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    expect(claimsJson(cyclic)).toContain("could not be shown");
  });
});

describe("clientLabel", () => {
  it("says so when an event names no application", () => {
    expect(clientLabel(null)).toBe("—");
    expect(clientLabel("billing-web")).toBe("billing-web");
  });
});
