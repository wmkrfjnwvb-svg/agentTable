//! Share links and access reviews, including a model-based test of the review state machine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::field::Classification;
use mt_core::id::{AccessReviewId, GrantId, PrincipalId, RecordId, ShareLinkId};
use mt_perm::review::MAX_REVIEW_ITEMS;
use mt_perm::Directory;
use mt_perm::{
    AccessReview, Action, Capability, FieldAccess, FieldMeta, FieldPermission, Inactive, PermError,
    Reason, ReviewState, ShareLink, ShareMode, Verdict, Visibility,
};
use proptest::prelude::*;
use std::collections::BTreeMap;

fn link(mode: ShareMode) -> ShareLink {
    ShareLink {
        id: ShareLinkId::from_u128(1),
        on: ORDERS,
        record: None,
        view: None,
        mode,
        fields: FieldAccess::All,
        created_by: p(ALICE),
        created_at: t(0),
        expires_at: Some(t(10_000)),
        revoked_at: None,
    }
}

fn invalid(result: Result<(), PermError>) -> String {
    match result {
        Err(PermError::Invalid(reason)) => reason,
        other => panic!("{other:?}"),
    }
}

#[test]
fn std_share_links() {
    let rec = |n: u128| RecordId::from_u128(n);
    for mode in [
        ShareMode::View,
        ShareMode::Comment,
        ShareMode::Edit,
        ShareMode::Submit,
    ] {
        let l = link(mode);
        l.check().unwrap();
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<ShareLink>(&json).unwrap(), l);
        for action in Action::ALL.iter().copied() {
            let allowed = mode.actions().contains(&action);
            let result = l.allows(&action.into(), &ORDERS, Some(rec(1)), t(5));
            assert_eq!(result.is_ok(), allowed, "{mode:?} {action}");
            if !allowed {
                assert_eq!(result, Err(Reason::NoGrant));
            }
            // Nothing administrative, ever.
            assert!(!(allowed && action.is_administrative()));
        }
    }
    let view = link(ShareMode::View);
    // Validity.
    assert_eq!(view.status(t(5)), Ok(()));
    assert_eq!(view.status(t(-1)), Err(Inactive::NotYetValid));
    assert_eq!(view.status(t(10_000)), Err(Inactive::Expired));
    let mut revoked = view.clone();
    revoked.revoked_at = Some(t(3));
    assert_eq!(revoked.status(t(3)), Err(Inactive::Revoked));
    assert_eq!(
        revoked.allows(&read(), &ORDERS, None, t(4)),
        Err(Reason::Inactive {
            why: Inactive::Revoked
        })
    );
    let mut forever = view.clone();
    forever.expires_at = None;
    assert_eq!(forever.status(t(i64::from(i32::MAX))), Ok(()));
    // Only its own table.
    assert_eq!(
        view.allows(&read(), &PAYMENTS, None, t(5)),
        Err(Reason::Elsewhere)
    );
    assert_eq!(view.allows(&read(), &ORDERS, None, t(5)), Ok(()));
    // A link to one record opens that record only, and no list.
    let mut one = view.clone();
    one.record = Some(rec(7));
    one.check().unwrap();
    assert_eq!(one.allows(&read(), &ORDERS, Some(rec(7)), t(5)), Ok(()));
    assert_eq!(
        one.allows(&read(), &ORDERS, Some(rec(8)), t(5)),
        Err(Reason::OutOfScope)
    );
    assert_eq!(
        one.allows(&read(), &ORDERS, None, t(5)),
        Err(Reason::OutOfScope)
    );
    // Custom capabilities are not given by links.
    assert_eq!(
        view.allows(&custom("salary.view"), &ORDERS, None, t(5)),
        Err(Reason::NoGrant)
    );
}

