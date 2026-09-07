//! lanyard — a local OIDC provider with nothing to configure.
//!
//! The binary is a thin shell over this library so integration tests can drive
//! the router directly on an ephemeral port.

pub mod api;
pub mod app;
pub mod b64;
pub mod banner;
pub mod client;
pub mod clock;
pub mod config;
pub mod doctor;
pub mod embed;
pub mod events;
pub mod health;
pub mod hosts;
pub mod keys;
pub mod links;
pub mod log_detail;
pub mod log_layer;
pub mod oidc;
pub mod persona;
pub mod registry;
pub mod runtime;
pub mod seam;
pub mod session;
pub mod store;
pub mod ui;
