# crates.io — reserve `lanyard-cli`?

**Decision: skip.** Do not reserve now. Revisit only if crates.io becomes a real
distribution channel, which CONCEPT §7 says it is not.

Checked 2026-09-04.

## Observed

`lanyard-cli` — available:

```
$ curl -s -w "\nHTTP %{http_code}\n" https://crates.io/api/v1/crates/lanyard-cli
{"errors":[{"detail":"crate `lanyard-cli` does not exist"}]}
HTTP 404
```

`lanyard` — taken, as CONCEPT §14 recorded:

```
$ curl -s https://crates.io/api/v1/crates/lanyard
name         lanyard
desc         UTF-8 C string types
max_version  0.1.3
repo         https://github.com/Speak2Erase/fmod-oxide
downloads    5530
updated      2024-06-11T02:50:59.251171Z
```

An FFI string-types crate vendored out of a FMOD binding project. 5,530 downloads
and untouched since June 2024. Nothing anyone would confuse with an identity
provider, and no sign of a rename or a squat worth contesting.

## Why skip

- **crates.io is not a channel for this tool.** CONCEPT §7 is explicit: the
  audience is PHP and .NET developers, and the channels that matter are GitHub
  release binaries, a Homebrew tap, a container image, and a curl installer.
  `cargo install` reaches a small slice of the audience.
- **The name that matters is already ours to pick.** The binary is `lanyard`
  regardless of the crate name; `fd`/`fd-find` and `delta`/`git-delta` are the
  well-worn version of this. Nobody reads the crate name after the install line.
- **A reservation is a real obligation.** Publishing a placeholder to hold a name
  means an empty crate in search results, and crates.io asks people not to squat.
  Publishing something functional means maintaining a published version from now
  on — which is a Phase 10 concern at the earliest.
- **The risk it defends against is small.** `lanyard-cli` is an odd name to land
  on independently, and if it were ever taken, `lanyard-server`, `lanyard-idp`,
  or `lanyard-oidc` all work. Crate name and binary name are independent, so
  losing the first choice costs one line of `Cargo.toml`.

## When to revisit

If the shared skeleton crate from CONCEPT §16 ever gets extracted, *that* one
genuinely belongs on crates.io — its consumers are Rust developers. Pick and
reserve a name at that point, as part of publishing something real.