#[test]
fn invalid_share_links() {
    let mut l = link(ShareMode::View);
    l.on = BASE1;
    assert_eq!(invalid(l.check()), "a share link opens one table");
    let mut l = link(ShareMode::Submit);
    l.record = Some(RecordId::from_u128(1));
    assert_eq!(
        invalid(l.check()),
        "a link for submitting new records cannot name a record"
    );
    let mut l = link(ShareMode::View);
    l.fields = FieldAccess::none();
    assert_eq!(invalid(l.check()), "the link includes no field at all");
    let mut l = link(ShareMode::View);
    l.expires_at = Some(t(0));
    assert_eq!(invalid(l.check()), "the link expires before it starts");
}

#[test]
fn std_share_links_never_reveal_protected_fields() {
    let fields = order_fields();
    let by_id: BTreeMap<_, _> = fields.iter().map(|f| (f.id.as_u128(), f)).collect();
    let permission = |mode: ShareMode, field: u128| link(mode).field_permission(by_id[&field]);
    let shown = |visibility, editable| FieldPermission {
        visibility,
        editable,
    };
    // Viewing: plain fields shown, classified and protected ones hidden, masks stay.
    assert_eq!(
        permission(ShareMode::View, AMOUNT),
        shown(Visibility::Visible, false)
    );
    assert_eq!(
        permission(ShareMode::View, STATUS),
        shown(Visibility::Visible, false)
    );
    assert_eq!(
        permission(ShareMode::View, NOTE),
        shown(Visibility::Masked, false)
    );
    for classified in [SALARY, EMAIL, SECRET, REQUESTER] {
        assert_eq!(
            permission(ShareMode::View, classified),
            shown(Visibility::Hidden, false),
            "{classified}"
        );
    }
    // Editing: what is shown may be changed, except generated values and masked ones.
    assert_eq!(
        permission(ShareMode::Edit, AMOUNT),
        shown(Visibility::Visible, true)
    );
    assert_eq!(
        permission(ShareMode::Edit, TOTAL),
        shown(Visibility::Visible, false)
    );
    assert_eq!(
        permission(ShareMode::Edit, NOTE),
        shown(Visibility::Masked, false)
    );
    assert_eq!(
        permission(ShareMode::Comment, AMOUNT),
        shown(Visibility::Visible, false)
    );
    // A form: nothing is shown, but open fields can be filled in.
    assert_eq!(
        permission(ShareMode::Submit, AMOUNT),
        shown(Visibility::Hidden, true)
    );
    assert_eq!(
        permission(ShareMode::Submit, TOTAL),
        shown(Visibility::Hidden, false)
    );
    assert_eq!(
        permission(ShareMode::Submit, NOTE),
        shown(Visibility::Hidden, false)
    );
    assert_eq!(
        permission(ShareMode::Submit, SALARY),
        shown(Visibility::Hidden, false)
    );
    // The link's own field list.
    let mut narrow = link(ShareMode::Edit);
    narrow.fields = FieldAccess::only([f(STATUS)]);
    assert_eq!(
        narrow.field_permission(by_id[&AMOUNT]),
        shown(Visibility::Hidden, false)
    );
    assert_eq!(
        narrow.field_permission(by_id[&STATUS]),
        shown(Visibility::Visible, true)
    );
    // A group's edit capability protects a field shown through the link.
    let guarded = FieldMeta {
        edit_capability: Some(Capability::custom("price.edit").unwrap()),
        ..FieldMeta {
            id: f(1),
            classification: Classification::Public,
            masked: false,
            generated: false,
            view_capability: None,
            edit_capability: None,
        }
    };
    assert_eq!(
        link(ShareMode::Edit).field_permission(&guarded),
        shown(Visibility::Visible, false)
    );
}

fn grants_in_force() -> Vec<mt_perm::Grant> {
    let mut list = vec![
        grant(1, BOB, read(), ORDERS),
        grant(2, FINANCE, cap(Action::ProcessApprove), ORDERS),
        grant(3, CAROL, read(), BASE1),
    ];
    let mut revoked = grant(4, DAVE, read(), ORDERS);
    revoked.revoked_at = Some(t(1));
    list.push(revoked);
    let mut later = grant(5, DAVE, read(), ORDERS);
    later.valid_from = Some(t(NOW + 1));
    list.push(later);
    list
}

