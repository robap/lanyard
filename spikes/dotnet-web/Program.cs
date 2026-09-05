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

    var body = signedIn
        ? $"""
            <p class="state">Signed in as <strong>{WebUtility.HtmlEncode(who)}</strong>.</p>
            <p>
              <a class="button" href="/secure">View my claims</a>
              <a class="button secondary" href="/logout">Log out</a>
            </p>
            <p class="note">
              "Log out" clears this app's cookie only — lanyard still remembers you,
              so signing in again will not show the picker. Full logout needs
              <code>/oidc/end_session</code>, which is roadmap Phase 5.
            </p>
          """
        : """
            <p class="state">Not signed in.</p>
            <p><a class="button" href="/secure">Log in with lanyard</a></p>
            <p class="note">
              That link goes to a page marked <code>[Authorize]</code>. ASP.NET Core
              turns the 401 into a redirect to lanyard, lanyard shows you a list of
              people, and you come back here signed in.
            </p>
          """;

    return Results.Content(Page("dotnet-web", body), "text/html");
});

// Sign out of this app only, without touching lanyard's own session — which is
// how you observe that lanyard remembers the persona per client_id.
app.MapGet("/logout", async (HttpContext ctx) =>
{
    await ctx.SignOutAsync(CookieAuthenticationDefaults.AuthenticationScheme);
    return Results.Redirect("/");
});

app.MapGet("/secure", [Authorize] (ClaimsPrincipal user) =>
{
    var email = user.FindFirst("email")?.Value
                ?? user.FindFirst(ClaimTypes.Email)?.Value
                ?? "(no email claim)";
    var claims = string.Join("\n", user.Claims.Select(c => $"  {c.Type} = {c.Value}"));
    return Results.Text($"email: {email}\n\nall claims:\n{claims}");
});

app.Run();

// A deliberately plain page shell, kept close to `php-web`'s so the two apps look
// like the same app. Unifying them properly — one template, every stack — is
// roadmap Phase 11.
static string Page(string title, string body) => $$"""
    <!doctype html>
    <html lang="en">
    <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{{title}}</title>
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
      code { font-family: ui-monospace, monospace; font-size: .875em; }
    </style>
    </head>
    <body>
    <h1>{{title}}</h1>
    {{body}}
    </body>
    </html>
    """;
