//! Provenance: where a cell value came from and who is accountable for it.
//!
//! Every cell change records a [`CellMeta`]:
//!
//! * the [`Source`] — a person, an import, a data source, an automation, a formula or AI;
//! * the [`Actor`] — the principal that performed the change and, for AI and automations, the
//!   **initiator** on whose authority it ran (dual identity);
//! * the [`ReviewState`] — whether an AI value is still a suggestion;
//! * the operation and trace IDs and the time.
//!
//! Rules enforced by [`CellMeta::check`] (product design, doc 18 §1):
//!
//! * an AI change always names an initiator; effective rights are the intersection of both;
//! * only AI values can be suggestions, and AI can never accept or reject its own suggestion;
//! * a person acts for themselves (no initiator).

use crate::decimal::Decimal;
use crate::error::CoreError;
use crate::id::{
    AgentId, AttachmentId, DataSourceId, FieldId, JobId, OpId, PrincipalId, RecordId, RunId,
    SkillId, TraceId,
};
use crate::time::Timestamp;

/// Maximum number of evidence references on one AI value.
pub const MAX_EVIDENCE: usize = 64;
/// Maximum number of characters in an evidence excerpt.
pub const MAX_EXCERPT_CHARS: usize = 500;
/// Maximum number of characters in a model name or external ID.
pub const MAX_LABEL_CHARS: usize = 200;

/// A confidence between 0 and 1, stored exactly in basis points (1/10 000).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "u16", into = "u16")]
pub struct Confidence(u16);

impl Confidence {
    /// Certain.
    pub const MAX: Confidence = Confidence(10_000);
    /// No confidence.
    pub const MIN: Confidence = Confidence(0);

    /// Builds a confidence from basis points (0 ..= 10 000).
    ///
    /// # Errors
    /// [`CoreError::InvalidDefinition`] above 10 000.
    pub fn from_basis_points(basis_points: u16) -> Result<Self, CoreError> {
        if basis_points <= Self::MAX.0 {
            Ok(Confidence(basis_points))
        } else {
            Err(CoreError::InvalidDefinition {
                reason: format!("confidence {basis_points} is above 10000 basis points"),
            })
        }
    }

    /// The value in basis points.
    #[must_use]
    pub const fn basis_points(self) -> u16 {
        self.0
    }

    /// The value as an exact fraction between 0 and 1.
    #[must_use]
    pub fn as_fraction(self) -> Decimal {
        Decimal::new(i64::from(self.0), 4).normalize()
    }

    /// Whether this confidence reaches a threshold.
    #[must_use]
    pub const fn meets(self, threshold: Confidence) -> bool {
        self.0 >= threshold.0
    }
}

impl TryFrom<u16> for Confidence {
    type Error = CoreError;
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Confidence::from_basis_points(value)
    }
}

impl From<Confidence> for u16 {
    fn from(value: Confidence) -> Self {
        value.0
    }
}

/// Which AI object produced a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiProducer {
    /// A stateless AI skill.
    Skill {
        /// The skill.
        skill: SkillId,
        /// The skill version that ran.
        version: u32,
    },
    /// An agent.
    Agent {
        /// The agent.
        agent: AgentId,
        /// The agent version that ran.
        version: u32,
    },
}

/// Something an AI value is based on.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceRef {
    /// Another cell.
    Cell {
        /// The record.
        record: RecordId,
        /// The field.
        field: FieldId,
    },
    /// A whole record.
    Record {
        /// The record.
        record: RecordId,
    },
    /// An attachment, optionally one page of it (1-based).
    Attachment {
        /// The attachment.
        attachment: AttachmentId,
        /// The page, if the evidence is on one page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page: Option<u32>,
    },
    /// A short quoted excerpt.
    Excerpt {
        /// The quoted text.
        text: String,
    },
}

/// The AI part of a provenance record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AiProvenance {
    /// The skill or agent.
    pub producer: AiProducer,
    /// The model that generated the value (provider-qualified name).
    pub model: String,
    /// Model-reported or calibrated confidence, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// What the value is based on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
}

/// Where a value came from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Typed or chosen by a person.
    Human,
    /// Loaded by an import job.
    Import {
        /// The import job.
        job: JobId,
    },
    /// Received from an authoritative data source.
    DataSource {
        /// The data source.
        source: DataSourceId,
        /// The record's ID in the external system, if known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        external_id: Option<String>,
    },
    /// Written by a rule or workflow run.
    Automation {
        /// The run.
        run: RunId,
    },
    /// Computed by a formula, lookup or rollup.
    Formula,
    /// Produced by an AI skill or agent.
    Ai(AiProvenance),
}

