//! The approval engine.
//!
//! An [`ApprovalDef`] is a small flow of steps: approval steps (one after another, all, any
//! one, or at least N approvers), condition branches (first matching case, with a mandatory
//! "other cases"), parallel blocks (join when all, any or N branches finish), fill-in steps
//! (someone completes fields) and notifications.
//!
//! An [`Approval`] is one run of a definition for one record. It is a sans-IO state machine:
//! [`Approval::step`] takes an [`Event`] (someone acts on a task, time passes, the request is
//! withdrawn) with an [`Env`] (the record's values, who holds a capability, now) and returns
//! [`Output`]s (tasks opened and closed, notifications, escalations, the outcome). It never
//! reads a clock, a database or a directory.
//!
//! Rules that always hold:
//! * The requester never approves their own request: they are never resolved as an approver,
//!   and tasks cannot be delegated or added to them.
//! * Only the assignee acts on a task; a rejection needs a reason.
//! * A running approval keeps the definition version it started with.
//! * When an approval ends, no task stays open.

use crate::error::LifecycleError;
use mt_core::condition::{Condition, Context};
use mt_core::field::DataType;
use mt_core::id::{FieldId, PrincipalId};
use mt_core::time::{DurationMs, Timestamp};
use mt_core::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Most steps in one definition, nested ones included.
pub const MAX_NODES: usize = 200;
/// Deepest nesting of branches and parallel blocks.
pub const MAX_NESTING: usize = 8;
/// Most tasks one approval may open (a safeguard against runaway re-assignment).
pub const MAX_TASKS: usize = 1_000;

/// Who acts at a step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Approver {
    /// A person.
    Person {
        /// The person.
        id: PrincipalId,
    },
    /// The people named in a field of the record (a "manager" or "budget owner" field).
    Field {
        /// The field.
        field: FieldId,
    },
    /// Everyone who holds a capability for the record (answered by the access policy).
    Capability {
        /// The capability key.
        capability: String,
    },
}

/// How the approvers of a step decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Mode {
    /// One after another; any rejection rejects.
    Sequential,
    /// All at once; all must approve; any rejection rejects.
    All,
    /// All at once; the first decision, approve or reject, decides.
    Any,
    /// All at once; approved when `required` approve, rejected when that can no longer happen.
    Quorum {
        /// How many approvals are needed.
        required: u16,
    },
}

/// When a parallel block is finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "join", rename_all = "snake_case")]
pub enum Join {
    /// When every branch has finished.
    All,
    /// When one branch has finished.
    Any,
    /// When `count` branches have finished.
    AtLeast {
        /// How many.
        count: u16,
    },
}

/// One case of a condition branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Case {
    /// Name ("Above 10,000").
    pub name: String,
    /// When it applies.
    pub when: Condition,
    /// What happens then.
    pub then: Vec<Node>,
}

/// One step of a definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "node", rename_all = "snake_case")]
pub enum Node {
    /// People approve or reject.
    Approval {
        /// Stable key.
        key: String,
        /// Name ("Controller").
        name: String,
        /// Who approves, in order.
        approvers: Vec<Approver>,
        /// How they decide.
        mode: Mode,
        /// Whether an approver may send the request back to the requester.
        #[serde(default)]
        allow_return: bool,
        /// How long a task may stay open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<DurationMs>,
        /// Who takes over an overdue task (only reported as overdue, if absent).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        escalate_to: Option<Approver>,
    },
    /// The first case whose condition holds runs; otherwise `otherwise`.
    Branch {
        /// Stable key.
        key: String,
        /// Name.
        name: String,
        /// The cases, by priority.
        cases: Vec<Case>,
        /// Other cases.
        otherwise: Vec<Node>,
    },
    /// The branches run at the same time.
    Parallel {
        /// Stable key.
        key: String,
        /// Name.
        name: String,
        /// The branches.
        branches: Vec<Vec<Node>>,
        /// When the block is finished.
        join: Join,
    },
    /// Someone completes fields.
    FillIn {
        /// Stable key.
        key: String,
        /// Name ("Finance enters the payment date").
        name: String,
        /// Who.
        assignee: Approver,
        /// The fields to complete.
        fields: Vec<FieldId>,
    },
    /// People are told; nobody waits.
    Notify {
        /// Stable key.
        key: String,
        /// Name.
        name: String,
        /// Who.
        to: Vec<Approver>,
    },
}

impl Node {
    /// The step's key.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Node::Approval { key, .. }
            | Node::Branch { key, .. }
            | Node::Parallel { key, .. }
            | Node::FillIn { key, .. }
            | Node::Notify { key, .. } => key,
        }
    }

    /// The step's name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Node::Approval { name, .. }
            | Node::Branch { name, .. }
            | Node::Parallel { name, .. }
            | Node::FillIn { name, .. }
            | Node::Notify { name, .. } => name,
        }
    }
}

/// A versioned approval flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDef {
    /// Stable key (`purchase_approval`).
    pub key: String,
    /// Name.
    pub name: String,
    /// Version, from 1; raised by every published change.
    pub version: u32,
    /// The steps, in order.
    pub steps: Vec<Node>,
}

fn invalid(message: impl Into<String>) -> LifecycleError {
    LifecycleError::invalid(message)
}

fn is_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && key.len() <= 64
}

/// A position in the definition: for each level, the step's index and the child list taken
/// (the case of a branch, `cases.len()` for "other cases", or the branch of a parallel block).
type Path = Vec<(usize, usize)>;

