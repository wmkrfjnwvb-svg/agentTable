//! Access reviews: periodically, someone other than the holder confirms that each grant
//! is still needed.
//!
//! A review is a small state machine over a snapshot of grants:
//!
//! ```text
//! open ──(every grant kept or marked for revocation)──▶ completed
//!   └──────────────────────────────────────────────────▶ cancelled
//! ```
//!
//! Nobody reviews their own access, a revocation needs a note, verdicts can be changed
//! until the review is completed, and completing returns the grants to revoke.

use crate::error::PermError;
use crate::grant::Grant;
use crate::policy::Directory;
use mt_core::id::{AccessReviewId, GrantId, PrincipalId};
use mt_core::time::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Maximum number of grants in one review.
pub const MAX_REVIEW_ITEMS: usize = 10_000;

/// What the reviewer decided about a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The grant is still needed.
    Keep,
    /// The grant is to be revoked.
    Revoke,
}

/// One grant under review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewItem {
    /// The grant.
    pub grant: GrantId,
    /// Who holds it.
    pub principal: PrincipalId,
    /// The verdict, once decided.
    pub verdict: Option<Verdict>,
    /// Who decided.
    pub reviewer: Option<PrincipalId>,
    /// When.
    pub decided_at: Option<Timestamp>,
    /// Why (required for a revocation).
    pub note: Option<String>,
}

/// Where a review stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Verdicts are being collected.
    Open,
    /// Every grant has a verdict and the review was closed.
    Completed,
    /// The review was abandoned; nothing is revoked.
    Cancelled,
}

/// A review of a set of grants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessReview {
    /// Stable ID.
    pub id: AccessReviewId,
    /// Name ("Q2 review of finance access").
    pub name: String,
    /// Who opened it.
    pub opened_by: PrincipalId,
    /// When it was opened.
    pub opened_at: Timestamp,
    /// When it should be completed.
    pub due_at: Option<Timestamp>,
    /// Where it stands.
    pub state: ReviewState,
    /// When it was completed or cancelled.
    pub closed_at: Option<Timestamp>,
    /// Who completed or cancelled it.
    pub closed_by: Option<PrincipalId>,
    items: BTreeMap<GrantId, ReviewItem>,
}

impl AccessReview {
    /// Opens a review of the grants that are in force at `opened_at`.
    ///
    /// # Errors
    /// [`PermError::Review`] for a review without a name, without a grant in force, with
    /// too many grants, or due before it opens.
    pub fn open<'g>(
        id: AccessReviewId,
        name: impl Into<String>,
        opened_by: PrincipalId,
        opened_at: Timestamp,
        due_at: Option<Timestamp>,
        grants: impl IntoIterator<Item = &'g Grant>,
    ) -> Result<AccessReview, PermError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(PermError::Review("a review needs a name".to_owned()));
        }
        if due_at.is_some_and(|due| due <= opened_at) {
            return Err(PermError::Review(
                "the review is due before it opens".to_owned(),
            ));
        }
        let items: BTreeMap<GrantId, ReviewItem> = grants
            .into_iter()
            .filter(|grant| grant.status(opened_at).is_ok())
            .map(|grant| {
                (
                    grant.id,
                    ReviewItem {
                        grant: grant.id,
                        principal: grant.principal,
                        verdict: None,
                        reviewer: None,
                        decided_at: None,
                        note: None,
                    },
                )
            })
            .collect();
        if items.is_empty() {
            return Err(PermError::Review(
                "there is no grant in force to review".to_owned(),
            ));
        }
        if items.len() > MAX_REVIEW_ITEMS {
            return Err(PermError::Review(format!(
                "a review covers at most {MAX_REVIEW_ITEMS} grants"
            )));
        }
        Ok(AccessReview {
            id,
            name,
            opened_by,
            opened_at,
            due_at,
            state: ReviewState::Open,
            closed_at: None,
            closed_by: None,
            items,
        })
    }

    /// The grants under review, by grant id.
    pub fn items(&self) -> impl Iterator<Item = &ReviewItem> {
        self.items.values()
    }

    /// The grants without a verdict yet.
    #[must_use]
    pub fn undecided(&self) -> Vec<GrantId> {
        self.items
            .values()
            .filter(|item| item.verdict.is_none())
            .map(|item| item.grant)
            .collect()
    }

    /// How many grants have a verdict, and how many there are.
    #[must_use]
    pub fn progress(&self) -> (usize, usize) {
        let decided = self
            .items
            .values()
            .filter(|item| item.verdict.is_some())
            .count();
        (decided, self.items.len())
    }

    /// Whether the review is still open after its due time.
    #[must_use]
    pub fn is_overdue(&self, now: Timestamp) -> bool {
        self.state == ReviewState::Open && self.due_at.is_some_and(|due| now > due)
    }

    fn require_open(&self) -> Result<(), PermError> {
        match self.state {
            ReviewState::Open => Ok(()),
            ReviewState::Completed => Err(PermError::Review("the review is completed".to_owned())),
            ReviewState::Cancelled => Err(PermError::Review("the review was cancelled".to_owned())),
        }
    }

    /// Records (or changes) the verdict on a grant.
    ///
    /// # Errors
    /// [`PermError::Review`] if the review is closed, the grant is not part of it, the
    /// reviewer holds the grant (directly or as a member of the group that holds it), or a
    /// revocation has no note.
    pub fn decide(
        &mut self,
        grant: GrantId,
        verdict: Verdict,
        reviewer: PrincipalId,
        at: Timestamp,
        note: Option<String>,
        directory: &dyn Directory,
    ) -> Result<(), PermError> {
        self.require_open()?;
        let item = self
            .items
            .get_mut(&grant)
            .ok_or_else(|| PermError::Review(format!("{grant} is not part of this review")))?;
        if item.principal == reviewer || directory.members_of(item.principal).contains(&reviewer) {
            return Err(PermError::Review(
                "nobody reviews their own access".to_owned(),
            ));
        }
        let note = note.filter(|text| !text.trim().is_empty());
        if verdict == Verdict::Revoke && note.is_none() {
            return Err(PermError::Review("a revocation needs a note".to_owned()));
        }
        item.verdict = Some(verdict);
        item.reviewer = Some(reviewer);
        item.decided_at = Some(at);
        item.note = note;
        Ok(())
    }

    /// Completes the review and returns the grants to revoke.
    ///
    /// # Errors
    /// [`PermError::Review`] if the review is closed or grants are still undecided.
    pub fn complete(&mut self, by: PrincipalId, at: Timestamp) -> Result<Vec<GrantId>, PermError> {
        self.require_open()?;
        let undecided = self.undecided().len();
        if undecided > 0 {
            return Err(PermError::Review(format!(
                "{undecided} grant(s) have no verdict yet"
            )));
        }
        self.state = ReviewState::Completed;
        self.closed_at = Some(at);
        self.closed_by = Some(by);
        Ok(self
            .items
            .values()
            .filter(|item| item.verdict == Some(Verdict::Revoke))
            .map(|item| item.grant)
            .collect())
    }

    /// Abandons the review; nothing is revoked.
    ///
    /// # Errors
    /// [`PermError::Review`] if the review is already closed.
    pub fn cancel(&mut self, by: PrincipalId, at: Timestamp) -> Result<(), PermError> {
        self.require_open()?;
        self.state = ReviewState::Cancelled;
        self.closed_at = Some(at);
        self.closed_by = Some(by);
        Ok(())
    }
}