fn review() -> AccessReview {
    AccessReview::open(
        AccessReviewId::from_u128(1),
        "Q4 review",
        p(ALICE),
        t(NOW),
        Some(t(NOW + 1_000)),
        &grants_in_force(),
    )
    .unwrap()
}

fn review_error(result: Result<impl core::fmt::Debug, PermError>) -> String {
    match result {
        Err(PermError::Review(message)) => message,
        other => panic!("{other:?}"),
    }
}

#[test]
fn std_access_reviews() {
    let org = Org::new();
    let mut review = review();
    // Only grants in force at the start are reviewed.
    let reviewed: Vec<GrantId> = review.items().map(|i| i.grant).collect();
    assert_eq!(reviewed, [gid(1), gid(2), gid(3)]);
    assert_eq!(review.progress(), (0, 3));
    assert_eq!(review.undecided(), [gid(1), gid(2), gid(3)]);
    assert!(!review.is_overdue(t(NOW + 1_000)));
    assert!(review.is_overdue(t(NOW + 1_001)));
    review
        .decide(gid(1), Verdict::Keep, p(ALICE), t(NOW + 1), None, &org)
        .unwrap();
    review
        .decide(
            gid(2),
            Verdict::Revoke,
            p(ALICE),
            t(NOW + 2),
            Some("left the team".into()),
            &org,
        )
        .unwrap();
    // A verdict can be changed until the review is completed.
    review
        .decide(
            gid(1),
            Verdict::Revoke,
            p(DAVE),
            t(NOW + 3),
            Some("not needed".into()),
            &org,
        )
        .unwrap();
    let first = review.items().next().unwrap();
    assert_eq!(
        (first.verdict, first.reviewer, first.note.as_deref()),
        (Some(Verdict::Revoke), Some(p(DAVE)), Some("not needed"))
    );
    assert_eq!(review.progress(), (2, 3));
    assert_eq!(
        review_error(review.complete(p(ALICE), t(NOW + 4))),
        "1 grant(s) have no verdict yet"
    );
    review
        .decide(
            gid(3),
            Verdict::Keep,
            p(ALICE),
            t(NOW + 5),
            Some("  ".into()),
            &org,
        )
        .unwrap();
    assert_eq!(
        review.items().nth(2).unwrap().note,
        None,
        "a blank note is no note"
    );
    let revoke = review.complete(p(ALICE), t(NOW + 6)).unwrap();
    assert_eq!(revoke, [gid(1), gid(2)]);
    assert_eq!(review.state, ReviewState::Completed);
    assert_eq!(
        (review.closed_at, review.closed_by),
        (Some(t(NOW + 6)), Some(p(ALICE)))
    );
    assert!(!review.is_overdue(t(NOW + 9_999)));
    // Closed is closed.
    assert_eq!(
        review_error(review.decide(gid(3), Verdict::Keep, p(ALICE), t(NOW + 7), None, &org)),
        "the review is completed"
    );
    assert_eq!(
        review_error(review.cancel(p(ALICE), t(NOW + 7))),
        "the review is completed"
    );
    let json = serde_json::to_string(&review).unwrap();
    assert_eq!(serde_json::from_str::<AccessReview>(&json).unwrap(), review);
    // Cancelling revokes nothing.
    let mut cancelled = self::review();
    cancelled.cancel(p(ALICE), t(NOW + 1)).unwrap();
    assert_eq!(cancelled.state, ReviewState::Cancelled);
    assert_eq!(
        review_error(cancelled.complete(p(ALICE), t(NOW + 2))),
        "the review was cancelled"
    );
}