impl ApprovalDef {
    fn walk<'d>(nodes: &'d [Node], depth: usize, visit: &mut dyn FnMut(&'d Node, usize)) {
        for node in nodes {
            visit(node, depth);
            match node {
                Node::Branch {
                    cases, otherwise, ..
                } => {
                    for case in cases {
                        Self::walk(&case.then, depth.saturating_add(1), visit);
                    }
                    Self::walk(otherwise, depth.saturating_add(1), visit);
                }
                Node::Parallel { branches, .. } => {
                    for branch in branches {
                        Self::walk(branch, depth.saturating_add(1), visit);
                    }
                }
                Node::Approval { .. } | Node::FillIn { .. } | Node::Notify { .. } => {}
            }
        }
    }

    /// Every step, nested ones included, in order.
    #[must_use]
    pub fn nodes(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        Self::walk(&self.steps, 0, &mut |node, _| out.push(node));
        out
    }

    /// The step with this key.
    #[must_use]
    pub fn node(&self, key: &str) -> Option<&Node> {
        self.nodes().into_iter().find(|node| node.key() == key)
    }

    fn list(&self, path: &[(usize, usize)]) -> Option<&[Node]> {
        let mut list = self.steps.as_slice();
        for (index, child) in path {
            list = match list.get(*index)? {
                Node::Branch {
                    cases, otherwise, ..
                } => match cases.get(*child) {
                    Some(case) => &case.then,
                    None if *child == cases.len() => otherwise,
                    None => return None,
                },
                Node::Parallel { branches, .. } => branches.get(*child)?,
                Node::Approval { .. } | Node::FillIn { .. } | Node::Notify { .. } => return None,
            };
        }
        Some(list)
    }

    /// Verifies the definition while it is drawn: keys, nesting, approvers, quorum sizes,
    /// branches with cases, parallel blocks with at least two branches.
    ///
    /// # Errors
    /// [`LifecycleError::Invalid`] describing the first problem found.
    pub fn check(&self) -> Result<(), LifecycleError> {
        if !is_key(&self.key) {
            return Err(invalid(format!("`{}` is not an approval key", self.key)));
        }
        if self.name.trim().is_empty() {
            return Err(invalid("an approval needs a name"));
        }
        if self.version == 0 {
            return Err(invalid("approval versions start at 1"));
        }
        if self.steps.is_empty() {
            return Err(invalid("an approval needs at least one step"));
        }
        let mut problem: Option<String> = None;
        let mut keys = BTreeSet::new();
        let mut count = 0_usize;
        Self::walk(&self.steps, 1, &mut |node, depth| {
            count = count.saturating_add(1);
            if problem.is_some() {
                return;
            }
            let key = node.key();
            problem = if depth > MAX_NESTING {
                Some(format!("steps are nested at most {MAX_NESTING} deep"))
            } else if !is_key(key) {
                Some(format!("`{key}` is not a step key"))
            } else if !keys.insert(key.to_owned()) {
                Some(format!("the step `{key}` is defined twice"))
            } else if node.name().trim().is_empty() {
                Some(format!("the step `{key}` needs a name"))
            } else {
                match node {
                    Node::Approval {
                        approvers, mode, ..
                    } => {
                        let people = approvers
                            .iter()
                            .all(|a| matches!(a, Approver::Person { .. }));
                        match mode {
                            _ if approvers.is_empty() => {
                                Some(format!("the step `{key}` has no approver"))
                            }
                            Mode::Quorum { required } if *required == 0 => {
                                Some(format!("the step `{key}` needs at least one approval"))
                            }
                            Mode::Quorum { required }
                                if people && usize::from(*required) > approvers.len() =>
                            {
                                Some(format!(
                                    "the step `{key}` needs more approvals than it has approvers"
                                ))
                            }
                            Mode::Sequential | Mode::All | Mode::Any | Mode::Quorum { .. } => None,
                        }
                    }
                    Node::Branch { cases, .. } if cases.is_empty() => {
                        Some(format!("the branch `{key}` needs at least one case"))
                    }
                    Node::Branch { cases, .. } => cases
                        .iter()
                        .find(|c| c.name.trim().is_empty())
                        .map(|_| format!("every case of `{key}` needs a name")),
                    Node::Parallel { branches, .. } if branches.len() < 2 => Some(format!(
                        "the parallel block `{key}` needs at least two branches"
                    )),
                    Node::Parallel {
                        branches,
                        join: Join::AtLeast { count },
                        ..
                    } if *count == 0 || usize::from(*count) > branches.len() => Some(format!(
                        "the parallel block `{key}` waits for 1 to {} branches",
                        branches.len()
                    )),
                    Node::FillIn { fields, .. } if fields.is_empty() => {
                        Some(format!("the fill-in step `{key}` has no field"))
                    }
                    Node::Notify { to, .. } if to.is_empty() => {
                        Some(format!("the notification `{key}` has no recipient"))
                    }
                    Node::Parallel { .. } | Node::FillIn { .. } | Node::Notify { .. } => None,
                }
            };
        });
        if let Some(problem) = problem {
            return Err(invalid(problem));
        }
        if count > MAX_NODES {
            return Err(invalid(format!(
                "an approval has at most {MAX_NODES} steps"
            )));
        }
        Ok(())
    }

