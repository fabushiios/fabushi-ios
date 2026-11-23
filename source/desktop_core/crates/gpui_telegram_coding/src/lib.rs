//! Local Coding Agent Host runtime facade.
//!
//! Provides local CLI probing, Git worktree isolation, non-invasive turn checkpoints,
//! persistent SQLite WAL session storage, and ACP / Claude Code / Codex CLI drivers.

pub mod driver;
pub mod git;
pub mod probe;
pub mod runtime;
pub mod store;

pub use probe::{
    probe_in_dirs, probe_local_coding_providers, probe_local_coding_providers_sync,
};
pub use runtime::{CodingHostRuntimeConfig, LocalCodingHostRuntime};
