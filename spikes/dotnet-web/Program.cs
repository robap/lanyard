using System.Net;
using System.Security.Claims;
using Microsoft.AspNetCore.Authentication.Cookies;
using Microsoft.AspNetCore.Authentication.OpenIdConnect;
using Microsoft.AspNetCore.Authentication;
using Microsoft.AspNetCore.Authorization;

var builder = WebApplication.CreateBuilder(args);

// Subtraction harness: DROP=Name,Name omits a setting so its absence can be observed
// without a rebuild. A setting nobody removed is not evidence of a minimum.
var dropped = (Environment.GetEnvironmentVariable("DROP") ?? "")
    .Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
    .ToHashSet(StringComparer.OrdinalIgnoreCase);
bool Keep(string name) => !dropped.Contains(name);
Console.WriteLine($"DROPPED: {(dropped.Count == 0 ? "(nothing)" : string.Join(",", dropped))}");

builder.Services
    .AddAuthentication(options =>
    {
        options.DefaultScheme = CookieAuthenticationDefaults.AuthenticationScheme;
        options.DefaultChallengeScheme = OpenIdConnectDefaults.AuthenticationScheme;
    })
    .AddCookie()
    .AddOpenIdConnect(options =>
    {
        // Phase 4 repointed this spike from Phase 0's `oidc-provider-mock` at
        // :9400 to lanyard itself. The DROP= subtraction harness is unchanged —
        // a setting nobody removed is still not evidence of a minimum.
        if (Keep("Authority")) options.Authority = "http://127.0.0.1:9500/oidc";
        if (Keep("RequireHttpsMetadata")) options.RequireHttpsMetadata = false;
        // Nothing was registered with lanyard. This client id was invented here
        // and lanyard has never heard of it.
        if (Keep("ClientId")) options.ClientId = "billing-web";
        if (Keep("ClientSecret")) options.ClientSecret = "unchecked";
        if (Keep("ResponseType")) options.ResponseType = "code";
        // Also what makes `id_token_hint` reach `/oidc/end_session`: the handler
        // can only send a hint it kept.
        if (Keep("SaveTokens")) options.SaveTokens = true;
        if (Keep("ScopeEmail")) options.Scope.Add("email");
        if (Keep("ScopeProfile")) options.Scope.Add("profile");
    });

builder.Services.AddAuthorization();

var app = builder.Build();

// Diagnostic only, not an auth setting: record every Set-Cookie this app emits, so
// the SameSite / Secure attributes on the correlation and nonce cookies are observed
// rather than assumed. Written to stdout for the spike log.
app.Use(async (context, next) =>
{
    context.Response.OnStarting(() =>
    {
        foreach (var value in context.Response.Headers.SetCookie)
        {
            Console.WriteLine($"SET-COOKIE {context.Request.Path} :: {value}");
        }
        return Task.CompletedTask;
    });
    await next();
});

app.UseAuthentication();
app.UseAuthorization();

// Read on every request rather than cached, so editing the shared page and
// reloading the browser shows the edit — which is what makes it worth having
// one file rather than three.
var sharedPage = Path.Combine(app.Environment.ContentRootPath, "..", "shared", "page.html");

// **The landing page does not redirect.** Hitting a protected page and being
// bounced straight to the provider is what a real app does, and it is also what
// makes the flow impossible to follow the first time: the browser has already
// arrived somewhere else before you have read anything. So the front door is a
// page with a button on it, and the redirects start when you press it.
app.MapGet("/", (ClaimsPrincipal user) =>
{
    var signedIn = user.Identity?.IsAuthenticated == true;
    var who = user.FindFirst("email")?.Value
              ?? user.FindFirst(ClaimTypes.Email)?.Value
              ?? user.FindFirst("sub")?.Value
              ?? user.FindFirst(ClaimTypes.NameIdentifier)?.Value
              ?? "somebody";

    var rows = signedIn
        ? string.Concat(user.Claims.Select(c =>
            $"<tr><td class=\"k\">{WebUtility.HtmlEncode(c.Type)}</td>"
            + $"<td class=\"v\">{WebUtility.HtmlEncode(c.Value)}</td></tr>"))
        : "";

    return Results.Content(Page("dotnet-web", signedIn, signedIn ? who : "", rows), "text/html");
});

// The login. `[Authorize]` with no principal is a 401, which the OpenID Connect
// handler turns into the redirect to lanyard — so this route *is* the login
// button's destination, and there is no lanyard URL anywhere in this file.
app.MapGet("/secure", [Authorize] () => Results.Redirect("/"));

// **The real log-out, and it is one line of ours.** `SignOutAsync` over both
// schemes: the cookie scheme drops this application's own cookie, and the OIDC
// scheme builds the redirect to lanyard's `end_session_endpoint` — a URL it
// reads out of the discovery document, adding `id_token_hint` because
// `SaveTokens` kept one and using its own `SignedOutCallbackPath` as the
// `post_logout_redirect_uri`. Nothing here names lanyard.
app.MapGet("/logout", async (HttpContext ctx) =>
{
    await ctx.SignOutAsync(CookieAuthenticationDefaults.AuthenticationScheme);
    await ctx.SignOutAsync(OpenIdConnectDefaults.AuthenticationScheme,
        new AuthenticationProperties { RedirectUri = "/" });
});

// Sign out of this app only, without touching lanyard's own session — which is
// how you observe that lanyard remembers the persona per client_id, and what
// makes the two-sessions problem visible rather than theoretical.
app.MapGet("/logout-local", async (HttpContext ctx) =>
{
    await ctx.SignOutAsync(CookieAuthenticationDefaults.AuthenticationScheme);
    return Results.Redirect("/");
});

app.Run();

// **One page, three stacks.** `spikes/shared/page.html` is read at runtime, not
// copied and not generated, so this application and `php-web` cannot render
// differently by accident — and every visible difference between them is a
// difference in the stack, which is the question these spikes exist to answer.
string Page(string app, bool signedIn, string who, string claimRows) =>
    File.ReadAllText(sharedPage)
        .Replace("{{APP}}", WebUtility.HtmlEncode(app))
        .Replace("{{WHO}}", WebUtility.HtmlEncode(who))
        .Replace("{{SIGNED_IN}}", signedIn ? "" : "hidden")
        .Replace("{{SIGNED_OUT}}", signedIn ? "hidden" : "")
        .Replace("{{CLAIM_ROWS}}", claimRows);