    /// Verifies the parts that name fields: conditions, approver fields (people fields) and
    /// fill-in fields. `type_of` returns the data type of a field of the table.
    ///
    /// # Errors
    /// [`LifecycleError::Invalid`].
    pub fn check_in_table(
        &self,
        type_of: &dyn Fn(FieldId) -> Option<DataType>,
    ) -> Result<(), LifecycleError> {
        let approver_ok = |approver: &Approver| match approver {
            Approver::Field { field } => match type_of(*field) {
                Some(DataType::Principal) => Ok(()),
                Some(other) => Err(invalid(format!(
                    "an approver field holds people, not {other} values"
                ))),
                None => Err(invalid(format!("{field} is not a field of the table"))),
            },
            Approver::Person { .. } | Approver::Capability { .. } => Ok(()),
        };
        for node in self.nodes() {
            match node {
                Node::Approval {
                    approvers,
                    escalate_to,
                    ..
                } => {
                    for approver in approvers.iter().chain(escalate_to) {
                        approver_ok(approver)?;
                    }
                }
                Node::Branch { cases, .. } => {
                    for case in cases {
                        case.when
                            .check(type_of)
                            .map_err(|e| invalid(format!("case “{}”: {e}", case.name)))?;
                    }
                }
                Node::FillIn {
                    assignee, fields, ..
                } => {
                    approver_ok(assignee)?;
                    if let Some(field) = fields.iter().find(|f| type_of(**f).is_none()) {
                        return Err(invalid(format!("{field} is not a field of the table")));
                    }
                }
                Node::Notify { to, .. } => {
                    for approver in to {
                        approver_ok(approver)?;
                    }
                }
                Node::Parallel { .. } => {}
            }
        }
        Ok(())
    }
}

/// Who holds a capability for the record: answered by the access policy.
pub trait People {
    /// The holders of a capability, in a stable order.
    fn holders(&self, capability: &str) -> Vec<PrincipalId>;
}

/// What a step needs to know about the world.
#[derive(Clone, Copy)]
pub struct Env<'a> {
    /// Capability holders.
    pub people: &'a dyn People,
    /// The record's values.
    pub value_of: &'a dyn Fn(FieldId) -> Option<&'a Value>,
    /// Today and the time zone, for conditions.
    pub context: &'a dyn Context,
    /// Now.
    pub now: Timestamp,
}

/// Where an approval stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Waiting for people.
    Running,
    /// Every required approval was given.
    Approved,
    /// Someone rejected it.
    Rejected,
    /// Sent back to the requester.
    Returned,
    /// Withdrawn or stopped.
    Cancelled,
    /// A step resolved to nobody; an administrator must cancel or fix it.
    Stuck,
}

impl Status {
    /// Whether the approval has ended (stuck ones wait for an administrator).
    #[must_use]
    pub const fn is_final(self) -> bool {
        match self {
            Status::Approved | Status::Rejected | Status::Returned | Status::Cancelled => true,
            Status::Running | Status::Stuck => false,
        }
    }
}

/// What a task asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Approve or reject.
    Approve,
    /// Complete fields.
    FillIn,
}

/// Where a task stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Waiting for its assignee.
    Open,
    /// Approved.
    Approved,
    /// Rejected.
    Rejected,
    /// Sent back to the requester.
    Returned,
    /// Fields completed.
    Done,
    /// No longer needed.
    Cancelled,
}

/// One person's task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Number within the approval, from 1.
    pub id: u32,
    /// The step.
    pub node: String,
    /// What it asks for.
    pub kind: TaskKind,
    /// Who acts.
    pub assignee: PrincipalId,
    /// Who it was delegated or escalated from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated_from: Option<PrincipalId>,
    /// Where it stands.
    pub status: TaskStatus,
    /// When it opened.
    pub opened_at: Timestamp,
    /// When it is due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<Timestamp>,
    /// Whether it was escalated (or reported overdue) already.
    #[serde(default)]
    pub escalated: bool,
    /// When it was decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acted_at: Option<Timestamp>,
    /// The assignee's comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// What someone does with a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "act", rename_all = "snake_case")]
pub enum Act {
    /// Approve.
    Approve {
        /// Optional comment.
        #[serde(default)]
        comment: Option<String>,
    },
    /// Reject, with a reason.
    Reject {
        /// Why.
        comment: String,
    },
    /// Send back to the requester, with a reason (where the step allows it).
    Return {
        /// Why.
        comment: String,
    },
    /// Complete a fill-in step (the caller saved the fields).
    Submit,
    /// Hand the task to someone else.
    Delegate {
        /// To whom.
        to: PrincipalId,
    },
    /// Ask someone else to approve this step as well.
    AddApprover {
        /// Whom.
        who: PrincipalId,
    },
    /// Comment without deciding.
    Comment {
        /// The comment.
        text: String,
    },
}

/// What happens to an approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Someone acts on a task.
    Act {
        /// The task.
        task: u32,
        /// Who acts.
        actor: PrincipalId,
        /// What they do.
        act: Act,
    },
    /// Time passed: escalate or report overdue tasks.
    Tick,
    /// The request is withdrawn or stopped.
    Cancel {
        /// By whom.
        by: PrincipalId,
    },
}

/// What an approval reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "output", rename_all = "snake_case")]
pub enum Output {
    /// A task opened.
    Opened {
        /// The task.
        task: u32,
        /// Its assignee.
        assignee: PrincipalId,
        /// Its step.
        node: String,
    },
    /// A task closed.
    Closed {
        /// The task.
        task: u32,
        /// How.
        status: TaskStatus,
    },
    /// People are to be told.
    Notified {
        /// The step.
        node: String,
        /// Whom.
        to: Vec<PrincipalId>,
    },
    /// A task was handed to someone else.
    Reassigned {
        /// The task.
        task: u32,
        /// From.
        from: PrincipalId,
        /// To.
        to: PrincipalId,
        /// Whether it was overdue (escalation) rather than delegated.
        escalated: bool,
    },
    /// A task is overdue and nobody takes over.
    Overdue {
        /// The task.
        task: u32,
    },
    /// A step resolved to nobody.
    Stuck {
        /// The step.
        node: String,
    },
    /// The approval ended.
    Finished {
        /// How.
        status: Status,
    },
}

