//! The versioned history of one reference set, with optional four-eyes approval.
//!
//! A [`Ledger`] is a small state machine. Its inputs are commits, proposals, approvals,
//! rejections and withdrawals; time and identities are inputs too (nothing here reads a
//! clock). Its guarantees:
//!
//! * the current state is always a valid set (every accepted change was verified);
//! * each accepted commit raises the version by exactly one and is recorded as a
//!   [`Revision`] with who made it, who approved it and when;
//! * replaying the revisions on the starting state reproduces the current state exactly
//!   ([`Ledger::restore`] verifies this), which is what makes "as of" browsing trustworthy;
//! * under [`ChangePolicy::Approval`] nothing changes without a second person: the proposer
//!   can neither approve nor reject their own proposal;
//! * a proposal is applied to the state *at approval time*. If the set changed meanwhile and
//!   the proposal no longer fits, the approval fails and the proposal stays pending, so it
//!   can be withdrawn or rejected; it never applies half-way.

use crate::change::{Change, State};
use crate::effective::EffectiveSet;
use crate::error::RefDataError;
use crate::model::{check_label, ChangePolicy};
use mt_core::id::PrincipalId;
use mt_core::time::Timestamp;
use serde::{Deserialize, Serialize};

/// Maximum number of changes in one commit or proposal.
pub const MAX_CHANGES_PER_COMMIT: usize = 10_000;
/// Maximum number of proposals waiting for a decision at the same time.
pub const MAX_PENDING_PROPOSALS: usize = 100;

/// One accepted set of changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    /// The version the set has after this revision (the first revision is version 1).
    pub version: u64,
    /// When it was accepted.
    pub at: Timestamp,
    /// Who made the changes.
    pub by: PrincipalId,
    /// Who approved them, if they went through a proposal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<PrincipalId>,
    /// The changes, in the order they were applied.
    pub changes: Vec<Change>,
}

/// Where a proposal stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProposalStatus {
    /// Waiting for a decision.
    Pending,
    /// Approved and applied.
    Approved {
        /// Who approved it.
        by: PrincipalId,
        /// When.
        at: Timestamp,
        /// The version it produced.
        version: u64,
    },
    /// Refused.
    Rejected {
        /// Who refused it.
        by: PrincipalId,
        /// When.
        at: Timestamp,
        /// Why.
        reason: String,
    },
    /// Taken back by its author.
    Withdrawn {
        /// When.
        at: Timestamp,
    },
}

/// Changes waiting for (or past) a second person's decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// Number of the proposal within the ledger, starting at 1.
    pub id: u64,
    /// The version of the set when the proposal was made.
    pub base_version: u64,
    /// Who proposed it.
    pub by: PrincipalId,
    /// When.
    pub at: Timestamp,
    /// The proposed changes.
    pub changes: Vec<Change>,
    /// The proposer's explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Where it stands.
    pub status: ProposalStatus,
}

/// The versioned history of one set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger {
    genesis: State,
    current: State,
    effective: EffectiveSet,
    revisions: Vec<Revision>,
    proposals: Vec<Proposal>,
}

fn check_changes(changes: &[Change]) -> Result<(), RefDataError> {
    if changes.is_empty() {
        return Err(RefDataError::EmptyChange);
    }
    if changes.len() > MAX_CHANGES_PER_COMMIT {
        return Err(RefDataError::Limit("too many changes in one commit"));
    }
    Ok(())
}

impl Ledger {
    /// Starts a ledger at version 0 with the given state.
    ///
    /// # Errors
    /// Whatever [`State::effective`] reports: the starting state must be a valid set.
    pub fn new(genesis: State) -> Result<Self, RefDataError> {
        let effective = genesis.effective()?;
        Ok(Ledger {
            current: genesis.clone(),
            genesis,
            effective,
            revisions: Vec::new(),
            proposals: Vec::new(),
        })
    }