impl Source {
    /// Whether the value was produced by AI.
    #[must_use]
    pub const fn is_ai(&self) -> bool {
        matches!(self, Source::Ai(_))
    }

    /// A short stable name of the source kind (matches the serde `kind` tag).
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Source::Human => "human",
            Source::Import { .. } => "import",
            Source::DataSource { .. } => "data_source",
            Source::Automation { .. } => "automation",
            Source::Formula => "formula",
            Source::Ai(_) => "ai",
        }
    }
}

/// Who performed a change (dual identity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Actor {
    /// The principal that performed the change (a person, an agent, a service).
    pub principal: PrincipalId,
    /// The principal on whose authority an AI or automation ran. `None` when a person acts
    /// for themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiator: Option<PrincipalId>,
}

impl Actor {
    /// A person acting for themselves.
    #[must_use]
    pub const fn direct(principal: PrincipalId) -> Self {
        Actor {
            principal,
            initiator: None,
        }
    }

    /// A principal acting on behalf of an initiator.
    #[must_use]
    pub const fn on_behalf(principal: PrincipalId, initiator: PrincipalId) -> Self {
        Actor {
            principal,
            initiator: Some(initiator),
        }
    }

    /// Every principal whose permissions must allow the change (the acting principal and, if
    /// present and different, the initiator). Effective rights are the intersection.
    #[must_use]
    pub fn accountable(&self) -> Vec<PrincipalId> {
        match self.initiator {
            Some(initiator) if initiator != self.principal => vec![self.principal, initiator],
            Some(_) | None => vec![self.principal],
        }
    }
}

/// Review state of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReviewState {
    /// The value is ordinary committed data.
    NotRequired,
    /// An AI suggestion waiting for a person.
    Suggested,
    /// A person accepted the suggestion.
    Accepted {
        /// Who accepted.
        by: PrincipalId,
        /// When.
        at: Timestamp,
    },
    /// A person rejected the suggestion.
    Rejected {
        /// Who rejected.
        by: PrincipalId,
        /// When.
        at: Timestamp,
    },
}

impl ReviewState {
    const fn name(self) -> &'static str {
        match self {
            ReviewState::NotRequired => "not_required",
            ReviewState::Suggested => "suggested",
            ReviewState::Accepted { .. } => "accepted",
            ReviewState::Rejected { .. } => "rejected",
        }
    }

    /// Whether the value counts as committed data (`not_required` or `accepted`).
    #[must_use]
    pub const fn is_effective(self) -> bool {
        matches!(
            self,
            ReviewState::NotRequired | ReviewState::Accepted { .. }
        )
    }

    /// Whether the value still waits for a decision.
    #[must_use]
    pub const fn is_pending(self) -> bool {
        matches!(self, ReviewState::Suggested)
    }

    /// Accepts a suggestion.
    ///
    /// # Errors
    /// [`CoreError::InvalidTransition`] unless the state is `suggested`.
    pub fn accept(self, by: PrincipalId, at: Timestamp) -> Result<ReviewState, CoreError> {
        match self {
            ReviewState::Suggested => Ok(ReviewState::Accepted { by, at }),
            ReviewState::NotRequired
            | ReviewState::Accepted { .. }
            | ReviewState::Rejected { .. } => Err(CoreError::InvalidTransition {
                from: self.name(),
                action: "accept",
            }),
        }
    }

    /// Rejects a suggestion.
    ///
    /// # Errors
    /// [`CoreError::InvalidTransition`] unless the state is `suggested`.
    pub fn reject(self, by: PrincipalId, at: Timestamp) -> Result<ReviewState, CoreError> {
        match self {
            ReviewState::Suggested => Ok(ReviewState::Rejected { by, at }),
            ReviewState::NotRequired
            | ReviewState::Accepted { .. }
            | ReviewState::Rejected { .. } => Err(CoreError::InvalidTransition {
                from: self.name(),
                action: "reject",
            }),
        }
    }
}

/// The provenance record of one cell change.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CellMeta {
    /// Where the value came from.
    pub source: Source,
    /// Who performed the change.
    pub actor: Actor,
    /// Review state.
    pub review: ReviewState,
    /// The operation that wrote the value.
    pub op: OpId,
    /// The trace the operation belongs to.
    pub trace: TraceId,
    /// When the value was written.
    pub at: Timestamp,
}

fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidDefinition {
        reason: reason.to_owned(),
    }
}

fn check_label(what: &str, text: &str) -> Result<(), CoreError> {
    if text.trim().is_empty() {
        return Err(CoreError::InvalidDefinition {
            reason: format!("{what} must not be empty"),
        });
    }
    if text.chars().count() > MAX_LABEL_CHARS {
        return Err(CoreError::InvalidDefinition {
            reason: format!("{what} is longer than {MAX_LABEL_CHARS} characters"),
        });
    }
    Ok(())
}

impl CellMeta {
    /// Verifies the provenance rules listed in the module documentation.
    ///
    /// # Errors
    /// [`CoreError::InvalidDefinition`] naming the broken rule.
    pub fn check(&self) -> Result<(), CoreError> {
        match &self.source {
            Source::Human => {
                if self.actor.initiator.is_some() {
                    return Err(invalid(
                        "a person acts for themselves: a human change has no initiator",
                    ));
                }
            }
            Source::DataSource { external_id, .. } => {
                if let Some(external_id) = external_id {
                    check_label("external id", external_id)?;
                }
            }
            Source::Import { .. } | Source::Automation { .. } | Source::Formula => {}
            Source::Ai(ai) => {
                if self.actor.initiator.is_none() {
                    return Err(invalid(
                        "an AI change must name the initiator on whose authority it ran",
                    ));
                }
                check_label("model name", &ai.model)?;
                if ai.evidence.len() > MAX_EVIDENCE {
                    return Err(invalid("too many evidence references"));
                }
                for evidence in &ai.evidence {
                    match evidence {
                        EvidenceRef::Excerpt { text } => {
                            if text.is_empty() || text.chars().count() > MAX_EXCERPT_CHARS {
                                return Err(invalid(
                                    "an evidence excerpt must have 1 to 500 characters",
                                ));
                            }
                        }
                        EvidenceRef::Attachment { page, .. } => {
                            if *page == Some(0) {
                                return Err(invalid("evidence page numbers start at 1"));
                            }
                        }
                        EvidenceRef::Cell { .. } | EvidenceRef::Record { .. } => {}
                    }
                }
            }
        }
        match self.review {
            ReviewState::NotRequired => {}
            ReviewState::Suggested => {
                if !self.source.is_ai() {
                    return Err(invalid("only AI values can be suggestions"));
                }
            }
            ReviewState::Accepted { by, at } | ReviewState::Rejected { by, at } => {
                if !self.source.is_ai() {
                    return Err(invalid("only AI values can be reviewed"));
                }
                if by == self.actor.principal {
                    return Err(invalid("AI cannot review its own suggestion"));
                }
                if at < self.at {
                    return Err(invalid(
                        "a review cannot be earlier than the value it reviews",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Whether the value counts as committed data.
    #[must_use]
    pub const fn is_effective(&self) -> bool {
        self.review.is_effective()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(n: u128) -> PrincipalId {
        PrincipalId::from_u128(n)
    }

    fn t(ms: i64) -> Timestamp {
        Timestamp::from_millis(ms).unwrap()
    }

    fn ai_source() -> Source {
        Source::Ai(AiProvenance {
            producer: AiProducer::Skill {
                skill: SkillId::from_u128(1),
                version: 3,
            },
            model: "provider/model-x".into(),
            confidence: Some(Confidence::from_basis_points(8_750).unwrap()),
            evidence: vec![
                EvidenceRef::Cell {
                    record: RecordId::from_u128(1),
                    field: FieldId::from_u128(2),
                },
                EvidenceRef::Attachment {
                    attachment: AttachmentId::from_u128(3),
                    page: Some(2),
                },
                EvidenceRef::Excerpt {
                    text: "Total due: 120.00".into(),
                },
                EvidenceRef::Record {
                    record: RecordId::from_u128(4),
                },
            ],
        })
    }

    fn meta(source: Source, actor: Actor, review: ReviewState) -> CellMeta {
        CellMeta {
            source,
            actor,
            review,
            op: OpId::from_u128(9),
            trace: TraceId::from_u128(8),
            at: t(1_000),
        }
    }

    fn reason(result: Result<(), CoreError>) -> String {
        match result {
            Err(CoreError::InvalidDefinition { reason }) => reason,
            other => panic!("expected InvalidDefinition, got {other:?}"),
        }
    }

    #[test]
    fn std_confidence_bounds_and_fraction() {
        assert_eq!(Confidence::from_basis_points(0).unwrap(), Confidence::MIN);
        assert_eq!(
            Confidence::from_basis_points(10_000).unwrap(),
            Confidence::MAX
        );
        let c = Confidence::from_basis_points(8_750).unwrap();
        assert_eq!(c.basis_points(), 8_750);
        assert_eq!(c.as_fraction().to_string(), "0.875");
        assert_eq!(Confidence::MAX.as_fraction().to_string(), "1");
        assert!(c.meets(Confidence::from_basis_points(8_000).unwrap()));
        assert!(c.meets(c));
        assert!(!c.meets(Confidence::MAX));
        assert_eq!(serde_json::to_string(&c).unwrap(), "8750");
        assert_eq!(serde_json::from_str::<Confidence>("8750").unwrap(), c);
    }

    #[test]
    fn invalid_confidence_above_one() {
        assert!(Confidence::from_basis_points(10_001).is_err());
        assert!(serde_json::from_str::<Confidence>("10001").is_err());
        assert!(serde_json::from_str::<Confidence>("-1").is_err());
        assert!(serde_json::from_str::<Confidence>("0.5").is_err());
    }

    #[test]
    fn std_valid_metas_for_every_source() {
        let cases = [
            meta(Source::Human, Actor::direct(p(1)), ReviewState::NotRequired),
            meta(
                Source::Import {
                    job: JobId::from_u128(1),
                },
                Actor::direct(p(1)),
                ReviewState::NotRequired,
            ),
            meta(
                Source::DataSource {
                    source: DataSourceId::from_u128(1),
                    external_id: Some("ORD-1".into()),
                },
                Actor::direct(p(2)),
                ReviewState::NotRequired,
            ),
            meta(
                Source::Automation {
                    run: RunId::from_u128(1),
                },
                Actor::on_behalf(p(2), p(1)),
                ReviewState::NotRequired,
            ),
            meta(
                Source::Formula,
                Actor::direct(p(2)),
                ReviewState::NotRequired,
            ),
            meta(
                ai_source(),
                Actor::on_behalf(p(5), p(1)),
                ReviewState::NotRequired,
            ),
            meta(
                ai_source(),
                Actor::on_behalf(p(5), p(1)),
                ReviewState::Suggested,
            ),
            meta(
                ai_source(),
                Actor::on_behalf(p(5), p(1)),
                ReviewState::Accepted {
                    by: p(1),
                    at: t(1_000),
                },
            ),
            meta(
                ai_source(),
                Actor::on_behalf(p(5), p(1)),
                ReviewState::Rejected {
                    by: p(7),
                    at: t(2_000),
                },
            ),
        ];
        let kinds = [
            "human",
            "import",
            "data_source",
            "automation",
            "formula",
            "ai",
            "ai",
            "ai",
            "ai",
        ];
        for (case, kind) in cases.iter().zip(kinds) {
            case.check().unwrap();
            assert_eq!(case.source.kind(), kind);
            let json = serde_json::to_value(case).unwrap();
            assert_eq!(json["source"]["kind"], kind);
            assert_eq!(&serde_json::from_value::<CellMeta>(json).unwrap(), case);
        }
        assert!(cases[0].is_effective());
        assert!(!cases[6].is_effective());
        assert!(cases[7].is_effective());
        assert!(!cases[8].is_effective());
    }

    #[test]
    fn std_actor_accountability() {
        assert_eq!(Actor::direct(p(1)).accountable(), vec![p(1)]);
        assert_eq!(Actor::on_behalf(p(5), p(1)).accountable(), vec![p(5), p(1)]);
        assert_eq!(Actor::on_behalf(p(1), p(1)).accountable(), vec![p(1)]);
    }

    #[test]
    fn std_review_transitions() {
        let accepted = ReviewState::Suggested.accept(p(1), t(5)).unwrap();
        assert_eq!(accepted, ReviewState::Accepted { by: p(1), at: t(5) });
        let rejected = ReviewState::Suggested.reject(p(1), t(5)).unwrap();
        assert_eq!(rejected, ReviewState::Rejected { by: p(1), at: t(5) });
        assert!(ReviewState::Suggested.is_pending() && !ReviewState::Suggested.is_effective());
        assert!(accepted.is_effective() && !accepted.is_pending());
        assert!(!rejected.is_effective() && !rejected.is_pending());
        assert!(ReviewState::NotRequired.is_effective());
    }

    #[test]
    fn invalid_review_transitions_from_every_other_state() {
        for state in [
            ReviewState::NotRequired,
            ReviewState::Accepted { by: p(1), at: t(1) },
            ReviewState::Rejected { by: p(1), at: t(1) },
        ] {
            for (result, action) in [
                (state.accept(p(2), t(2)), "accept"),
                (state.reject(p(2), t(2)), "reject"),
            ] {
                match result {
                    Err(CoreError::InvalidTransition { from, action: got }) => {
                        assert_eq!(got, action);
                        assert_eq!(from, state.name());
                    }
                    other => panic!("expected InvalidTransition, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn invalid_dual_identity_rules() {
        let m = meta(
            Source::Human,
            Actor::on_behalf(p(1), p(2)),
            ReviewState::NotRequired,
        );
        assert!(reason(m.check()).contains("no initiator"));
        let m = meta(ai_source(), Actor::direct(p(5)), ReviewState::NotRequired);
        assert!(reason(m.check()).contains("must name the initiator"));
    }

    #[test]
    fn invalid_review_rules() {
        let m = meta(Source::Human, Actor::direct(p(1)), ReviewState::Suggested);
        assert!(reason(m.check()).contains("only AI values can be suggestions"));
        let m = meta(
            Source::Formula,
            Actor::direct(p(1)),
            ReviewState::Accepted {
                by: p(2),
                at: t(2_000),
            },
        );
        assert!(reason(m.check()).contains("only AI values can be reviewed"));
        let m = meta(
            ai_source(),
            Actor::on_behalf(p(5), p(1)),
            ReviewState::Accepted {
                by: p(5),
                at: t(2_000),
            },
        );
        assert!(reason(m.check()).contains("cannot review its own"));
        let m = meta(
            ai_source(),
            Actor::on_behalf(p(5), p(1)),
            ReviewState::Rejected {
                by: p(1),
                at: t(999),
            },
        );
        assert!(reason(m.check()).contains("earlier than"));
    }

    #[test]
    fn invalid_ai_details() {
        let with = |edit: &dyn Fn(&mut AiProvenance)| {
            let Source::Ai(mut ai) = ai_source() else {
                unreachable!()
            };
            edit(&mut ai);
            meta(
                Source::Ai(ai),
                Actor::on_behalf(p(5), p(1)),
                ReviewState::Suggested,
            )
        };
        assert!(reason(with(&|ai| ai.model = " ".into()).check())
            .contains("model name must not be empty"));
        assert!(
            reason(with(&|ai| ai.model = "m".repeat(MAX_LABEL_CHARS + 1)).check())
                .contains("longer than")
        );
        assert!(reason(
            with(&|ai| ai.evidence = vec![EvidenceRef::Excerpt {
                text: String::new()
            }])
            .check()
        )
        .contains("excerpt"));
        assert!(reason(
            with(&|ai| ai.evidence = vec![EvidenceRef::Excerpt {
                text: "x".repeat(MAX_EXCERPT_CHARS + 1)
            }])
            .check()
        )
        .contains("excerpt"));
        assert!(reason(
            with(&|ai| ai.evidence = vec![EvidenceRef::Attachment {
                attachment: AttachmentId::from_u128(1),
                page: Some(0)
            }])
            .check()
        )
        .contains("start at 1"));
        assert!(reason(
            with(&|ai| ai.evidence = vec![
                EvidenceRef::Record {
                    record: RecordId::from_u128(1)
                };
                MAX_EVIDENCE + 1
            ])
            .check()
        )
        .contains("too many"));
        with(&|ai| {
            ai.evidence = vec![EvidenceRef::Excerpt {
                text: "é".repeat(MAX_EXCERPT_CHARS),
            }]
        })
        .check()
        .unwrap();
        with(&|ai| {
            ai.confidence = None;
            ai.evidence.clear();
            ai.producer = AiProducer::Agent {
                agent: AgentId::from_u128(2),
                version: 1,
            };
        })
        .check()
        .unwrap();
    }

    #[test]
    fn invalid_external_id() {
        let m = meta(
            Source::DataSource {
                source: DataSourceId::from_u128(1),
                external_id: Some(String::new()),
            },
            Actor::direct(p(1)),
            ReviewState::NotRequired,
        );
        assert!(reason(m.check()).contains("external id must not be empty"));
        let m = meta(
            Source::DataSource {
                source: DataSourceId::from_u128(1),
                external_id: None,
            },
            Actor::direct(p(1)),
            ReviewState::NotRequired,
        );
        m.check().unwrap();
    }
}