/// One line of the approval's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// When.
    pub at: Timestamp,
    /// Who.
    pub actor: PrincipalId,
    /// The task, if any.
    pub task: Option<u32>,
    /// What happened, in words.
    pub what: String,
}

/// The progress of a running part.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Progress {
    Done,
    Waiting,
    Rejected,
    Returned,
    Stuck(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "active", rename_all = "snake_case")]
enum Active {
    Approval {
        /// Approvers not yet asked (sequential mode), in order.
        queue: Vec<PrincipalId>,
        /// The step's tasks.
        tasks: Vec<u32>,
    },
    FillIn {
        task: u32,
    },
    Branch {
        seq: Box<Seq>,
    },
    Parallel {
        branches: Vec<Seq>,
    },
}

/// A sequence of steps being run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Seq {
    path: Path,
    index: usize,
    active: Option<Active>,
}

impl Seq {
    fn new(path: Path) -> Self {
        Seq {
            path,
            index: 0,
            active: None,
        }
    }

    /// Every task opened under this sequence's running part.
    fn tasks(&self, out: &mut Vec<u32>) {
        match &self.active {
            Some(Active::Approval { tasks, .. }) => out.extend(tasks.iter().copied()),
            Some(Active::FillIn { task }) => out.push(*task),
            Some(Active::Branch { seq }) => seq.tasks(out),
            Some(Active::Parallel { branches }) => {
                for branch in branches {
                    branch.tasks(out);
                }
            }
            None => {}
        }
    }
}

/// One run of an approval definition for one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    /// The definition's key.
    pub def: String,
    /// The definition's version (kept while running).
    pub version: u32,
    /// Who asked.
    pub requester: PrincipalId,
    /// Where it stands.
    pub status: Status,
    /// Every task, in the order opened.
    pub tasks: Vec<Task>,
    /// What happened.
    pub history: Vec<Entry>,
    /// When it started.
    pub started_at: Timestamp,
    /// When it ended.
    pub ended_at: Option<Timestamp>,
    root: Seq,
}

struct Runner<'d, 'e> {
    def: &'d ApprovalDef,
    env: Env<'e>,
    requester: PrincipalId,
    tasks: &'d mut Vec<Task>,
    outputs: Vec<Output>,
}

impl Runner<'_, '_> {
    /// The people behind `approvers`, without duplicates; without the requester when they
    /// would approve their own request.
    fn resolve(&self, approvers: &[Approver], exclude_requester: bool) -> Vec<PrincipalId> {
        let mut out: Vec<PrincipalId> = Vec::new();
        for approver in approvers {
            let found = match approver {
                Approver::Person { id } => vec![*id],
                Approver::Field { field } => match (self.env.value_of)(*field) {
                    Some(Value::Principal(people)) => people.clone(),
                    Some(_) | None => Vec::new(),
                },
                Approver::Capability { capability } => self.env.people.holders(capability),
            };
            for person in found {
                if !(exclude_requester && person == self.requester) && !out.contains(&person) {
                    out.push(person);
                }
            }
        }
        out
    }

    fn open(&mut self, node: &Node, kind: TaskKind, assignee: PrincipalId) -> Option<u32> {
        if self.tasks.len() >= MAX_TASKS {
            return None;
        }
        let id = u32::try_from(self.tasks.len()).ok()?.checked_add(1)?;
        let due_at = match node {
            Node::Approval {
                timeout: Some(t), ..
            } => self.env.now.checked_add(*t),
            Node::Approval { timeout: None, .. }
            | Node::Branch { .. }
            | Node::Parallel { .. }
            | Node::FillIn { .. }
            | Node::Notify { .. } => None,
        };
        self.tasks.push(Task {
            id,
            node: node.key().to_owned(),
            kind,
            assignee,
            delegated_from: None,
            status: TaskStatus::Open,
            opened_at: self.env.now,
            due_at,
            escalated: false,
            acted_at: None,
            comment: None,
        });
        self.outputs.push(Output::Opened {
            task: id,
            assignee,
            node: node.key().to_owned(),
        });
        Some(id)
    }

    fn task(&self, id: u32) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    fn close_open(&mut self, ids: &[u32]) {
        for task in self.tasks.iter_mut() {
            if ids.contains(&task.id) && task.status == TaskStatus::Open {
                task.status = TaskStatus::Cancelled;
                self.outputs.push(Output::Closed {
                    task: task.id,
                    status: TaskStatus::Cancelled,
                });
            }
        }
    }

