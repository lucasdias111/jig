//! What Jig's AI decides, in plain Rust: the agent's types and the words
//! it's sent, which shell commands only read, the tasks and the Map's
//! tools, the providers' config, and what OpenCode's and ACP agents'
//! messages say. Nothing here starts a process or touches the network.

pub mod acp;
pub mod agent;
pub mod config;
pub mod config_file;
pub mod map;
pub mod opencode;
pub mod read_only;
pub mod tasks;
