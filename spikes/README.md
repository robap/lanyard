# spikes — the apps lanyard is tested against

Four small applications, each one a real SDK doing a real thing. They exist to be
*observed against*: every acceptance criterion in `docs/features/` that says "a
.NET app logs in" is re-runnable here, by you, in a browser.

They are not examples. `examples/` — published, CI-run, deliberately boring — is
a different artifact and is post-v1. These are messier on purpose: they carry
`DROP=` switches for omitting a setting to see what breaks.

| Spike | Port | Stack | What it shows |
|---|---|---|---|
| [`dotnet-web/`](dotnet-web/) | 5000 | ASP.NET Core `AddOpenIdConnect` | A browser login, end to end. The picker, the ID token, the claims |
| [`php-web/`](php-web/) | 5001 | `jumbojett/openid-connect-php` | A second app with its own persona, and `/oidc/userinfo` answering a real client |
| [`node-spa/`](node-spa/) | 5173 | `oidc-client-ts`, no build step | A **public client** with PKCE `S256` and no `client_secret` anywhere |
| [`dotnet-api/`](dotnet-api/) | 5080 | ASP.NET Core `AddJwtBearer` | A resource server. Where `lanyard token` and the six failure flags get pointed |

Each of the three web apps also commits a **`lanyard.yaml`** — the personas it
needs, travelling with the repo. `dotnet-web` and `php-web` scope theirs with
`client:`, so each gets its own picker; `node-spa` deliberately does not, so its
`kiosk` is everybody's. One `lanyard link` per file and they are there.

The three web apps render **one page**: [`shared/page.html`](shared/page.html),
read at runtime by `dotnet-web` and `php-web` and fetched by `node-spa`. Nothing
is copied, generated, or built. That is the point — when the page is a constant,
every visible difference between two stacks is a difference in the stack, which
is the question these applications exist to answer. `curl -s localhost:5000/`
and `curl -s localhost:5001/` differ in the application's name and in nothing
else.

## Five minutes, from nothing

Everything below runs on loopback and needs no configuration. **Nothing is ever
registered with lanyard** — the client ids in these apps were invented in their
own source files.

**Terminal 1 — lanyard.**

```
cargo build --release
./target/release/lanyard serve
```

The banner prints a `UI →` line. That is the persona picker, and you can open it
now: <http://127.0.0.1:9500/_/>. It lists the three built-in people and says no
login is in progress.

**Then link the three projects**, once, from anywhere:

```
lanyard link spikes/dotnet-web/lanyard.yaml
lanyard link spikes/php-web/lanyard.yaml
lanyard link spikes/node-spa/lanyard.yaml
```

The file is named, not discovered. Each `link` prints the absolute path it
recorded and the personas it found; `lanyard links` lists them later. `lanyard
serve` needs no restart — it re-reads the registry on the next request — and the
banner's `Personas →` block now has a line per source.

**Terminal 2 — the .NET app.**

```
cd spikes/dotnet-web
dotnet run
```

**Terminal 3 — the PHP app.** The dependency is not committed, so install it once:

```
cd spikes/php-web
composer install          # first time only
php -S localhost:5001 index.php
```

**Terminal 4 — the SPA.** `oidc-client-ts` is vendored, so there is no
`npm install` and no `node` involved:

```
cd spikes/node-spa
python3 -m http.server 5173
```

## What to actually look at

Do these in order, **in one browser profile**. The order is the point.