    /// Runs a sequence as far as it goes.
    fn advance(&mut self, seq: &mut Seq) -> Progress {
        loop {
            if let Some(active) = seq.active.as_mut() {
                let node = self
                    .def
                    .list(&seq.path)
                    .and_then(|list| list.get(seq.index))
                    .cloned();
                let Some(node) = node else {
                    return Progress::Stuck(
                        "the definition changed under a running approval".to_owned(),
                    );
                };
                let progress = self.evaluate(&node, active);
                match progress {
                    Progress::Done => {
                        seq.active = None;
                        seq.index = seq.index.saturating_add(1);
                        continue;
                    }
                    Progress::Waiting
                    | Progress::Rejected
                    | Progress::Returned
                    | Progress::Stuck(_) => return progress,
                }
            }
            let Some(node) = self
                .def
                .list(&seq.path)
                .and_then(|list| list.get(seq.index))
                .cloned()
            else {
                return Progress::Done;
            };
            match &node {
                Node::Notify { key, to, .. } => {
                    let people = self.resolve(to, false);
                    self.outputs.push(Output::Notified {
                        node: key.clone(),
                        to: people,
                    });
                    seq.index = seq.index.saturating_add(1);
                }
                Node::Approval {
                    key,
                    approvers,
                    mode,
                    ..
                } => {
                    let people = self.resolve(approvers, true);
                    let enough = match mode {
                        Mode::Quorum { required } => people.len() >= usize::from(*required),
                        Mode::Sequential | Mode::All | Mode::Any => !people.is_empty(),
                    };
                    if !enough {
                        self.outputs.push(Output::Stuck { node: key.clone() });
                        return Progress::Stuck(format!("the step `{key}` has no one to approve"));
                    }
                    let (now, later) = match mode {
                        Mode::Sequential => {
                            let mut first = people;
                            let later = first.split_off(first.len().min(1));
                            (first, later)
                        }
                        Mode::All | Mode::Any | Mode::Quorum { .. } => (people, Vec::new()),
                    };
                    let mut tasks = Vec::new();
                    for person in now {
                        if let Some(id) = self.open(&node, TaskKind::Approve, person) {
                            tasks.push(id);
                        }
                    }
                    seq.active = Some(Active::Approval {
                        queue: later,
                        tasks,
                    });
                }
                Node::FillIn { key, assignee, .. } => {
                    let people = self.resolve(std::slice::from_ref(assignee), false);
                    let Some(person) = people.first().copied() else {
                        self.outputs.push(Output::Stuck { node: key.clone() });
                        return Progress::Stuck(format!(
                            "the step `{key}` has no one to fill it in"
                        ));
                    };
                    match self.open(&node, TaskKind::FillIn, person) {
                        Some(task) => seq.active = Some(Active::FillIn { task }),
                        None => return Progress::Stuck("too many tasks".to_owned()),
                    }
                }
                Node::Branch { cases, .. } => {
                    let chosen = cases
                        .iter()
                        .position(|case| case.when.evaluate(self.env.value_of, self.env.context))
                        .unwrap_or(cases.len());
                    let mut path = seq.path.clone();
                    path.push((seq.index, chosen));
                    seq.active = Some(Active::Branch {
                        seq: Box::new(Seq::new(path)),
                    });
                }
                Node::Parallel { branches, .. } => {
                    let seqs = (0..branches.len())
                        .map(|branch| {
                            let mut path = seq.path.clone();
                            path.push((seq.index, branch));
                            Seq::new(path)
                        })
                        .collect();
                    seq.active = Some(Active::Parallel { branches: seqs });
                }
            }
        }
    }

    /// Whether a running part has finished, and moves it on (next sequential approver,
    /// branches of a branch or parallel block).
    fn evaluate(&mut self, node: &Node, active: &mut Active) -> Progress {
        match active {
            Active::Approval { queue, tasks } => {
                let Node::Approval { mode, .. } = node else {
                    return Progress::Stuck("the step changed kind".to_owned());
                };
                let statuses: Vec<TaskStatus> = tasks
                    .iter()
                    .filter_map(|id| self.task(*id).map(|t| t.status))
                    .collect();
                let approved = statuses
                    .iter()
                    .filter(|s| **s == TaskStatus::Approved)
                    .count();
                let rejected = statuses
                    .iter()
                    .filter(|s| **s == TaskStatus::Rejected)
                    .count();
                let open = statuses.iter().filter(|s| **s == TaskStatus::Open).count();
                let total = statuses.len().saturating_add(queue.len());
                let progress = if statuses.contains(&TaskStatus::Returned) {
                    Progress::Returned
                } else {
                    match mode {
                        Mode::Sequential | Mode::All if rejected > 0 => Progress::Rejected,
                        Mode::Sequential if open == 0 => {
                            if queue.is_empty() {
                                Progress::Done
                            } else {
                                let next = queue.remove(0);
                                match self.open(node, TaskKind::Approve, next) {
                                    Some(id) => {
                                        tasks.push(id);
                                        Progress::Waiting
                                    }
                                    None => Progress::Stuck("too many tasks".to_owned()),
                                }
                            }
                        }
                        Mode::All if approved == total => Progress::Done,
                        Mode::Any if approved > 0 => Progress::Done,
                        Mode::Any if rejected > 0 => Progress::Rejected,
                        Mode::Quorum { required } if approved >= usize::from(*required) => {
                            Progress::Done
                        }
                        Mode::Quorum { required }
                            if rejected > total.saturating_sub(usize::from(*required)) =>
                        {
                            Progress::Rejected
                        }
                        Mode::Sequential | Mode::All | Mode::Any | Mode::Quorum { .. } => {
                            Progress::Waiting
                        }
                    }
                };
                if !matches!(progress, Progress::Waiting) {
                    let ids = tasks.clone();
                    self.close_open(&ids);
                }
                progress
            }
            Active::FillIn { task } => match self.task(*task).map(|t| t.status) {
                Some(TaskStatus::Done) => Progress::Done,
                Some(TaskStatus::Open) => Progress::Waiting,
                Some(
                    TaskStatus::Approved
                    | TaskStatus::Rejected
                    | TaskStatus::Returned
                    | TaskStatus::Cancelled,
                )
                | None => Progress::Stuck("the fill-in task ended unexpectedly".to_owned()),
            },
            Active::Branch { seq } => self.advance(seq),
            Active::Parallel { branches } => {
                let Node::Parallel { join, .. } = node else {
                    return Progress::Stuck("the step changed kind".to_owned());
                };
                let mut done = 0_usize;
                let mut ended: Option<Progress> = None;
                for branch in branches.iter_mut() {
                    match self.advance(branch) {
                        Progress::Done => done = done.saturating_add(1),
                        Progress::Waiting => {}
                        other @ (Progress::Rejected | Progress::Returned | Progress::Stuck(_)) => {
                            ended.get_or_insert(other);
                        }
                    }
                }
                let needed = match join {
                    Join::All => branches.len(),
                    Join::Any => 1,
                    Join::AtLeast { count } => usize::from(*count),
                };
                let progress = match ended {
                    Some(other) => other,
                    None if done >= needed => Progress::Done,
                    None => Progress::Waiting,
                };
                if !matches!(progress, Progress::Waiting) {
                    let mut ids = Vec::new();
                    for branch in branches.iter() {
                        branch.tasks(&mut ids);
                    }
                    self.close_open(&ids);
                }
                progress
            }
        }
    }
}

