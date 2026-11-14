//! Lifecycle state categories.
//!
//! Customers name, add and remove lifecycle states freely, but every state belongs to exactly
//! one [`StateCategory`]. SLA clocks, dashboards, the loop-health check and AI reason about
//! categories, never about state names, so they work on any template.

use crate::error::{clip, CoreError};
use crate::field::string_enum;
use core::fmt;
use core::str::FromStr;

string_enum! {
    /// The category of a lifecycle state.
    StateCategory, "state category";
    /// Created, nobody has started yet.
    New => "new",
    /// Being worked on.
    InProgress => "in_progress",
    /// Waiting for someone inside the organisation.
    WaitingInternal => "waiting_internal",
    /// Waiting for the requester or a third party; SLA clocks pause here.
    WaitingExternal => "waiting_external",
    /// Waiting for an approval decision.
    PendingApproval => "pending_approval",
    /// Finished successfully (terminal).
    DoneSuccess => "done_success",
    /// Finished unsuccessfully (terminal).
    DoneFailure => "done_failure",
    /// Stopped before completion (terminal).
    Cancelled => "cancelled",
}

impl StateCategory {
    /// Terminal categories end the process instance.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            StateCategory::DoneSuccess | StateCategory::DoneFailure | StateCategory::Cancelled
        )
    }

    /// Open (non-terminal) categories still need someone to act.
    #[must_use]
    pub const fn is_open(self) -> bool {
        !self.is_terminal()
    }

    /// Whether SLA clocks pause while a record is in a state of this category.
    #[must_use]
    pub const fn pauses_sla(self) -> bool {
        matches!(self, StateCategory::WaitingExternal)
    }

    /// Whether a record in this category must have an owner (every open category does).
    #[must_use]
    pub const fn requires_owner(self) -> bool {
        self.is_open()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std_categories_partition_into_open_and_terminal() {
        let terminal: Vec<_> = StateCategory::ALL
            .iter()
            .filter(|c| c.is_terminal())
            .collect();
        assert_eq!(
            terminal,
            [
                &StateCategory::DoneSuccess,
                &StateCategory::DoneFailure,
                &StateCategory::Cancelled
            ]
        );
        for category in StateCategory::ALL {
            assert_ne!(category.is_open(), category.is_terminal());
            assert_eq!(category.requires_owner(), category.is_open());
            assert_eq!(
                category.pauses_sla(),
                *category == StateCategory::WaitingExternal
            );
        }
    }

    #[test]
    fn std_text_and_serde_round_trip() {
        for category in StateCategory::ALL {
            assert_eq!(
                category.to_string().parse::<StateCategory>().unwrap(),
                *category
            );
            let json = serde_json::to_string(category).unwrap();
            assert_eq!(
                serde_json::from_str::<StateCategory>(&json).unwrap(),
                *category
            );
        }
        assert_eq!(StateCategory::ALL.len(), 8);
    }

    #[test]
    fn invalid_unknown_category() {
        assert!(matches!(
            "paused".parse::<StateCategory>(),
            Err(CoreError::UnknownCode {
                kind: "state category",
                ..
            })
        ));
    }
}