    /// Rebuilds a ledger from what storage kept: the starting state, the revisions and the
    /// proposals. Every revision is replayed and verified, so a ledger that loads is a
    /// ledger whose history is consistent.
    ///
    /// # Errors
    /// [`RefDataError::UnknownVersion`] when revision numbers are not 1, 2, 3, …; any error
    /// of a revision that does not apply; [`RefDataError::Invalid`] for inconsistent
    /// proposals.
    pub fn restore(
        genesis: State,
        revisions: Vec<Revision>,
        proposals: Vec<Proposal>,
    ) -> Result<Self, RefDataError> {
        let mut ledger = Ledger::new(genesis)?;
        for revision in revisions {
            let expected = ledger.version().saturating_add(1);
            if revision.version != expected {
                return Err(RefDataError::UnknownVersion {
                    version: revision.version,
                });
            }
            check_changes(&revision.changes)?;
            let (next, effective) = ledger.current.apply(&revision.changes)?;
            ledger.current = next;
            ledger.effective = effective;
            ledger.revisions.push(revision);
        }
        for (index, proposal) in proposals.iter().enumerate() {
            let expected = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
            let applied_version = match &proposal.status {
                ProposalStatus::Approved { version, .. } => Some(*version),
                ProposalStatus::Pending
                | ProposalStatus::Rejected { .. }
                | ProposalStatus::Withdrawn { .. } => None,
            };
            let consistent = proposal.id == expected
                && proposal.base_version <= ledger.version()
                && applied_version
                    .is_none_or(|v| v > proposal.base_version && v <= ledger.version());
            if !consistent {
                return Err(RefDataError::invalid(
                    "proposal",
                    format!("proposal {} does not fit the history", proposal.id),
                ));
            }
        }
        ledger.proposals = proposals;
        Ok(ledger)
    }

    /// The current version (0 before the first change).
    #[must_use]
    pub fn version(&self) -> u64 {
        u64::try_from(self.revisions.len()).unwrap_or(u64::MAX)
    }

    /// The current stored state.
    #[must_use]
    pub const fn state(&self) -> &State {
        &self.current
    }

    /// The starting state (version 0).
    #[must_use]
    pub const fn genesis(&self) -> &State {
        &self.genesis
    }

    /// The current set, validated and indexed.
    #[must_use]
    pub const fn effective(&self) -> &EffectiveSet {
        &self.effective
    }

    /// Every accepted revision, oldest first.
    #[must_use]
    pub fn revisions(&self) -> &[Revision] {
        &self.revisions
    }

    /// Every proposal ever made, oldest first.
    #[must_use]
    pub fn proposals(&self) -> &[Proposal] {
        &self.proposals
    }

    /// The proposals still waiting for a decision.
    pub fn pending(&self) -> impl Iterator<Item = &Proposal> {
        self.proposals
            .iter()
            .filter(|p| p.status == ProposalStatus::Pending)
    }

    fn accept(
        &mut self,
        by: PrincipalId,
        approved_by: Option<PrincipalId>,
        at: Timestamp,
        changes: Vec<Change>,
    ) -> Result<u64, RefDataError> {
        let (next, effective) = self.current.apply(&changes)?;
        let version = self.version().saturating_add(1);
        self.current = next;
        self.effective = effective;
        self.revisions.push(Revision {
            version,
            at,
            by,
            approved_by,
            changes,
        });
        Ok(version)
    }

    /// Applies changes at once. Only for sets whose policy is [`ChangePolicy::Direct`].
    /// `expected_version` is the version the caller looked at; a mismatch means someone
    /// else changed the set first.
    ///
    /// # Errors
    /// [`RefDataError::ApprovalRequired`], [`RefDataError::VersionConflict`],
    /// [`RefDataError::EmptyChange`], [`RefDataError::Limit`], or the reason the changes do
    /// not apply. On error nothing changes.
    pub fn commit(
        &mut self,
        by: PrincipalId,
        at: Timestamp,
        expected_version: u64,
        changes: Vec<Change>,
    ) -> Result<u64, RefDataError> {
        if self.current.set.change_policy == ChangePolicy::Approval {
            return Err(RefDataError::ApprovalRequired);
        }
        check_changes(&changes)?;
        if expected_version != self.version() {
            return Err(RefDataError::VersionConflict {
                expected: expected_version,
                actual: self.version(),
            });
        }
        self.accept(by, None, at, changes)
    }