impl Approval {
    /// Starts an approval for a record.
    ///
    /// # Errors
    /// [`LifecycleError::Invalid`] for an invalid definition.
    pub fn start(
        def: &ApprovalDef,
        requester: PrincipalId,
        env: Env<'_>,
    ) -> Result<(Approval, Vec<Output>), LifecycleError> {
        def.check()?;
        let mut approval = Approval {
            def: def.key.clone(),
            version: def.version,
            requester,
            status: Status::Running,
            tasks: Vec::new(),
            history: vec![Entry {
                at: env.now,
                actor: requester,
                task: None,
                what: format!("requested approval ({} v{})", def.name, def.version),
            }],
            started_at: env.now,
            ended_at: None,
            root: Seq::new(Vec::new()),
        };
        let outputs = approval.run(def, env);
        Ok((approval, outputs))
    }

    fn run(&mut self, def: &ApprovalDef, env: Env<'_>) -> Vec<Output> {
        let mut root = core::mem::replace(&mut self.root, Seq::new(Vec::new()));
        let mut runner = Runner {
            def,
            env,
            requester: self.requester,
            tasks: &mut self.tasks,
            outputs: Vec::new(),
        };
        let progress = runner.advance(&mut root);
        let mut outputs = runner.outputs;
        self.root = root;
        let status = match progress {
            Progress::Done => Status::Approved,
            Progress::Waiting => Status::Running,
            Progress::Rejected => Status::Rejected,
            Progress::Returned => Status::Returned,
            Progress::Stuck(_) => Status::Stuck,
        };
        if status != self.status {
            self.status = status;
            if status.is_final() {
                self.ended_at = Some(env.now);
                outputs.extend(self.close_all());
                outputs.push(Output::Finished { status });
            }
        }
        outputs
    }

    fn close_all(&mut self) -> Vec<Output> {
        let mut outputs = Vec::new();
        for task in &mut self.tasks {
            if task.status == TaskStatus::Open {
                task.status = TaskStatus::Cancelled;
                outputs.push(Output::Closed {
                    task: task.id,
                    status: TaskStatus::Cancelled,
                });
            }
        }
        outputs
    }

