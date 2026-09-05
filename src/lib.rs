//! lanyard — a local OIDC provider with nothing to configure.
//!
//! The binary is a thin shell over this library so integration tests can drive
//! the router directly on an ephemeral port.

pub mod app;
pub mod b64;
pub mod banner;
pub mod client;
pub mod config;
pub mod keys;
pub mod oidc;
pub mod persona;
pub mod seam;
pub mod session;
pub mod store;
pub mod ui;