1. **Open <http://localhost:5000/>.** A page that says *Not signed in* and
   nothing else has happened yet — **the front door does not redirect**, so you
   can read it before the flow starts. Click **Log in with lanyard**: now you are
   at lanyard's picker.

   **Look at who is on it.** `dev-admin`, `billing-readonly` and `locked-out` —
   the people in `spikes/dotnet-web/lanyard.yaml` — plus `ada`, `mira` and
   `nobody`, because links add and never subtract. `qa-bot` is *not* there: it
   belongs to `spike-php`. Under the list, lanyard says how many it hid and
   offers **Show all**, which lists every persona from every source with the
   client and the file each came from — without abandoning the login.

   Click **Dev Admin** and you land back on the same page, signed in as
   `dev-admin@billing.test`, with every claim the app received in a table. No
   password, no consent screen, no realm — and nothing about `dev-admin` was
   configured anywhere but in a file this repository ships.

   Open the network tab before you click and you can read the whole protocol off
   it — seven requests, and not one of them happened before you pressed a
   button:

   ```
   GET  302  localhost:5000/secure              [Authorize] → challenge
   GET  302  127.0.0.1:9500/oidc/authorize      lanyard stores the request
   GET  200  127.0.0.1:9500/_/?req=…            the picker            ← you are here
     …click Ada Bell…
   POST 200  127.0.0.1:9500/_/pick              mints the code
   POST 302  localhost:5000/signin-oidc         response_mode=form_post
   GET  302  localhost:5000/secure              now signed in, so it just redirects
   GET  200  localhost:5000/                    the claim table
   ```

   Two of those are worth noticing. `/_/pick` answers `200`, not `302`, because
   .NET asked for `response_mode=form_post` — so lanyard returns a page whose
   form POSTs the code, which is why the *next* request is a `POST`. And
   `/oidc/authorize` never appears in the address bar for more than an instant;
   it is a `302` you can only really see here.

2. **Open <http://localhost:5001/>.** The same *Not signed in* page from a
   completely different stack. Click **Log in with lanyard** and the picker
   appears *again* — different `client_id`, so lanyard asks again.

   **It is a different picker.** `qa-bot` is on it and `dev-admin` is not. Same
   instance, same port, same browser: the list a developer reads is the one
   their project shipped, which is the whole of "one instance, every project"
   stopping being a tax on the tenth project. `kiosk` is on both, because
   `node-spa` scoped nobody.

   Click **QA Bot**. The PHP app shows `qa-bot@php.test`.

3. **Go back to <http://localhost:5000/>.** Still Dev Admin. Now click **Clear
   this app's cookie only** — the second, quieter button — and then **Log in
   with lanyard** again: **the picker does not appear** and you are Dev Admin
   again.

   That is the whole claim. Two apps, one running provider, two different people
   at once, each picked from a list its own repository shipped, and nothing was
   configured to make it so. lanyard remembers your selection per `client_id`,
   and clearing an application's own cookie does not touch that.

4. **Open <http://localhost:5173/> and click Sign in.** A public client, no
   secret, PKCE `S256`. Third app, third session, same instance. Its picker
   carries `kiosk` — its own, unscoped — and neither `dev-admin` nor `qa-bot`.
   Pick **Kiosk User**. Leave it open
   for a minute and watch the network tab: with `offline_access` and
   `automaticSilentRenew`, the access token renews through a
   `POST /oidc/token` with `grant_type=refresh_token` — **no redirect, no
   `/authorize`, no iframe**, and the address bar never moves.

5. **Look at <http://127.0.0.1:9500/_/>.** With no login in progress this is the
   **unfiltered** view: every persona from every source, each labelled with the
   client it is scoped to and the file it came out of. This is the page to open
   when a filtered picker surprised you.

   The **This browser** section now lists
   `billing-web` → Dev Admin, `spike-php` → QA Bot and `node-spa` → Kiosk User,
   each with
   two buttons. **Forget** drops one application's person, so its next login
   shows the picker and the others are untouched. **Expire now** kills that
   application's live tokens and **keeps** the person — so the application's own
   renew path runs, rather than the picker appearing. That asymmetry is the
   whole reason both buttons exist.

