//! lanyard — a local OIDC provider with nothing to configure.
//!
//! The binary is a thin shell over this library so integration tests can drive
//! the router directly on an ephemeral port.

pub mod api;
pub mod app;
pub mod b64;
pub mod banner;
pub mod client;
pub mod config;
pub mod embed;
pub mod events;
pub mod keys;
pub mod links;
pub mod log_detail;
pub mod log_layer;
pub mod oidc;
pub mod persona;
pub mod registry;
pub mod seam;
pub mod session;
pub mod store;
pub mod ui;
