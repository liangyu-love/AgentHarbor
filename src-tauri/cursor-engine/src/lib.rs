//! Serves Cursor's Agent protocol on this machine so Cursor (the IDE and
//! `cursor-agent`) can run its Agent on models AgentHarbor routes.
//!
//! The Cursor protocol adapter, conversation runtime and Agent loop are
//! derived from cursor-byok (https://github.com/leookun/cursor-byok, MIT,
//! Copyright (c) 2026 leookun). Model calls go to AgentHarbor's router, which
//! reaches the relays, account sources and aggregates.
pub mod access;
pub mod api;
pub mod config;
pub mod cursor;
pub mod engine;
pub mod error;
pub mod host;
pub mod model;
pub mod network;
pub mod provider;
pub mod run;
pub mod search;
pub mod store;

pub use engine::{Engine, EngineConfig};
pub use error::{Error, Result};
pub use host::{CommitLocale, CommitSettings, Egress, EngineSettings, Host, Searcher, TabMode};