6. **Log in as `nobody`.** Start any of them again with `?prompt=login` — or just
   tick **Always ask** in the picker — and choose the person with no name and no
   email. It is still on every picker, because links add and never subtract, and
   losing the flagship persona by linking a project would be a bad trade. The
   .NET app renders no `email` row at all. That persona exists to break
   applications, and finding out which of yours it breaks is the point.

   `locked-out` is the same idea with a name on it: a real account, with claims,
   that your authorization code is supposed to turn away.

7. **Try to break the one rejection.** lanyard accepts any client id, any secret,
   any audience — and exactly one thing is refused:

   ```
   open 'http://127.0.0.1:9500/oidc/authorize?client_id=x&response_type=code&redirect_uri=https%3A%2F%2Fevil.example.com%2Fcb'
   ```

   A `400` rendered at lanyard, no redirect, naming the URI and the rule. The
   same rule guards `post_logout_redirect_uri`, and a rejected log-out logs
   nobody out:

   ```
   open 'http://127.0.0.1:9500/oidc/end_session?post_logout_redirect_uri=https%3A%2F%2Fevil.example.com%2F'
   ```

8. **Break a link on purpose.** `mv spikes/php-web/lanyard.yaml /tmp/` while
   `lanyard serve` is running, then reload <http://127.0.0.1:9500/_/>. The page
   renders `200` with a band at the top naming the file that is gone; every other
   source's personas are still listed; the process is still serving. Start a
   login from `dotnet-web` and the warning is on that request's line on
   `serve`'s stdout, on `/_/log`, and in `lanyard logs --json`. `mv` it back and
   reload — `qa-bot` returns, with no restart and no re-`link`.

   **A machine-wide daemon must not die because one of ten projects has a
   typo.** The fatal-ness moved rather than disappearing: `lanyard link` parses
   the file and refuses a bad one.

9. **Mint from the CLI, for a scoped persona.** The CLI sends
   `client_id=lanyard-cli`, so `dev-admin` is invisible to it:

   ```
   lanyard token --as dev-admin
   ```

   It exits non-zero with an empty stdout and a stderr line naming the file
   `dev-admin` is defined in, the client it is scoped to, and the flag that
   fixes it. Do as it says:

   ```
   curl -H "Authorization: Bearer $(lanyard token --as dev-admin \
     --client billing-web --aud billing-api)" localhost:5080/orders
   ```

   `kiosk` needs no flag at all — `node-spa` scoped nobody:

   ```
   lanyard token --as kiosk --aud billing-api
   ```

10. **Log out — and do this one last, because it ends the demo.** Click **Log
   out** in `php-web`. Your browser goes `localhost:5001` →
   `127.0.0.1:9500/oidc/end_session` → back to `localhost:5001`, signed out of
   both sessions with no cookie deleted by hand.

   Now go to <http://localhost:5000/>, click **Clear this app's cookie only**,
   and log in again: **the picker appears**, where step 3 signed you straight
   back in as Ada. That is the cost of the decision, and it is deliberate —
   there is one SSO session and logging out clears all of it, exactly as every
   real IdP does. Logging out of one application logs you out of all three.

   Steps 1–9 all still work; they just start over. That is why this step is
   last.

## Resetting

- **Click Log out** in any of the three apps — one click, both sessions, and the
  other two apps with them.
- **Log out of lanyard** on <http://127.0.0.1:9500/_/> — the same thing from
  lanyard's side, without going near an application.
- **Close the browser** — the session cookie has no expiry, so it goes.
- **Restart `lanyard serve`** — sessions, refresh tokens and revocations all
  live in memory, so everybody is logged out and the picker comes back.
- **Tick "Always ask"** in the picker to see the picker every time without
  logging anybody out.
- **`lanyard unlink`** each file to put the persona list back to the three
  built-ins. The `lanyard.yaml` files stay in the repositories; re-linking is one
  command.

## Ports

`5000`, `5001`, `5080` and `5173` are the values baked into these spikes and into
the acceptance criteria. If one is taken on your machine, change it in that
spike's own config and in nothing else — lanyard does not need to be told.
