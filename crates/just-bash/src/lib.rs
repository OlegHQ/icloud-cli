//! just-bash - A simulated bash environment
//!
//! This library provides a complete parser and interpreter for bash scripts.

// Vendored fork: upstream emits many rustc/clippy findings; keep workspace `clippy -D warnings` green.
#![allow(warnings)]
#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]

pub mod ast;
pub mod bash;
pub mod commands;
pub mod fs;
pub mod interpreter;
pub mod network;
pub mod parser;
#[cfg(feature = "sandbox")]
pub mod sandbox;
pub mod shell;

pub use ast::types::*;
pub use bash::Bash;
pub use commands::{Command, CommandContext, CommandResult};
pub use fs::{FileSystem, InMemoryFs};
pub use parser::{parse, ParseException, Parser};
#[cfg(feature = "sandbox")]
pub use sandbox::Sandbox;