#[test]
fn invalid_reviews_and_verdicts() {
    let org = Org::new();
    let grants = grants_in_force();
    let open = |name: &str, due: Option<i64>, grants: &[mt_perm::Grant]| {
        AccessReview::open(
            AccessReviewId::from_u128(1),
            name,
            p(ALICE),
            t(NOW),
            due.map(t),
            grants,
        )
        .map(|_| ())
    };
    assert_eq!(
        review_error(open(" ", None, &grants)),
        "a review needs a name"
    );
    assert_eq!(
        review_error(open("x", Some(NOW), &grants)),
        "the review is due before it opens"
    );
    assert_eq!(
        review_error(open("x", None, &grants[3..])),
        "there is no grant in force to review"
    );
    let many: Vec<_> = (0..=MAX_REVIEW_ITEMS as u128)
        .map(|n| grant(100 + n, BOB, read(), ORDERS))
        .collect();
    assert_eq!(
        review_error(open("x", None, &many)),
        "a review covers at most 10000 grants"
    );
    open("x", None, &many[..MAX_REVIEW_ITEMS]).unwrap();
    let mut review = review();
    assert!(
        review_error(review.decide(gid(9), Verdict::Keep, p(ALICE), t(NOW), None, &org))
            .contains("is not part of this review")
    );
    // Nobody reviews their own access, directly or through a group (Carol is in Finance).
    assert_eq!(
        review_error(review.decide(gid(1), Verdict::Keep, p(BOB), t(NOW), None, &org)),
        "nobody reviews their own access"
    );
    assert_eq!(
        review_error(review.decide(gid(2), Verdict::Keep, p(CAROL), t(NOW), None, &org)),
        "nobody reviews their own access"
    );
    assert_eq!(
        review_error(review.decide(gid(1), Verdict::Revoke, p(ALICE), t(NOW), None, &org)),
        "a revocation needs a note"
    );
    assert_eq!(review.progress(), (0, 3), "refused verdicts change nothing");
}

/// Steps of the review state machine.
#[derive(Debug, Clone)]
enum Step {
    Decide {
        grant: u8,
        revoke: bool,
        reviewer: u8,
        note: bool,
    },
    Complete,
    Cancel,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => (0_u8..4, any::<bool>(), 0_u8..4, any::<bool>())
            .prop_map(|(grant, revoke, reviewer, note)| Step::Decide { grant, revoke, reviewer, note }),
        1 => Just(Step::Complete),
        1 => Just(Step::Cancel),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    /// The review agrees with a simple model after every step: which grants are decided,
    /// which are to be revoked, and when it may close.
    #[test]
    fn prop_reviews_follow_their_model(steps in prop::collection::vec(step(), 0..24)) {
        let org = Org::new();
        let reviewers = [ALICE, BOB, DAVE, CAROL];
        let holders = [BOB, FINANCE, CAROL];
        let mut review = review();
        let mut model: BTreeMap<u128, (bool, Option<bool>)> = (1..=3).map(|g| (g, (false, None))).collect();
        let mut state = ReviewState::Open;
        for (at, step) in steps.into_iter().enumerate() {
            let now = t(NOW + at as i64);
            match step {
                Step::Decide { grant, revoke, reviewer, note } => {
                    let id = u128::from(grant) + 1;
                    let who = reviewers[usize::from(reviewer)];
                    let verdict = if revoke { Verdict::Revoke } else { Verdict::Keep };
                    let result = review.decide(gid(id), verdict, p(who), now, note.then(|| "why".to_owned()), &org);
                    let in_review = model.contains_key(&id);
                    let own = in_review && {
                        let holder = holders[(id - 1) as usize];
                        holder == who || mt_perm::Directory::members_of(&org, p(holder)).contains(&p(who))
                    };
                    let expected_ok = state == ReviewState::Open && in_review && !own && (!revoke || note);
                    prop_assert_eq!(result.is_ok(), expected_ok, "{:?}", result);
                    if expected_ok {
                        model.insert(id, (true, Some(revoke)));
                    }
                }
                Step::Complete => {
                    let result = review.complete(p(ALICE), now);
                    let all_decided = model.values().all(|(decided, _)| *decided);
                    prop_assert_eq!(result.is_ok(), state == ReviewState::Open && all_decided);
                    if let Ok(revoked) = result {
                        let expected: Vec<GrantId> = model.iter().filter(|(_, (_, v))| *v == Some(true)).map(|(g, _)| gid(*g)).collect();
                        prop_assert_eq!(revoked, expected);
                        state = ReviewState::Completed;
                    }
                }
                Step::Cancel => {
                    let result = review.cancel(p(ALICE), now);
                    prop_assert_eq!(result.is_ok(), state == ReviewState::Open);
                    if result.is_ok() {
                        state = ReviewState::Cancelled;
                    }
                }
            }
            prop_assert_eq!(review.state, state);
            let decided = model.values().filter(|(d, _)| *d).count();
            prop_assert_eq!(review.progress(), (decided, 3));
        }
        let _ = PrincipalId::from_u128(0);
    }
}

