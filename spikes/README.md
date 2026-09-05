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
   at lanyard's picker. Click **Ada Bell** and you land back on `/secure` with
   `email: ada@example.test` and the full claim list. No password, no consent
   screen, no realm.

   Open the network tab before you click and you can read the whole protocol off
   it — six requests, and not one of them happened before you pressed a button:

   ```
   GET  302  localhost:5000/secure              [Authorize] → challenge
   GET  302  127.0.0.1:9500/oidc/authorize      lanyard stores the request
   GET  200  127.0.0.1:9500/_/?req=…            the picker            ← you are here
     …click Ada Bell…
   POST 200  127.0.0.1:9500/_/pick              mints the code
   POST 302  localhost:5000/signin-oidc         response_mode=form_post
   GET  200  localhost:5000/secure              signed in
   ```

   Two of those are worth noticing. `/_/pick` answers `200`, not `302`, because
   .NET asked for `response_mode=form_post` — so lanyard returns a page whose
   form POSTs the code, which is why the *next* request is a `POST`. And
   `/oidc/authorize` never appears in the address bar for more than an instant;
   it is a `302` you can only really see here.

2. **Open <http://localhost:5001/>.** The same *Not signed in* page from a
   completely different stack. Click **Log in with lanyard** and the picker
   appears *again* — different `client_id`, so lanyard asks again. Click
   **Mira Okonkwo**. The PHP app shows Mira.

3. **Go back to <http://localhost:5000/>.** Still Ada. Now visit
   <http://localhost:5000/logout> (this clears the .NET app's own cookie, not
   lanyard's) and then `/secure` again: **the picker does not appear** and you
   are Ada again.

   That is the whole claim. Two apps, one running provider, two different people
   at once, and nothing was configured to make it so. lanyard remembers your
   selection per `client_id`.

   > **There is no full log-out yet, and this step is where you notice.**
   > There are two sessions — the app's cookie and lanyard's — and nothing today
   > ends both. Deleting `lanyard_session` by hand does nothing visible, because
   > the app's own cookie means the browser never asks lanyard again; deleting
   > only the app's cookie sends you to lanyard, which still remembers you.
   >
   > What it should be is one click that clears both: the app drops its cookie
   > and redirects the browser through `/oidc/end_session`, lanyard drops
   > `lanyard_session`, and you land back on the app fully signed out. That is
   > [Phase 5](../ROADMAP.md#phase-5--session-lifecycle-and-the-remaining-endpoints).
   > Until then, the two working resets are **tick "Always ask"** in the picker
   > and **restart `lanyard serve`**.

4. **Open <http://localhost:5173/> and click Sign in.** A public client, no
   secret, PKCE `S256`. Third app, third session, same instance.

5. **Log in as `nobody`.** Start any of them again with `?prompt=login` — or just
   tick **Always ask** in the picker — and choose the person with no name and no
   email. The .NET app renders `email: (no email claim)`. That persona exists to
   break applications, and finding out which of yours it breaks is the point.

6. **Try to break the one rejection.** lanyard accepts any client id, any secret,
   any audience — and exactly one thing is refused:

   ```
   open 'http://127.0.0.1:9500/oidc/authorize?client_id=x&response_type=code&redirect_uri=https%3A%2F%2Fevil.example.com%2Fcb'
   ```

   A `400` rendered at lanyard, no redirect, naming the URI and the rule.

## Resetting

- **Close the browser** — the session cookie has no expiry, so it goes.
- **Restart `lanyard serve`** — sessions live in memory, so everybody is logged
  out and the picker comes back.
- **Tick "Always ask"** in the picker to see the picker every time without
  restarting anything. This is the closest thing to a log-out today.

There is deliberately **no "log out" button anywhere yet**. RP-initiated logout
(`/oidc/end_session`) and a visible control on `/_/` are
[Phase 5](../ROADMAP.md#phase-5--session-lifecycle-and-the-remaining-endpoints).

## Ports

`5000`, `5001`, `5080` and `5173` are the values baked into these spikes and into
the acceptance criteria. If one is taken on your machine, change it in that
spike's own config and in nothing else — lanyard does not need to be told.
