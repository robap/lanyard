# Name check — "lanyard"

**Verdict: no conflict. Keep the name.** No existing developer tool named
lanyard occupies the identity/auth/local-dev-server space. The name is busy in a
neighbouring space (Discord presence), which affects discoverability, not
trademark or confusion.

Run 2026-09-04. CONCEPT §14 asked for this: registry availability was already
known; an *established project with the same name and a similar audience* was the
open question.

## Queries run

### GitHub — repositories named `lanyard`, by stars

`https://api.github.com/search/repositories?q=lanyard+in:name&sort=stars&order=desc`

**354 total repositories.** Top 10:

| Stars | Repo | Description |
|---|---|---|
| 1463 | `Phineas/lanyard` | Expose your Discord presence and activities to a RESTful API and WebSocket |
| 878 | `cnrad/lanyard-profile-readme` | Discord Presence in your GitHub Profile |
| 90 | `alii/use-lanyard` | React hook for Lanyard for tracking your Discord presence |
| 76 | `ourzora/lanyard` | Decentralized allowlists for web3 |
| 70 | `barbarbar338/react-use-lanyard` | Use Lanyard API easily in your React app |
| 28 | `bringhurst/lanyard` | WebGL GIS visualization, based on WorldWind Java |
| 28 | `xaronnn/js-lanyard` | Pure JavaScript library for Lanyard |
| 15 | `pxseu/lanyard-ui` | UI to access kv and visualize your Discord User / Status |
| 13 | `dustinrouillard/lanyard-online-users` | Page displaying online Lanyard users |
| 13 | `renderghost/lanyards` | Profile for researchers, built on the AT Protocol |

### Web — `"lanyard" developer tool CLI`

Top results: `skupperproject/lanyard` (Alpine Docker image bundling networking and
diagnostic tools), the GitHub `lanyard` and `lanyard-api` topic pages,
`eggsy/lanyard-visualizer`, `Phineas/lanyard`, `pxseu/lanyard-ui`,
`lanyard.eggsy.xyz`.

### Web — `"lanyard" OIDC OR OAuth OR "identity provider" open source`

**No result named lanyard.** Results were the expected field — Ory Hydra, Dex,
Authentik, OpenIddict, MITREid Connect, `panva/node-oidc-provider`, the OpenID
Foundation certification list, `cerberauth/awesome-openid-connect`.

### Web — `"lanyard" local development server authentication tool github`

Top results: `pxseu/lanyard-ui`, `Haykkonen/LanyardMeteor` (Meteor modules for
network analysis and development testing), `eggsy/lanyard-visualizer`,
`barbarbar338/react-use-lanyard`, `zeropingheroes/lanyard` (LAN party ticket
selling and management), `skupperproject/lanyard`, `Phineas/lanyard`.

### crates.io

See [crates-io-reservation.md](crates-io-reservation.md). `lanyard` is an
unrelated FFI string crate (0.1.3, last published 2024-06-11); `lanyard-cli` is
free.

## Reading

**The one prominent project is `Phineas/lanyard`** at ~1.5k stars, and most of
the rest of the GitHub list is its ecosystem — wrappers, React hooks, profile
embeds. It is a Discord presence API. Different audience, different problem,
no overlap with a local OIDC provider; nobody choosing between them.

The other hits are a web3 allowlist tool, a LAN-party ticketing system, a WebGL
GIS viewer, and a Docker image of network diagnostic tools. None is an identity
provider, an auth library, or a local development service. Nothing here would
make a developer land on the wrong project or think ours is a fork of theirs.

**The real cost is SEO, not confusion.** A bare search for "lanyard github"
returns Discord tooling for the whole first page. Practical consequences:

- Never use the bare word as the searchable identity. The repo description,
  README first line, and any post title should always carry the qualifier —
  "lanyard — a local OIDC provider for development", not "lanyard".
- This reinforces CONCEPT §13's first-line test rather than conflicting with it.
- The GitHub `lanyard` topic is effectively Discord's. Use topics like
  `oidc`, `oauth2`, `identity-provider`, `developer-tools` instead.

Nothing here justifies a rename. Re-check before Phase 10 (distribution) if any
of the above changes.