    /// Records changes for a second person to decide on. The changes must apply to the
    /// current state (a proposal that is wrong today is refused today). Returns the
    /// proposal number.
    ///
    /// # Errors
    /// [`RefDataError::EmptyChange`], [`RefDataError::Limit`], [`RefDataError::Invalid`] for
    /// a bad note, or the reason the changes do not apply.
    pub fn propose(
        &mut self,
        by: PrincipalId,
        at: Timestamp,
        changes: Vec<Change>,
        note: Option<String>,
    ) -> Result<u64, RefDataError> {
        check_changes(&changes)?;
        if let Some(note) = &note {
            check_label("proposal note", note)?;
        }
        if self.pending().count() >= MAX_PENDING_PROPOSALS {
            return Err(RefDataError::Limit("too many pending proposals"));
        }
        self.current.apply(&changes)?;
        let id = u64::try_from(self.proposals.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        self.proposals.push(Proposal {
            id,
            base_version: self.version(),
            by,
            at,
            changes,
            note,
            status: ProposalStatus::Pending,
        });
        Ok(id)
    }

    fn pending_proposal(&mut self, id: u64) -> Result<&mut Proposal, RefDataError> {
        self.proposals
            .iter_mut()
            .find(|p| p.id == id && p.status == ProposalStatus::Pending)
            .ok_or(RefDataError::NotPending { id })
    }

    /// Approves a pending proposal and applies it to the current state. Returns the new
    /// version.
    ///
    /// # Errors
    /// [`RefDataError::NotPending`], [`RefDataError::SelfApproval`], or the reason the
    /// changes no longer apply (the proposal then stays pending).
    pub fn approve(
        &mut self,
        id: u64,
        by: PrincipalId,
        at: Timestamp,
    ) -> Result<u64, RefDataError> {
        let proposal = self.pending_proposal(id)?;
        let (proposer, changes) = (proposal.by, proposal.changes.clone());
        if proposer == by {
            return Err(RefDataError::SelfApproval);
        }
        let version = self.accept(proposer, Some(by), at, changes)?;
        self.pending_proposal(id)?.status = ProposalStatus::Approved { by, at, version };
        Ok(version)
    }

    /// Refuses a pending proposal, with a reason.
    ///
    /// # Errors
    /// [`RefDataError::NotPending`], [`RefDataError::SelfApproval`] (the proposer withdraws
    /// instead), [`RefDataError::Invalid`] for a blank reason.
    pub fn reject(
        &mut self,
        id: u64,
        by: PrincipalId,
        at: Timestamp,
        reason: &str,
    ) -> Result<(), RefDataError> {
        let proposal = self.pending_proposal(id)?;
        check_label("rejection reason", reason)?;
        if proposal.by == by {
            return Err(RefDataError::SelfApproval);
        }
        proposal.status = ProposalStatus::Rejected {
            by,
            at,
            reason: reason.to_owned(),
        };
        Ok(())
    }

    /// Takes a pending proposal back. Only its author can.
    ///
    /// # Errors
    /// [`RefDataError::NotPending`], [`RefDataError::NotProposer`].
    pub fn withdraw(
        &mut self,
        id: u64,
        by: PrincipalId,
        at: Timestamp,
    ) -> Result<(), RefDataError> {
        let proposal = self.pending_proposal(id)?;
        if proposal.by != by {
            return Err(RefDataError::NotProposer);
        }
        proposal.status = ProposalStatus::Withdrawn { at };
        Ok(())
    }

    /// The state the set had at a version (0 = the starting state).
    ///
    /// # Errors
    /// [`RefDataError::UnknownVersion`] for a version that does not exist yet.
    pub fn as_of(&self, version: u64) -> Result<State, RefDataError> {
        if version > self.version() {
            return Err(RefDataError::UnknownVersion { version });
        }
        let count = usize::try_from(version).unwrap_or(usize::MAX);
        let mut state = self.genesis.clone();
        for revision in self.revisions.iter().take(count) {
            state = state.apply(&revision.changes)?.0;
        }
        Ok(state)
    }

    /// The version that was current at an instant: the number of leading revisions accepted
    /// at or before it. (Versions, not clock times, define the order of changes; times are
    /// recorded for people.)
    #[must_use]
    pub fn version_at(&self, at: Timestamp) -> u64 {
        let count = self
            .revisions
            .iter()
            .take_while(|revision| revision.at <= at)
            .count();
        u64::try_from(count).unwrap_or(u64::MAX)
    }
}
