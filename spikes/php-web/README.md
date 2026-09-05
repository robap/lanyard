# php-web — a second app, a second persona, and `/userinfo`

`jumbojett/openid-connect-php` against lanyard with defaults. It is Phase 4's
criterion 3 and half of criterion 4, and it is the client that proves
`/oidc/userinfo` answers a real SDK rather than a `curl`.

```
lanyard serve &          # 127.0.0.1:9500
cd spikes/php-web
composer install         # first time only — vendor/ is not committed
php -S localhost:5001 index.php
```

Then open <http://localhost:5001/>. You get a landing page, not a redirect —
click **Log in with lanyard** and pick **Mira Okonkwo**. The point of this spike
is being logged in as somebody *other* than whoever the .NET app is.

| URL | What it does |
|---|---|
| `/` | The whole page: a **Log in** button, or who you are, the claim table, and the two log-out buttons |
| `/secure` | Calls `authenticate()`, which starts the redirect. Same URL the .NET app uses for the same thing, because the page is shared |
| `/?code=…` | The callback. Redeems the code, reads UserInfo, then redirects to `/` so the address bar is clean and a reload cannot replay a spent code |
| `/logout` | **The real log-out**: `$oidc->signOut($idToken, 'http://localhost:5001/')`, which reads `end_session_endpoint` out of the discovery document, sends the ID token as `id_token_hint`, redirects, and exits. No lanyard URL appears in `index.php` |
| `/logout-local` | Drops **this app's** session only. lanyard still remembers you, so signing in again signs you straight back in — which is the difference the two buttons exist to show |

The page itself is [`../shared/page.html`](../shared/page.html), read at runtime
and shared with `dotnet-web` and `node-spa`. Nothing about it is PHP's.

Signed in, the claim table shows three rows — `email`, `name`, `sub`. They come
from three `requestUserInfo()` calls, which is
`GET /oidc/userinfo` with a bearer token, server to server. If lanyard's UserInfo
response were wrong or its access token unverifiable, this page would say
`FAILED:` and the exception.

## The whole configuration

```php
$oidc = new OpenIDConnectClient(
    'http://127.0.0.1:9500/oidc',  // issuer, plain HTTP
    'spike-php',                   // any client id, no registration
    'spike-secret'                 // any secret, checked by nobody
);
$oidc->setRedirectURL('http://localhost:5001/');
$oidc->addScope(['openid', 'email', 'profile']);
$oidc->setHttpUpgradeInsecureRequests(false);

// and, on /logout:
$oidc->signOut($_SESSION['id_token'], 'http://localhost:5001/');
```

`signOut()` **requires `end_session_endpoint` in the discovery document** or it
throws, and it needs the ID token kept in the session — which is the one line
Phase 5 added to the login path. The second argument is the
`post_logout_redirect_uri`: loopback, and therefore accepted.

**PHP needs one setting that .NET does not**, and one that .NET needs that PHP
does not. `setHttpUpgradeInsecureRequests(false)` stops jumbojett rewriting the
`http://` authorization URL to `https://`; there is no `RequireHttpsMetadata`
equivalent because the library never objected to a plain-HTTP issuer in the first
place. Measured in [`../../docs/decisions/php-spike.md`](../../docs/decisions/php-spike.md).

```
DROP=HttpUpgradeInsecureRequests php -S localhost:5001 index.php
```

## `authenticate()` is both halves of the flow

It starts the redirect *and* handles the callback, which is convenient and is
also why this spike used to call it on every page load — making the front door an
instant redirect with nothing readable in between. It now runs only for
`?action=login` and for the callback, so the flow is something you step through
rather than something that happens to you.

Anything that goes wrong — a refused UserInfo call, an issuer mismatch, a spent
code — surfaces as `FAILED: <class>` and the message on the page, which is what
you want from a spike and not what you want from an application.
