//! Process (product design doc 17: domain D7, capability C6, processes P5 and P6).
//!
//! * [`lifecycle`] — states with categories, guarded transitions, required and editable
//!   fields, and the closed-loop check (no record can get stuck by design).
//! * [`calendar`] — business calendars: working hours, days off, deadlines in working time.
//! * [`sla`] — SLA clocks with pauses, warnings and breaches (a sans-IO state machine).
//! * [`approval`] — the approval engine: sequential / all / any / quorum steps, priority
//!   branches, parallel blocks, fill-in steps, notifications, delegation, added approvers,
//!   send-back, escalation, versions, and simulation (a sans-IO state machine).
//!
//! Pure: no clock, no I/O; time and the world are inputs. Builds for WebAssembly.
#![forbid(unsafe_code)]

pub mod approval;
pub mod calendar;
pub mod error;
pub mod lifecycle;
pub mod sla;

pub use approval::{
    simulate, Act, Approval, ApprovalDef, Approver, Case, Entry, Env, Event, Join, Mode, Node,
    Output, People, SimulatedStep, Status, Task, TaskKind, TaskStatus,
};
pub use calendar::BusinessCalendar;
pub use error::LifecycleError;
pub use lifecycle::{Blocker, Lifecycle, Move, Offer, StateDef, Transition};
pub use sla::{ClockState, SlaClock, SlaEvent, SlaInput, SlaPolicy};
