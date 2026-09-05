# PHP over plain HTTP — what was needed

**Nothing beyond defaults.** CONCEPT §10 expected PHP to be relaxed about
`http://` and not block us. Confirmed by running it.

## What was run

| | |
|---|---|
| PHP | 8.3.6 (cli, NTS), Pop!_OS 24.04 |
| Composer | 2.10.3 |
| Library | `jumbojett/openid-connect-php` v1.0.2 |
| Pulled in | `phpseclib/phpseclib` 3.0.57, `paragonie/constant_time_encoding` v3.1.3, `paragonie/random_compat` v9.99.100 |
| Server | `php -S localhost:5001 -t .` — no nginx, no Apache |
| Provider | `oidc-provider-mock` 0.4.6, issuer `http://localhost:9400` |
| Browser | Google Chrome for Testing 149.0.7827.55, driven with Selenium |

## The extension that had to be installed

`php -m` originally listed `json` and `openssl` but **not `curl`**, and
`jumbojett/openid-connect-php` declares `ext-curl` as a hard requirement, so
`composer require` refuses before downloading anything.

```
sudo apt install -y php8.3-curl      # 8.3.6-0ubuntu0.24.04.10
```

After it:

```
$ php -m | grep -i curl
curl
$ php -r 'echo curl_version()["version"];'
8.5.0
```

`composer require jumbojett/openid-connect-php` then resolved on the first try —
4 installs, no platform-requirement complaints. **`ext-curl` was the only
blocker.** Nothing else about the toolchain needed touching.

## The login

`index.php` is 30 lines and does exactly this:

```php
$oidc = new OpenIDConnectClient('http://localhost:9400', 'spike-php', 'spike-secret');
$oidc->setRedirectURL('http://localhost:5001/');
$oidc->addScope(['openid', 'email', 'profile']);
$oidc->authenticate();
echo $oidc->requestUserInfo('email');
```

Browser output after clicking Ada on the provider's picker:

```
email: ada@example.test

name:  Ada Bell
sub:   ada
```

The library performed real discovery, a real code exchange, and real JWKS
signature validation against a plain-HTTP issuer without complaint.

`client_id=spike-php` was never registered anywhere. The provider accepted it and
went straight to the picker — the same "accept everything" property lanyard is
built around (north star 1), confirmed working end to end from PHP.

## Was anything beyond defaults needed?

**No.** Specifically:

| Setting | Verdict |
|---|---|
| `setHttpUpgradeInsecureRequests(false)` | **Not needed.** Removed it, re-ran the login, and it completed unchanged: `email: ada@example.test`. CONCEPT §10 mentions this as jumbojett's escape hatch "for exactly this" — for v1.0.2 against an `http://` issuer with an explicitly-set redirect URL, it is not required. Keep it in mind for an app that lets the library *derive* its own redirect URL, which is the case the flag actually guards. |
| `setVerifyHost` / `setVerifyPeer` | Never touched. Irrelevant over HTTP. |
| `providerConfigParam` overrides | Never touched. Discovery worked. |
| PKCE | Not configured, not needed — the provider accepted the plain code exchange with `client_secret`. |

## PHP does not have .NET's origin constraint

The .NET leg found that its login breaks entirely on a non-localhost HTTP origin,
because its correlation cookie is marked `Secure`
([dotnet-http-settings.md](dotnet-http-settings.md)). The same test on PHP:

Same app, same provider, only the app's origin moved from `http://localhost:5001`
to `http://web.localtest.me:5001` — **the login completed normally**,
`email: ada@example.test`.

The reason is visible in the cookie jar. PHP's session cookie:

```
PHPSESSID   path=/   sameSite=Lax   secure=false   httpOnly=false
```

No `Secure` flag, so nothing for the browser to reject on a plain-HTTP origin.
jumbojett keeps its `state` and `nonce` in `$_SESSION` behind that one ordinary
cookie rather than in dedicated `Secure` correlation cookies the way .NET does.

**PHP will not be what forces HTTPS onto the roadmap.** That confirms CONCEPT
§10's read and localises the entire HTTPS question to .NET.