    /// The open tasks.
    #[must_use]
    pub fn open_tasks(&self) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Open)
            .collect()
    }

    /// The open tasks of one person.
    #[must_use]
    pub fn tasks_of(&self, person: PrincipalId) -> Vec<&Task> {
        self.open_tasks()
            .into_iter()
            .filter(|t| t.assignee == person)
            .collect()
    }

    /// Applies an event.
    ///
    /// # Errors
    /// [`LifecycleError::Version`] for a definition of another version;
    /// [`LifecycleError::State`] when the approval has ended or the task is not open;
    /// [`LifecycleError::NotAllowed`] when the actor is not the assignee, the act does not fit
    /// the task, a rejection or return has no reason, or the new person is the requester.
    pub fn step(
        &mut self,
        def: &ApprovalDef,
        event: Event,
        env: Env<'_>,
    ) -> Result<Vec<Output>, LifecycleError> {
        if def.key != self.def {
            return Err(LifecycleError::state(format!(
                "this approval runs `{}`, not `{}`",
                self.def, def.key
            )));
        }
        if def.version != self.version {
            return Err(LifecycleError::Version {
                running: self.version,
                given: def.version,
            });
        }
        if self.status.is_final() {
            return Err(LifecycleError::state("the approval has ended"));
        }
        let mut outputs = Vec::new();
        match event {
            Event::Cancel { by } => {
                self.status = Status::Cancelled;
                self.ended_at = Some(env.now);
                self.history.push(Entry {
                    at: env.now,
                    actor: by,
                    task: None,
                    what: "cancelled".to_owned(),
                });
                outputs.extend(self.close_all());
                outputs.push(Output::Finished {
                    status: Status::Cancelled,
                });
                return Ok(outputs);
            }
            Event::Tick => {
                if self.status == Status::Stuck {
                    return Ok(outputs);
                }
                outputs.extend(self.tick(def, env));
            }
            Event::Act { task, actor, act } => {
                if self.status == Status::Stuck {
                    return Err(LifecycleError::state(
                        "the approval is stuck; an administrator must cancel it",
                    ));
                }
                outputs.extend(self.act(def, task, actor, act, env)?);
            }
        }
        outputs.extend(self.run(def, env));
        Ok(outputs)
    }

    fn act(
        &mut self,
        def: &ApprovalDef,
        id: u32,
        actor: PrincipalId,
        act: Act,
        env: Env<'_>,
    ) -> Result<Vec<Output>, LifecycleError> {
        let requester = self.requester;
        let node_key = self
            .tasks
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.node.clone())
            .ok_or_else(|| LifecycleError::state(format!("there is no task {id}")))?;
        let asked = self.asked(def, &node_key);
        let task = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| LifecycleError::state(format!("there is no task {id}")))?;
        if task.status != TaskStatus::Open {
            return Err(LifecycleError::state(format!("task {id} is closed")));
        }
        if task.assignee != actor {
            return Err(LifecycleError::NotAllowed(format!(
                "task {id} is not yours"
            )));
        }
        let node = def
            .node(&task.node)
            .ok_or_else(|| LifecycleError::state("the step no longer exists"))?;
        let allow_return = matches!(
            node,
            Node::Approval {
                allow_return: true,
                ..
            }
        );
        let reason = |text: &str| -> Result<String, LifecycleError> {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Err(LifecycleError::NotAllowed("give a reason".to_owned()))
            } else {
                Ok(trimmed.to_owned())
            }
        };
        let mut outputs = Vec::new();
        let decide = |task: &mut Task, status: TaskStatus, comment: Option<String>| {
            task.status = status;
            task.acted_at = Some(env.now);
            task.comment = comment;
        };
        let what = match (&act, task.kind) {
            (Act::Approve { comment }, TaskKind::Approve) => {
                let comment = comment.clone().filter(|c| !c.trim().is_empty());
                decide(task, TaskStatus::Approved, comment);
                "approved".to_owned()
            }
            (Act::Reject { comment }, TaskKind::Approve) => {
                let comment = reason(comment)?;
                decide(task, TaskStatus::Rejected, Some(comment));
                "rejected".to_owned()
            }
            (Act::Return { comment }, TaskKind::Approve) => {
                if !allow_return {
                    return Err(LifecycleError::NotAllowed(
                        "this step cannot send requests back".to_owned(),
                    ));
                }
                let comment = reason(comment)?;
                decide(task, TaskStatus::Returned, Some(comment));
                "sent back to the requester".to_owned()
            }
            (Act::Submit, TaskKind::FillIn) => {
                decide(task, TaskStatus::Done, None);
                "completed the fields".to_owned()
            }
            (Act::Delegate { to }, TaskKind::Approve | TaskKind::FillIn) => {
                let own_request = *to == requester && task.kind == TaskKind::Approve;
                if own_request || *to == actor || asked.contains(to) {
                    return Err(LifecycleError::NotAllowed(
                        "delegate to someone else who is not the requester and not already asked"
                            .to_owned(),
                    ));
                }
                task.delegated_from = Some(task.assignee);
                task.assignee = *to;
                outputs.push(Output::Reassigned {
                    task: id,
                    from: actor,
                    to: *to,
                    escalated: false,
                });
                format!("delegated to {to}")
            }
            (Act::AddApprover { who }, TaskKind::Approve) => {
                if *who == requester || asked.contains(who) {
                    return Err(LifecycleError::NotAllowed(
                        "add someone who is not the requester and not already asked".to_owned(),
                    ));
                }
                self.add_approver(def, &node_key, *who, env, &mut outputs)?;
                format!("added {who} as an approver")
            }
            (Act::Comment { text }, TaskKind::Approve | TaskKind::FillIn) => {
                let text = reason(text)?;
                format!("commented: {text}")
            }
            (
                Act::Approve { .. }
                | Act::Reject { .. }
                | Act::Return { .. }
                | Act::AddApprover { .. },
                TaskKind::FillIn,
            )
            | (Act::Submit, TaskKind::Approve) => {
                return Err(LifecycleError::NotAllowed(
                    "that does not fit this task".to_owned(),
                ));
            }
        };
        if let Some(task) = self.tasks.iter().find(|t| t.id == id) {
            if task.status != TaskStatus::Open {
                outputs.push(Output::Closed {
                    task: id,
                    status: task.status,
                });
            }
        }
        self.history.push(Entry {
            at: env.now,
            actor,
            task: Some(id),
            what,
        });
        Ok(outputs)
    }

    fn add_approver(
        &mut self,
        def: &ApprovalDef,
        node_key: &str,
        who: PrincipalId,
        env: Env<'_>,
        outputs: &mut Vec<Output>,
    ) -> Result<(), LifecycleError> {
        let node = def
            .node(node_key)
            .cloned()
            .ok_or_else(|| LifecycleError::state("the step no longer exists"))?;
        let mut root = core::mem::replace(&mut self.root, Seq::new(Vec::new()));
        let mut opened = true;
        let found = {
            let mut runner = Runner {
                def,
                env,
                requester: self.requester,
                tasks: &mut self.tasks,
                outputs: Vec::new(),
            };
            let found = Self::find_step(&mut root, def, node_key, &mut |active| match active {
                Active::Approval { queue, tasks } => {
                    let sequential = matches!(
                        node,
                        Node::Approval {
                            mode: Mode::Sequential,
                            ..
                        }
                    );
                    if sequential {
                        queue.insert(0, who);
                    } else if let Some(id) = runner.open(&node, TaskKind::Approve, who) {
                        tasks.push(id);
                    } else {
                        opened = false;
                    }
                }
                Active::FillIn { .. } | Active::Branch { .. } | Active::Parallel { .. } => {}
            });
            outputs.extend(runner.outputs);
            found
        };
        self.root = root;
        if !found {
            Err(LifecycleError::state("the step is not running"))
        } else if !opened {
            Err(LifecycleError::NotAllowed(format!(
                "an approval opens at most {MAX_TASKS} tasks"
            )))
        } else {
            Ok(())
        }
    }

    /// Everyone asked at a step: assignees of its tasks that were not cancelled, and the
    /// approvers still waiting for their turn in a sequential step.
    fn asked(&self, def: &ApprovalDef, key: &str) -> Vec<PrincipalId> {
        let mut people: Vec<PrincipalId> = self
            .tasks
            .iter()
            .filter(|t| t.node == key && t.status != TaskStatus::Cancelled)
            .map(|t| t.assignee)
            .collect();
        people.extend(Self::queue_of(&self.root, def, key));
        people
    }

    /// The waiting approvers of the running step `key`.
    fn queue_of(seq: &Seq, def: &ApprovalDef, key: &str) -> Vec<PrincipalId> {
        let here = def
            .list(&seq.path)
            .and_then(|list| list.get(seq.index))
            .is_some_and(|node| node.key() == key);
        match &seq.active {
            Some(Active::Approval { queue, .. }) if here => queue.clone(),
            Some(Active::Branch { seq: inner }) => Self::queue_of(inner, def, key),
            Some(Active::Parallel { branches }) => branches
                .iter()
                .flat_map(|b| Self::queue_of(b, def, key))
                .collect(),
            Some(Active::Approval { .. } | Active::FillIn { .. }) | None => Vec::new(),
        }
    }

    /// Finds the running part of the step `key` and applies `change` to it.
    fn find_step(
        seq: &mut Seq,
        def: &ApprovalDef,
        key: &str,
        change: &mut dyn FnMut(&mut Active),
    ) -> bool {
        let here = def
            .list(&seq.path)
            .and_then(|list| list.get(seq.index))
            .is_some_and(|node| node.key() == key);
        match seq.active.as_mut() {
            Some(active) if here => {
                change(active);
                true
            }
            Some(Active::Branch { seq: inner }) => Self::find_step(inner, def, key, change),
            Some(Active::Parallel { branches }) => branches
                .iter_mut()
                .any(|b| Self::find_step(b, def, key, change)),
            Some(Active::Approval { .. } | Active::FillIn { .. }) | None => false,
        }
    }

    fn tick(&mut self, def: &ApprovalDef, env: Env<'_>) -> Vec<Output> {
        let mut outputs = Vec::new();
        let overdue: Vec<u32> = self
            .tasks
            .iter()
            .filter(|t| {
                t.status == TaskStatus::Open
                    && !t.escalated
                    && t.due_at.is_some_and(|d| d <= env.now)
            })
            .map(|t| t.id)
            .collect();
        // One at a time: each escalation sees who the previous ones handed tasks to.
        for id in overdue {
            let Some(key) = self
                .tasks
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.node.clone())
            else {
                continue;
            };
            let target = match def.node(&key) {
                Some(Node::Approval {
                    escalate_to: Some(approver),
                    ..
                }) => {
                    let asked = self.asked(def, &key);
                    let mut scratch = Vec::new();
                    let runner = Runner {
                        def,
                        env,
                        requester: self.requester,
                        tasks: &mut scratch,
                        outputs: Vec::new(),
                    };
                    runner
                        .resolve(std::slice::from_ref(approver), true)
                        .into_iter()
                        .find(|p| !asked.contains(p))
                }
                Some(_) | None => None,
            };
            let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
                continue;
            };
            task.escalated = true;
            match target {
                Some(to) => {
                    let from = task.assignee;
                    task.delegated_from = Some(from);
                    task.assignee = to;
                    outputs.push(Output::Reassigned {
                        task: id,
                        from,
                        to,
                        escalated: true,
                    });
                    self.history.push(Entry {
                        at: env.now,
                        actor: from,
                        task: Some(id),
                        what: format!("overdue: escalated to {to}"),
                    });
                }
                None => outputs.push(Output::Overdue { task: id }),
            }
        }
        outputs
    }
}

