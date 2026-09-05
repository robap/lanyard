// Verify a lanyard token the way a real relying party would: against the live
// JWKS URL over HTTP, not a locally pasted key.
//
//   node jose-verify.mjs <token> <issuer> [audience]
//
// Prints the verified payload as JSON on success, or `ERROR <name>: <message>`
// and exits 1 on failure. North star 4 — tokens are checked by `jose`, not by
// our own code agreeing with itself.

import { createRemoteJWKSet, jwtVerify } from 'jose'

const [token, issuer, audience] = process.argv.slice(2)

if (!token || !issuer) {
  console.error('usage: node jose-verify.mjs <token> <issuer> [audience]')
  process.exit(2)
}

// Discovery is the RP's entry point, so resolve jwks_uri from the document
// rather than assuming its shape.
const discoveryUrl = `${issuer.replace(/\/$/, '')}/.well-known/openid-configuration`

try {
  const res = await fetch(discoveryUrl)
  if (!res.ok) throw new Error(`discovery returned ${res.status}`)
  const doc = await res.json()

  const jwks = createRemoteJWKSet(new URL(doc.jwks_uri))
  const options = { issuer: doc.issuer }
  if (audience) options.audience = audience

  const { payload } = await jwtVerify(token, jwks, options)
  console.log(JSON.stringify(payload, null, 2))
} catch (err) {
  console.error(`ERROR ${err.name}: ${err.message}`)
  process.exit(1)
}
