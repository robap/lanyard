<?php
// Phase 0 asked: does jumbojett/openid-connect-php complete a real login against
// an http:// issuer with nothing but defaults? Phase 4 repointed it at lanyard,
// where `requestUserInfo()` is what proves /oidc/userinfo answers a real client.
// Phase 5 added the log-out, which is `$oidc->signOut(...)` and nothing else.
// Deliberately minimal — every line that is here had to be.
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

function e(?string $raw): string
{
    return htmlspecialchars($raw ?? '', ENT_QUOTES, 'UTF-8');
}

/**
 * **One page, three stacks.** `spikes/shared/page.html` is read at runtime, not
 * copied and not generated, so this application and `dotnet-web` cannot render
 * differently by accident — and every visible difference between them is a
 * difference in the stack, which is the question these spikes exist to answer.
 */
function page(string $app, bool $signedIn, string $who, array $claims): string
{
    $rows = '';
    foreach ($claims as $name => $value) {
        $rows .= '<tr><td class="k">' . e($name) . '</td>'
            . '<td class="v">' . e(is_scalar($value) ? (string) $value : json_encode($value))
            . '</td></tr>';
    }

    return strtr(
        file_get_contents(__DIR__ . '/../shared/page.html'),
        [
            '{{APP}}'        => e($app),
            '{{WHO}}'        => e($who),
            '{{SIGNED_IN}}'  => $signedIn ? '' : 'hidden',
            '{{SIGNED_OUT}}' => $signedIn ? 'hidden' : '',
            '{{CLAIM_ROWS}}' => $rows,
        ]
    );
}

$path       = parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH) ?: '/';
$isCallback = isset($_GET['code']) || isset($_GET['error']);

// **The real log-out, and it is one line of ours.** `signOut()` reads
// `end_session_endpoint` out of lanyard's discovery document, sends the ID token
// as `id_token_hint`, redirects, and exits — so this application's session has
// to go first. The second argument is the `post_logout_redirect_uri`, which is
// this app's own root: loopback, and therefore accepted.
if ($path === '/logout') {
    $idToken  = $_SESSION['id_token'] ?? null;
    $_SESSION = [];
    session_destroy();
    $oidc->signOut($idToken, getenv('BASE_URL') ?: 'http://localhost:5001/');
    exit;
}

// This app's session only. lanyard still remembers the persona, so signing in
// again signs you straight back in as the same person — which is the difference
// the two buttons exist to show.
if ($path === '/logout-local') {
    $_SESSION = [];
    session_destroy();
    header('Location: /');
    exit;
}

// **`authenticate()` only runs when you asked for it.** It is both the start of
// the flow and the callback handler, so calling it on every page load — which is
// what this spike used to do — means the front door is an instant redirect and
// the whole flow is invisible. Now the front door is a page with a button.
if ($path === '/secure' || $isCallback) {
    try {
        $oidc->authenticate();
        // Kept so the log-out above has an `id_token_hint` to send. lanyard
        // never requires one, but a real IdP might, and a spike that skipped it
        // would be testing a shape production does not have.
        $_SESSION['id_token'] = $oidc->getIdToken();
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
        echo page('php-web', false, '', [
            get_class($e) => $e->getMessage(),
        ]);
        exit;
    }
}

$user = $_SESSION['user'] ?? null;

if ($user) {
    echo page('php-web', true, $user['email'] ?: $user['sub'], $user);
    exit;
}

echo page('php-web', false, '', []);
