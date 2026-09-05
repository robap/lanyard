// The Phase 2 acceptance client: a resource server that validates lanyard's
// tokens with stock `AddJwtBearer`, pointed at the discovery document and
// nothing else. It exists to be observed against — Phase 3 aims its six failure
// flags at this same `/orders`.
//
//   dotnet run                     # ClockSkew = Zero, the honest default here
//   CLOCK_SKEW=default dotnet run  # .NET's real default: five minutes
//   DROP=Audience dotnet run       # omit a setting to observe its absence

using Microsoft.AspNetCore.Authentication.JwtBearer;
using Microsoft.Extensions.Options;

var builder = WebApplication.CreateBuilder(args);
builder.WebHost.UseUrls("http://127.0.0.1:5080");

// Subtraction harness, as in the Phase 0 spike: a setting nobody removed is not
// evidence of a minimum.
var dropped = (Environment.GetEnvironmentVariable("DROP") ?? "")
    .Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
    .ToHashSet(StringComparer.OrdinalIgnoreCase);
bool Keep(string name) => !dropped.Contains(name);

// `zero` is this harness's default, not .NET's. .NET's own default is five
// minutes, which would accept a 60-second lanyard token for roughly six — and
// would make the "90 seconds later returns 401" criterion quietly pass for the
// wrong reason. Both numbers get observed from this one build.
var clockSkew = (Environment.GetEnvironmentVariable("CLOCK_SKEW") ?? "zero").Trim().ToLowerInvariant();

Console.WriteLine($"DROPPED: {(dropped.Count == 0 ? "(nothing)" : string.Join(",", dropped))}");
Console.WriteLine($"CLOCK_SKEW: {clockSkew}");

builder.Services
    .AddAuthentication(JwtBearerDefaults.AuthenticationScheme)
    .AddJwtBearer(options =>
    {
        if (Keep("Authority")) options.Authority = "http://127.0.0.1:9500/oidc";
        if (Keep("RequireHttpsMetadata")) options.RequireHttpsMetadata = false;
        if (Keep("Audience")) options.Audience = "billing-api";
        if (clockSkew == "zero") options.TokenValidationParameters.ClockSkew = TimeSpan.Zero;

        // Diagnostics only, not auth settings: the reason a token was refused is
        // the whole output of this harness.
        options.Events = new JwtBearerEvents
        {
            OnTokenValidated = context =>
            {
                // .NET remaps `sub` onto ClaimTypes.NameIdentifier unless
                // MapInboundClaims is turned off, so look under both names.
                var user = context.Principal;
                var sub = user?.FindFirst("sub")?.Value
                          ?? user?.FindFirst(System.Security.Claims.ClaimTypes.NameIdentifier)?.Value;
                Console.WriteLine($"TOKEN OK: sub={sub}");
                return Task.CompletedTask;
            },
            OnAuthenticationFailed = context =>
            {
                Console.WriteLine($"TOKEN REFUSED: {context.Exception.GetType().Name}: {context.Exception.Message}");
                return Task.CompletedTask;
            },
        };
    });

builder.Services.AddAuthorization();

var app = builder.Build();

app.UseAuthentication();
app.UseAuthorization();

app.MapGet("/orders", () => Results.Json(new[] { new { id = 1, total = 42.00 } }))
   .RequireAuthorization();

// Consume the discovery document through the handler's *own* configuration
// manager before serving, so "did a stock RP accept a document with no
// authorization_endpoint?" is answered in this log rather than inferred.
var jwtOptions = app.Services
    .GetRequiredService<IOptionsMonitor<JwtBearerOptions>>()
    .Get(JwtBearerDefaults.AuthenticationScheme);
try
{
    var config = await jwtOptions.ConfigurationManager!.GetConfigurationAsync(CancellationToken.None);
    Console.WriteLine(
        $"DISCOVERY OK: issuer={config.Issuer} jwks_uri={config.JwksUri} " +
        $"signing_keys={config.SigningKeys.Count} " +
        $"authorization_endpoint={(string.IsNullOrEmpty(config.AuthorizationEndpoint) ? "(absent)" : config.AuthorizationEndpoint)} " +
        $"token_endpoint={(string.IsNullOrEmpty(config.TokenEndpoint) ? "(absent)" : config.TokenEndpoint)}");
}
catch (Exception e)
{
    Console.WriteLine($"DISCOVERY FAILED: {e.GetType().Name}: {e.Message}");
}

app.Run();
