//! SecAFS rollback snapshot subsystem.
//!
//! Per-volume Copy-on-Write rollback via PG triggers + typed undo tables.
//! See `docs/superpowers/specs/2026-04-27-secafs-chat-rollback-design.md`.

pub mod restore;
pub mod schema;
pub mod snapshots;
pub mod state;
pub mod triggers;
pub mod types;

pub use restore::{restore_to, RestoreOutcome};
pub use snapshots::{commit, list};
pub use state::{enable, disable, get_state, DisableResult};
pub use types::{SnapshotInfo, UndoOp, VolumeRollbackState};