/// One step a simulation passed through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulatedStep {
    /// The step.
    pub node: String,
    /// Who would be asked.
    pub people: Vec<PrincipalId>,
}

/// Steps through a definition for a record as if everyone approved: who would be asked, in
/// order, and how it would end (the designer's "simulate" button).
///
/// # Errors
/// [`LifecycleError::Invalid`] for an invalid definition.
pub fn simulate(
    def: &ApprovalDef,
    requester: PrincipalId,
    env: Env<'_>,
) -> Result<(Vec<SimulatedStep>, Status), LifecycleError> {
    let (mut approval, _) = Approval::start(def, requester, env)?;
    let mut steps: Vec<SimulatedStep> = Vec::new();
    let mut seen = 0_usize;
    for _ in 0..MAX_TASKS {
        for task in approval.tasks.iter().skip(seen) {
            match steps.last_mut() {
                Some(step) if step.node == task.node => step.people.push(task.assignee),
                Some(_) | None => steps.push(SimulatedStep {
                    node: task.node.clone(),
                    people: vec![task.assignee],
                }),
            }
        }
        seen = approval.tasks.len();
        let Some((id, assignee, kind)) = approval
            .open_tasks()
            .first()
            .map(|t| (t.id, t.assignee, t.kind))
        else {
            break;
        };
        let act = match kind {
            TaskKind::Approve => Act::Approve { comment: None },
            TaskKind::FillIn => Act::Submit,
        };
        approval.step(
            def,
            Event::Act {
                task: id,
                actor: assignee,
                act,
            },
            env,
        )?;
    }
    Ok((steps, approval.status))
}
