//! A client's copy of a base: the state the server confirmed, plus the client's own batches
//! not yet answered. What the client shows is the confirmed state with its pending batches
//! applied on top — re-applied after every change from the server, which is the rebase.
//! When every pending batch is answered, the copy equals the server's state.

use crate::error::OpsError;
use crate::op::{Batch, ClientId, ClientOpId, Op, Outcome};
use crate::state::BaseState;
use std::collections::VecDeque;

/// A client's copy of a base.
#[derive(Debug, Clone)]
pub struct Replica {
    client: ClientId,
    next: u64,
    confirmed: BaseState,
    pending: VecDeque<Batch>,
}

impl Replica {
    /// Starts from the server's state (a snapshot at a sequence). `last` is the client's
    /// last batch number, so numbers keep increasing across reconnects.
    #[must_use]
    pub fn new(client: ClientId, last: u64, confirmed: BaseState) -> Self {
        Replica {
            client,
            next: last,
            confirmed,
            pending: VecDeque::new(),
        }
    }

    /// The client.
    #[must_use]
    pub const fn client(&self) -> ClientId {
        self.client
    }

    /// The state the server confirmed.
    #[must_use]
    pub const fn confirmed(&self) -> &BaseState {
        &self.confirmed
    }

    /// The batches sent and not yet answered, oldest first.
    pub fn pending(&self) -> impl Iterator<Item = &Batch> {
        self.pending.iter()
    }

    /// Makes a batch of the client's operations, keeps it as pending, and returns it to
    /// send.
    ///
    /// # Errors
    /// [`OpsError::Limit`] when the client has used every batch number.
    pub fn submit(&mut self, ops: Vec<Op>) -> Result<Batch, OpsError> {
        let seq = self
            .next
            .checked_add(1)
            .ok_or(OpsError::Limit("batch numbers exhausted"))?;
        self.next = seq;
        let batch = Batch {
            id: ClientOpId {
                client: self.client,
                seq,
            },
            base: self.confirmed.base(),
            ops,
        };
        self.pending.push_back(batch.clone());
        Ok(batch)
    }

    /// Takes in what the server sent: every accepted batch of the base, in order (the
    /// client's own and everyone else's), and the refusals of the client's own batches.
    /// An accepted batch already applied is ignored (a repeat after a reconnect).
    ///
    /// # Errors
    /// [`OpsError::Gap`] when an accepted batch was missed: the client must catch up from
    /// the server's journal or a new snapshot. [`OpsError::Replay`] when the server's
    /// change does not fit (a bug on one side).
    pub fn receive(&mut self, outcome: &Outcome) -> Result<(), OpsError> {
        match outcome {
            Outcome::Accepted(accepted) => {
                if accepted.seq > self.confirmed.seq() {
                    self.confirmed.replay(accepted)?;
                }
                self.answered(accepted.id);
            }
            Outcome::Rejected(rejection) => self.answered(rejection.id),
        }
        Ok(())
    }

    fn answered(&mut self, id: ClientOpId) {
        self.pending.retain(|batch| batch.id != id);
    }

    /// What the client shows: the confirmed state with the pending batches applied on top
    /// (those that no longer apply are left out; the server will refuse them).
    #[must_use]
    pub fn view(&self) -> BaseState {
        let mut view = self.confirmed.clone();
        for batch in &self.pending {
            view.apply_local(batch);
        }
        view
    }
}
