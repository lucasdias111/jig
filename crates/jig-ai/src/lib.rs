//! The AI layer: providers' config and keys, the coding agents, and the
//! task planner's tasks.
//!
//! Calls are blocking; the app runs them on a background thread.

pub mod agent;
pub mod config;
pub mod config_file;
mod http;
pub mod keys;
mod models;
pub mod tasks;

pub use config::{AgentModel, Config, ProviderConfig, ProviderKind, split_model};
pub use config_file::{ConfigFile, ProviderTemplate, TEMPLATES, template_named};
pub use keys::{KeySource, KeyStore};
