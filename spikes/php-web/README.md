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
| `/` | Landing page, or the claims if you are signed in |
| `/?action=login` | Calls `authenticate()`, which starts the redirect |
| `/?code=…` | The callback. Redeems the code, reads UserInfo, then redirects to `/` so the address bar is clean and a reload cannot replay a spent code |
| `/?action=logout` | Drops **this app's** session. lanyard still remembers you — full logout is roadmap Phase 5 |

Signed in, the page shows:

```
email: mira@example.test

name:  Mira Okonkwo
sub:   mira
```

Those three lines come from three `requestUserInfo()` calls, which is
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
```

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
