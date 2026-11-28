//! Execution-local accounting for a tree of module calls.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Context, ErrorCode, ExecutionCancelledError, ModuleError};

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// A shareable ledger that bounds how many module attempts may start and their total cost.
#[derive(Debug)]
pub struct CallBudget {
    max_calls: Option<u64>,
    max_cost: Option<u64>,
    estimates: HashMap<String, u64>,
    state: Mutex<BudgetState>,
}

#[derive(Debug, Default)]
struct BudgetState {
    calls_used: u128,
    calls_reserved: u128,
    cost_used: u128,
}

/// A consistent view of the budget's counters.
///
/// Counters above the largest portable integer are displayed at that bound.
/// Accounting and limit decisions continue to use the full totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallBudgetSnapshot {
    pub calls_used: u64,
    pub calls_reserved: u64,
    pub cost_used: u64,
}

impl CallBudget {
    /// Create a budget with optional call and cost limits.
    ///
    /// All limits and estimates are nonnegative safe integers. A module
    /// without an estimate starts with a charge of zero.
    pub fn new(
        max_calls: Option<u64>,
        max_cost: Option<u64>,
        estimates: HashMap<String, u64>,
    ) -> Result<Self, ModuleError> {
        for units in max_calls
            .into_iter()
            .chain(max_cost)
            .chain(estimates.values().copied())
        {
            validate_units(units)?;
        }
        if estimates.keys().any(|id| {
            id.len() > crate::MAX_MODULE_ID_LENGTH || !crate::module_id_pattern().is_match(id)
        }) {
            return Err(ModuleError::invalid_input(
                "Call budget estimate module ID is invalid",
            ));
        }
        Ok(Self {
            max_calls,
            max_cost,
            estimates,
            state: Mutex::new(BudgetState::default()),
        })
    }

    /// Return the current counters without changing them.
    pub fn snapshot(&self) -> CallBudgetSnapshot {
        let state = self.state.lock();
        CallBudgetSnapshot {
            calls_used: portable_counter(state.calls_used),
            calls_reserved: portable_counter(state.calls_reserved),
            cost_used: portable_counter(state.cost_used),
        }
    }

    pub(crate) fn reserve(
        self: &Arc<Self>,
        module_id: &str,
    ) -> Result<BudgetReservation, ModuleError> {
        let mut state = self.state.lock();
        if self
            .max_calls
            .is_some_and(|limit| state.calls_used + state.calls_reserved >= u128::from(limit))
        {
            return Err(exceeded("calls"));
        }
        state.calls_reserved += 1;
        Ok(BudgetReservation {
            permit: Arc::new(BudgetPermit {
                budget: Arc::clone(self),
                module_id: module_id.to_owned(),
                estimate: self.estimates.get(module_id).copied().unwrap_or_default(),
                phase: Mutex::new(AttemptPhase::Reserved),
            }),
        })
    }
}

fn portable_counter(value: u128) -> u64 {
    u64::try_from(value)
        .unwrap_or(MAX_SAFE_INTEGER)
        .min(MAX_SAFE_INTEGER)
}

fn validate_units(units: u64) -> Result<(), ModuleError> {
    if units > MAX_SAFE_INTEGER {
        return Err(ModuleError::invalid_input(
            "Call budget units must be nonnegative safe integers",
        ));
    }
    Ok(())
}

fn exceeded(dimension: &str) -> ModuleError {
    ModuleError::new(
        ErrorCode::CallBudgetExceeded,
        format!("The invocation exceeded its {dimension} budget."),
    )
    .with_details(HashMap::from([("dimension".into(), json!(dimension))]))
}

/// The pipeline owns this reservation until it ends or hands work to a producer.
pub(crate) struct BudgetReservation {
    permit: Arc<BudgetPermit>,
}

impl BudgetReservation {
    pub(crate) fn permit(&self, module_id: &str) -> Option<Arc<BudgetPermit>> {
        (self.permit.module_id == module_id).then(|| Arc::clone(&self.permit))
    }
}

impl Drop for BudgetReservation {
    fn drop(&mut self) {
        let mut phase = self.permit.phase.lock();
        if *phase == AttemptPhase::Reserved {
            self.permit.budget.state.lock().calls_reserved -= 1;
            *phase = AttemptPhase::Closed;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AttemptPhase {
    Reserved,
    Started { reported: bool },
    Closed,
}

pub(crate) struct BudgetPermit {
    budget: Arc<CallBudget>,
    module_id: String,
    estimate: u64,
    phase: Mutex<AttemptPhase>,
}

impl BudgetPermit {
    pub(crate) fn start(self: &Arc<Self>) -> Result<ActiveBudget, ModuleError> {
        let mut phase = self.phase.lock();
        if *phase != AttemptPhase::Reserved {
            return Err(ExecutionCancelledError::without_module(
                "The call no longer has a budget reservation",
            )
            .into());
        }
        let mut state = self.budget.state.lock();
        let cost = state.cost_used + u128::from(self.estimate);
        if self
            .budget
            .max_cost
            .is_some_and(|limit| cost > u128::from(limit))
        {
            return Err(exceeded("cost"));
        }
        state.cost_used = cost;
        state.calls_used += 1;
        state.calls_reserved -= 1;
        *phase = AttemptPhase::Started { reported: false };
        Ok(ActiveBudget {
            permit: Arc::clone(self),
        })
    }

    fn record_cost(&self, actual: u64) -> Result<(), ModuleError> {
        validate_units(actual)?;
        let mut phase = self.phase.lock();
        if *phase != (AttemptPhase::Started { reported: false }) {
            return Err(ModuleError::invalid_input(
                "Actual cost can only be recorded once per active attempt",
            ));
        }
        let mut state = self.budget.state.lock();
        state.cost_used = state.cost_used - u128::from(self.estimate) + u128::from(actual);
        *phase = AttemptPhase::Started { reported: true };
        if self
            .budget
            .max_cost
            .is_some_and(|limit| state.cost_used > u128::from(limit))
        {
            return Err(exceeded("cost"));
        }
        Ok(())
    }
}

/// Keeps cost reporting open for the running handler or stream producer.
pub(crate) struct ActiveBudget {
    permit: Arc<BudgetPermit>,
}

impl ActiveBudget {
    pub(crate) fn bind<T>(&self, context: &mut Context<T>) {
        context.runtime.active_budget = Arc::downgrade(&self.permit);
    }
}

impl Drop for ActiveBudget {
    fn drop(&mut self) {
        *self.permit.phase.lock() = AttemptPhase::Closed;
    }
}

impl<T: Clone> Context<T> {
    /// Derive a context sharing the given call budget.
    #[must_use]
    pub fn with_budget(&self, budget: Option<Arc<CallBudget>>) -> Self {
        let mut derived = self.clone();
        derived.runtime.budget = budget;
        derived.runtime.active_budget = Weak::new();
        derived
    }
}

impl<T> Context<T> {
    /// Replace this invocation's estimated cost with its actual total cost.
    ///
    /// Without a budget this is a no-op. A charged actual amount remains in
    /// the ledger even when it exceeds the cost limit and returns an error.
    pub fn record_cost(&self, actual: u64) -> Result<(), ModuleError> {
        if self.runtime.budget.is_none() {
            return Ok(());
        }
        self.runtime
            .active_budget
            .upgrade()
            .ok_or_else(|| {
                ModuleError::invalid_input("Actual cost requires an active call attempt")
            })?
            .record_cost(actual)
    }
}