// ---- model-based: the review against a reference ----

#[derive(Debug, Clone)]
enum ReviewOp {
    Decide {
        grant: u128,
        keep: bool,
        reviewer: u128,
        note: bool,
    },
    Complete {
        by: u128,
    },
    Cancel {
        by: u128,
    },
}

fn review_op() -> impl Strategy<Value = ReviewOp> {
    let who = prop::sample::select(vec![ALICE, BOB, CAROL, DAVE]);
    prop_oneof![
        6 => (1_u128..=5, any::<bool>(), who.clone(), any::<bool>())
            .prop_map(|(grant, keep, reviewer, note)| ReviewOp::Decide { grant, keep, reviewer, note }),
        2 => who.clone().prop_map(|by| ReviewOp::Complete { by }),
        1 => who.prop_map(|by| ReviewOp::Cancel { by }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every sequence of verdicts, completions and cancellations behaves like the reference:
    /// only open reviews change, nobody reviews their own (or their group's) access, a
    /// revocation needs a note, completion needs every verdict and returns the revocations,
    /// and a refused step changes nothing.
    #[test]
    fn model_access_reviews_follow_the_reference(ops in prop::collection::vec(review_op(), 1..40)) {
        let org = Org::new();
        let mut review = review();
        // The reference: state, and per reviewed grant its holder and verdict.
        let mut open = true;
        let mut verdicts: BTreeMap<u128, (PrincipalId, Option<bool>)> = review
            .items()
            .map(|i| (i.grant.as_u128(), (i.principal, None)))
            .collect();
        for (step, op) in ops.into_iter().enumerate() {
            let at = t(NOW + 1 + i64::try_from(step).unwrap());
            let before = review.clone();
            match op {
                ReviewOp::Decide { grant, keep, reviewer, note } => {
                    let verdict = if keep { Verdict::Keep } else { Verdict::Revoke };
                    let result = review.decide(gid(grant), verdict, p(reviewer), at, note.then(|| "left the team".to_owned()), &org);
                    let expected = open
                        && verdicts.get(&grant).is_some_and(|(holder, _)| {
                            *holder != p(reviewer) && !org.members_of(*holder).contains(&p(reviewer))
                        })
                        && (keep || note);
                    prop_assert_eq!(result.is_ok(), expected, "{:?}", result);
                    if expected {
                        if let Some(slot) = verdicts.get_mut(&grant) {
                            slot.1 = Some(keep);
                        }
                    }
                }
                ReviewOp::Complete { by } => {
                    let result = review.complete(p(by), at);
                    let expected = open && verdicts.values().all(|(_, v)| v.is_some());
                    prop_assert_eq!(result.is_ok(), expected);
                    if let Ok(revoked) = result {
                        let wanted: Vec<GrantId> = verdicts.iter().filter(|(_, (_, v))| *v == Some(false)).map(|(g, _)| gid(*g)).collect();
                        prop_assert_eq!(revoked, wanted);
                        prop_assert_eq!(review.state, ReviewState::Completed);
                        open = false;
                    }
                }
                ReviewOp::Cancel { by } => {
                    let result = review.cancel(p(by), at);
                    prop_assert_eq!(result.is_ok(), open);
                    if result.is_ok() {
                        prop_assert_eq!(review.state, ReviewState::Cancelled);
                        open = false;
                    }
                }
            }
            if review == before {
                continue;
            }
            // Something changed: it must have been allowed, and the progress must agree.
            let decided = verdicts.values().filter(|(_, v)| v.is_some()).count();
            prop_assert_eq!(review.progress(), (decided, verdicts.len()));
        }
    }
}
