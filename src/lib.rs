//! MCP server for the [VinylDNS](https://www.vinyldns.io) DNS management API.
//!
//! The crate is a binary first; the library target exists so integration
//! tests can drive the server in-process.

pub mod client;
pub mod config;
pub mod dns;
pub mod pending;
pub mod server;
pub mod signer;
