//! The OIDC side of lanyard: the protocol surface, and the one function that
//! turns a persona plus overrides into a signed token.

pub mod authorize;
pub mod code;
pub mod cors;
pub mod flaw;
pub mod issue;
pub mod jws;
pub mod redirect_uri;
pub mod routes;
pub mod scope;
pub mod token;
pub mod userinfo;
