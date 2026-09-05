//! The OIDC side of lanyard: the protocol surface, and the one function that
//! turns a persona plus overrides into a signed token.

pub mod issue;
pub mod jws;
pub mod routes;
pub mod token;
