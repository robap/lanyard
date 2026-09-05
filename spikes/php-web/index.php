<?php
// Phase 0 asked: does jumbojett/openid-connect-php complete a real login against
// an http:// issuer with nothing but defaults? Phase 4 repointed it at lanyard,
// where `requestUserInfo()` is what proves /oidc/userinfo answers a real client
// rather than a curl. Deliberately minimal — every line that is here had to be.
//
//   composer install
//   php -S localhost:5001 index.php

require __DIR__ . '/vendor/autoload.php';

use Jumbojett\OpenIDConnectClient;

session_start();

$oidc = new OpenIDConnectClient(
    'http://127.0.0.1:9500/oidc',  // lanyard's issuer, plain HTTP on purpose
    'spike-php',                   // any client id, no registration
    'spike-secret'                 // any secret, checked by nobody
);
$oidc->setRedirectURL(getenv('BASE_URL') ?: 'http://localhost:5001/');
$oidc->addScope(['openid', 'email', 'profile']);

// Anything switched on by DROP= is a setting we are testing the need for, so its
// absence is observable rather than assumed.
$drop = array_filter(array_map('trim', explode(',', getenv('DROP') ?: '')));
if (!in_array('HttpUpgradeInsecureRequests', $drop, true)) {
    $oidc->setHttpUpgradeInsecureRequests(false);
}

/**
 * A deliberately plain page shell, kept close to `dotnet-web`'s so the two apps
 * look like the same app. Unifying them properly — one template, every stack —
 * is roadmap Phase 11.
 */
function page(string $title, string $body): string
{
    return <<<HTML
    <!doctype html>
    <html lang="en">
    <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{$title}</title>
    <style>
      :root { color-scheme: light dark; }
      body { font: 15px/1.6 ui-sans-serif, system-ui, sans-serif;
             max-width: 36rem; margin: 4rem auto; padding: 0 1rem; }
      h1 { font-size: 1.3rem; margin: 0 0 1.5rem; }
      .state { font-size: 1.05rem; }
      .button { display: inline-block; padding: .55rem 1rem; margin: .25rem .4rem .25rem 0;
                border-radius: 7px; background: #2f5bd7; color: #fff;
                text-decoration: none; font-weight: 600; }
      .button.secondary { background: transparent; color: inherit;
                          border: 1px solid currentColor; font-weight: 400; }
      .note { color: #666; font-size: .875rem; margin-top: 2rem; }
      pre { background: rgba(127,127,127,.12); padding: 1rem; border-radius: 8px;
            overflow-x: auto; }
      code { font-family: ui-monospace, monospace; font-size: .875em; }
    </style>
    </head>
    <body>
    <h1>{$title}</h1>
    {$body}
    </body>
    </html>
    HTML;
}

function e(?string $raw): string
{
    return htmlspecialchars($raw ?? '', ENT_QUOTES, 'UTF-8');
}

$action     = $_GET['action'] ?? null;
$isCallback = isset($_GET['code']) || isset($_GET['error']);

if ($action === 'logout') {
    // This app's session only. lanyard still remembers the persona, so signing
    // in again will not show the picker — full logout needs /oidc/end_session,
    // which is roadmap Phase 5.
    $_SESSION = [];
    session_destroy();
    header('Location: /');
    exit;
}

// **`authenticate()` only runs when you asked for it.** It is both the start of
// the flow and the callback handler, so calling it on every page load — which is
// what this spike used to do — means the front door is an instant redirect and
// the whole flow is invisible. Now the front door is a page with a button.
if ($action === 'login' || $isCallback) {
    try {
        $oidc->authenticate();
        // Three calls to GET /oidc/userinfo with a bearer token, server to
        // server. This is the thing this spike is evidence of.
        $_SESSION['user'] = [
            'email' => $oidc->requestUserInfo('email'),
            'name'  => $oidc->requestUserInfo('name'),
            'sub'   => $oidc->requestUserInfo('sub'),
        ];
        // Redirect so the address bar loses ?code=… and a reload does not try to
        // redeem a spent code.
        header('Location: /');
        exit;
    } catch (Throwable $e) {
        http_response_code(500);
        echo page('php-web', sprintf(
            '<p class="state">Login failed.</p><pre>%s%s%s</pre>'
                . '<p><a class="button" href="/">Start over</a></p>',
            e(get_class($e)),
            "\n\n",
            e($e->getMessage())
        ));
        exit;
    }
}

$user = $_SESSION['user'] ?? null;

if ($user) {
    $claims = sprintf(
        "email: %s\n\nname:  %s\nsub:   %s",
        e($user['email']),
        e($user['name']),
        e($user['sub'])
    );
    echo page('php-web', sprintf(
        '<p class="state">Signed in as <strong>%s</strong>.</p>'
            . '<pre>%s</pre>'
            . '<p><a class="button secondary" href="/?action=logout">Log out</a></p>'
            . '<p class="note">Those three lines came from three '
            . '<code>requestUserInfo()</code> calls — <code>GET /oidc/userinfo</code> '
            . 'with a bearer token, server to server.</p>',
        e($user['email'] ?: $user['sub']),
        $claims
    ));
    exit;
}

echo page('php-web', '<p class="state">Not signed in.</p>'
    . '<p><a class="button" href="/?action=login">Log in with lanyard</a></p>'
    . '<p class="note">That link calls <code>$oidc-&gt;authenticate()</code>, which '
    . 'redirects to lanyard, shows you a list of people, and comes back here.</p>');
