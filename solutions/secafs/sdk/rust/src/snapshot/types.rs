//! Public types for the snapshot subsystem.

use serde::{Deserialize, Serialize};

/// Per-volume rollback state row.
#[derive(Debug, Clone)]
pub struct VolumeRollbackState {
    pub volume_id: String,
    pub rollback_enabled: bool,
    pub current_snap_id: i64,
}

/// One committed snapshot row (label is the message id when written by the plugin).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub snap_id: i64,
    pub committed_at: chrono::DateTime<chrono::Utc>,
    pub label: Option<String>,
}

/// Inverse-op classifier for the typed undo tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoOp {
    Insert,
    Update,
    Delete,
}

impl UndoOp {
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'I' => Some(UndoOp::Insert),
            'U' => Some(UndoOp::Update),
            'D' => Some(UndoOp::Delete),
            _ => None,
        }
    }

    pub fn to_char(self) -> char {
        match self {
            UndoOp::Insert => 'I',
            UndoOp::Update => 'U',
            UndoOp::Delete => 'D',
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_op_round_trip() {
        for op in [UndoOp::Insert, UndoOp::Update, UndoOp::Delete] {
            assert_eq!(UndoOp::from_char(op.to_char()), Some(op));
        }
        assert_eq!(UndoOp::from_char('Z'), None);
    }
}
