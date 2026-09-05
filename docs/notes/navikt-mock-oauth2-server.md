# Notes — navikt/mock-oauth2-server README

Read end to end 2026-09-04 (686 lines, `master`). CONCEPT §12 makes this a
standing instruction: it is the most thorough thing in the category and its
Docker networking notes cover the same ground as CONCEPT §8.

Instrument, not competitor — per the spec, its login-screen design is explicitly
out of scope here.

## Docker networking behaviours

### 1. The issuer is derived per-request from the URL — **DIFFER**

Discovery at `http://localhost:8080/default/.well-known/openid-configuration`
returns:

```json
{
   "issuer": "http://localhost:8080/default",
   "authorization_endpoint": "http://localhost:8080/default/authorize",
   "jwks_uri": "http://localhost:8080/default/jwks"
}
```

Host, port, and the first path segment all come from the incoming request. Reach
the same server at `mock-oauth2-server:8080` and every URL in that document
changes, including `issuer`.

**This is precisely what CONCEPT §8 tells us not to do**, and the README is the
evidence for why: two of its sections exist only to work around it. The Windows
note — `docker run -p 8080:8080 -h localhost $IMAGE_NAME` — is the same
workaround again, setting the *container's* hostname so the derived issuer comes
out saying `localhost`.

lanyard resolves one issuer at startup, prints it, and serves it to everyone.
Worth citing this README in our own docs as the thing we deliberately didn't copy.

### 2. `host.docker.internal` as the one name that resolves from both sides — **COPY**

Their Scenario 2, for when a browser is involved (i.e. any authorization code
flow):

> 1. Add `127.0.0.1 host.docker.internal` to your `/etc/hosts` file (Linux only;
>    macOS and Windows Docker Desktop add this automatically).
> 2. Set `hostname: host.docker.internal` on the mock server service.

Same technique CONCEPT §8 lands on: make one name resolve identically from the
host and from inside the network, then use it everywhere. Copy the technique.

Two refinements for us:

- CONCEPT §8 offers `lanyard` + `127.0.0.1 lanyard` as the alternative. Prefer it
  — `host.docker.internal` on Linux needs `--add-host=host.docker.internal:host-gateway`
  for container→host traffic, so the name means two different things depending on
  direction, and their `/etc/hosts` line only fixes the browser half.
- **Caution from our own .NET spike:** `http://lanyard:9500` is *not* a secure
  context, while `http://localhost:9500` is. See
  [dotnet-http-settings.md](../decisions/dotnet-http-settings.md). Whatever name
  we recommend, lanyard must not set `Secure` on its cookies in HTTP mode.

### 3. Host port and container port are allowed to differ — **DIFFER (warn)**

Their own Compose example publishes `8090:8080`:

```yaml
  mock-oauth2-server:
    image: ghcr.io/navikt/mock-oauth2-server:$MOCK_OAUTH2_SERVER_VERSION
    ports:
      - 8090:8080
```

> Your app reaches the mock server at `http://mock-oauth2-server:8080` (internal
> Docker network). From your host machine the mock server is at
> `http://localhost:8090`.

Because their issuer embeds host **and port**, those are two different issuer
strings for one server. It is only safe in Scenario 1, where no browser ever sees
a token. CONCEPT §8's "port must match on both sides of the colon" is the
right rule; this README is a live example of the failure mode it prevents.

lanyard should default them equal and **warn on mismatch** — the README does not
warn, it just documents the asymmetry, and a reader skimming for a Compose
snippet will copy `8090:8080` straight into a browser flow.

### 4. `SERVER_HOSTNAME` binds the wildcard by default — **COPY the knob, DIFFER on the default**

> `SERVER_HOSTNAME` — Hostname to bind to. Defaults to the wildcard address
> (typically `0.0.0.0` or `::` …)

Correct for a container-first tool, wrong for lanyard, which is native-first
(CONCEPT §7). CONCEPT §8 already has the right shape: a `LANYARD_BIND` env var so
the container binds `0.0.0.0` while the native binary stays on loopback. Keep
loopback as the default and let the image set the variable.

### 5. Health endpoint — **COPY**

> Health check: `GET /isalive` returns `200` when the server is ready.

Exactly the `depends_on: condition: service_healthy` enabler CONCEPT §8 asks for.
Copy the behaviour, not the spelling — `/health` or `/healthz` reads better to
people outside NAV.

### 6. CORS reflected automatically — **COPY, with a narrower rule**

> The server automatically adds CORS headers to every response when an `Origin`
> header is present. No configuration is required.
> `Access-Control-Allow-Origin: <origin>` / `Access-Control-Allow-Credentials: true`

Zero-config CORS matches our north star and CONCEPT §6's "CORS on `/token` and
`/jwks`". They reflect *any* origin; CONCEPT §6 says accept any **localhost**
origin, which is the better boundary and consistent with our localhost-only
redirect rule. Copy the ergonomics, keep the tighter rule.

## Non-networking, noted in passing

- **Multi-issuer by first path segment** (`/issuer-a/…`, `/issuer-b/…`, no
  config) — **IGNORE.** Clever, but it is the mechanism behind behaviour 1, and
  it collides with CONCEPT §3's decision to mount OIDC under `/oidc/...`. Our
  multi-project story is "any client_id, one issuer", which is a different and
  simpler bet.
- **HTTPS via `NettyWrapper` + a PKCS12 keystore**, with a "add
  `ssl.sslKeystore.keyStore` to your client's truststore" tip — **DIFFER.**
  Per-client truststore edits are the ergonomics CONCEPT §9 rejects in favour of
  a local CA installed once. Their approach does confirm §9's "runtimes do not
  inherit OS trust" point though.
- **`interactiveLogin`, `loginPagePath`, and `requestMappings` matching on
  `"requestParam": "subject"`** — the closest thing in the category to our
  persona picker. Out of scope per the spec; noted and left alone.
- **`tid` auto-added on every token** (set to the `issuerId`) — interesting
  overlap with CONCEPT §6's Entra claim profile, where `tid` is the tenant id.
  Ours should come from the profile, not from an issuer name.
- **README framing** — CONCEPT §13 already dissects the "Scriptable OAuth2/OpenID
  Connect server for JVM tests and Docker Compose" first line. Reading the whole
  thing confirms the diagnosis: the document is excellent reference material and
  never once says what problem it saves you from.
