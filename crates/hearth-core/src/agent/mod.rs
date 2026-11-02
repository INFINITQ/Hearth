//! The coding agent: tools that read and change a project, the permission rules around them,
//! and the loop that lets a model use them.

pub mod matcher;
pub mod paths;
pub mod permissions;
pub mod prompt;
pub mod runner;
pub mod text;
pub mod tools;

pub use permissions::PermissionMode;
pub use runner::{Agent, Decision, PermissionReply};
