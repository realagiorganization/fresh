//! SSH remote editing support
//!
//! This module provides remote file system access and process execution
//! via an SSH connection to a Python agent running on the remote host.

mod channel;
mod connection;
mod filesystem;
mod protocol;
mod spawner;

pub use channel::AgentChannel;
pub use connection::{ConnectionParams, SshConnection};
pub use filesystem::RemoteFileSystem;
pub use protocol::{
    decode_base64, encode_base64, ls_params, read_params, stat_params, write_params, AgentRequest,
    AgentResponse,
};
pub use spawner::{
    LocalProcessSpawner, ProcessSpawner, RemoteProcessSpawner, SpawnError, SpawnResult,
};

/// The Python agent source code, embedded at compile time.
pub const AGENT_SOURCE: &str = include_str!("agent.py");

#[cfg(test)]
mod tests;
