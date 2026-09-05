# dotnet-api — the acceptance harness

A resource server that validates lanyard's tokens with stock `AddJwtBearer`,
pointed at the discovery document and nothing else. It exists to be *observed
against*: Phase 2 used it to answer "does a real .NET API accept a lanyard
token, and does it stop accepting one after 60 seconds", and Phase 3 aims its
six failure flags at this same `/orders`.

This is a harness, not an example. `examples/dotnet-api/` — published, CI-run,
deliberately boring — is a different artifact and is post-v1.

```
lanyard serve &                        # must be on 127.0.0.1:9500
dotnet run                             # ClockSkew = Zero
CLOCK_SKEW=default dotnet run          # .NET's real default: five minutes
DROP=Audience dotnet run               # omit a setting to observe its absence

curl -s -o /dev/null -w '%{http_code}\n' \
  -H "Authorization: Bearer $(lanyard token --as ada --aud billing-api)" \
  http://127.0.0.1:5080/orders
```

Two knobs, both in the subtraction style of the Phase 0 spike:

- **`DROP=Name,Name`** omits an `AddJwtBearer` setting so its absence can be
  observed without a rebuild. A setting nobody removed is not evidence of a
  minimum.
- **`CLOCK_SKEW=zero|default`** picks the token-lifetime tolerance. `zero` is
  this harness's default and *not* .NET's; see
  [`docs/decisions/dotnet-jwt-bearer-settings.md`](../../docs/decisions/dotnet-jwt-bearer-settings.md).

Startup prints `DISCOVERY OK:` with what the handler's own configuration manager
resolved, so whether a stock RP accepts lanyard's document is answered in the log
rather than inferred.
