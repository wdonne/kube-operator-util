//! This is a utility to manage the status field of a Kubernetes resource. It maintains the last
//! five condition objects. It also updates the field `health`, with its subfield `status`, which
//! has the predefined values `Healthy`, `Unhealthy` and `Unknown`. The field `phase` has the
//! predefined values `Pending` and `Ready`. It is allowed to use other values for the fields.
use crate::util::simple_patch_params;
use chrono::{DateTime, SecondsFormat, Utc};
use kube::Api;
use kube_core::ResourceExt;
use kube_core::params::Patch;
use schemars::_private::serde_json::json;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::cmp;
use std::fmt::Debug;

pub const ERROR: &str = "Error";
pub const FALSE: &str = "False";
pub const HEALTHY: &str = "Healthy";
pub const OK: &str = "OK";
pub const PENDING: &str = "Pending";
pub const READY: &str = "Ready";
pub const TRUE: &str = "True";
pub const UNHEALTHY: &str = "Unhealthy";
pub const UNKNOWN: &str = "Unknown";

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    #[serde(rename = "type")]
    condition_type: String,
    last_transition_time: String,
    message: String,
    reason: String,
    status: String,
}

pub trait GetStatus {
    fn status(&self) -> Option<&Status>;
}

pub trait Patchable: Clone + DeserializeOwned + Debug + ResourceExt + GetStatus + 'static {}

impl<T> Patchable for T where T: Clone + DeserializeOwned + Debug + ResourceExt + GetStatus + 'static
{}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct Health {
    status: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct Status {
    conditions: Vec<Condition>,
    health: Health,
    phase: String,
}

fn add_condition(conditions: &[Condition], condition: &Condition) -> Vec<Condition> {
    let mut last: Vec<Condition> = at_most_last_four(conditions).to_vec();

    last.push(condition.clone());
    last
}

fn at_most_last_four(v: &[Condition]) -> &[Condition] {
    if v.len() < 5 {
        v
    } else {
        &v[cmp::max(0, v.len() - 4)..v.len()]
    }
}

/// Creates a default condition, which will indicate readiness.
pub fn condition() -> Condition {
    Condition {
        condition_type: READY.to_string(),
        last_transition_time: now(),
        message: OK.to_string(),
        reason: OK.to_string(),
        status: TRUE.to_string(),
    }
}

/// Creates a condition that indicates an error state.
pub fn error_condition(message: &str) -> Condition {
    Condition {
        condition_type: READY.to_string(),
        last_transition_time: now(),
        message: message.to_string(),
        reason: ERROR.to_string(),
        status: FALSE.to_string(),
    }
}

/// Creates a health object with the status `Healthy`.
pub fn healthy() -> Health {
    Health {
        status: HEALTHY.to_string(),
    }
}

pub fn is_not_ready(current_status: Option<&Status>) -> bool {
    current_status.is_none_or(|s| !s.is_ready())
}

pub fn next_status(current_status: Option<&Status>, error: Option<&str>) -> Status {
    error.map_or_else(
        || set_ready(current_status),
        |e| set_error(current_status, e),
    )
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub async fn patch_status<T>(
    api: &Api<T>,
    obj: &T,
    error: Option<&str>,
    field_manager: &str,
) -> Result<T, kube::Error>
where
    T: Patchable,
{
    api.patch_status(
        &obj.name_any(),
        &simple_patch_params(field_manager),
        &Patch::Merge(&json!({"status": next_status(obj.status(), error)})),
    )
    .await
}

/// Updates the given status with an error condition or creates a new status object with the
/// error condition. The status will be unhealthy and the phase will be pending.
/// ```rust
/// use kube_operator_util::status;
///
/// let s = status::set_error(None, "Error");
/// assert_eq!(true, s.is_pending() && s.is_unhealthy())
/// ```
pub fn set_error(current_status: Option<&Status>, error: &str) -> Status {
    current_status.map_or(status().with_error(error), |s| s.with_error(error))
}

/// Updates the given status with a readiness condition or creates a new status object with the
/// readiness condition. The status will be healthy and the phase will be ready.
/// ```rust
/// use kube_operator_util::status;
///
/// let s = status::set_ready(None);
/// assert_eq!(true, s.is_ready() && s.is_healthy())
/// ```
pub fn set_ready(current_status: Option<&Status>) -> Status {
    current_status.map_or(status().with_condition(&condition()), |s| {
        s.with_condition(&condition())
    })
}

/// Create a healthy status in the "Ready" phase and with no conditions.
/// ```rust
/// use kube_operator_util::status;
///
/// let s = status::status();
/// assert_eq!(true, s.is_ready() && s.is_healthy())
/// ```
pub fn status() -> Status {
    Status {
        conditions: Vec::new(),
        health: healthy(),
        phase: READY.to_string(),
    }
}

fn timestamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).map(|o| o.to_utc()).ok()
}

/// Creates a health object with the status `Unhealthy`.
pub fn unhealthy() -> Health {
    Health {
        status: UNHEALTHY.to_string(),
    }
}

/// Creates a health object with the status `Unknown`.
pub fn unknown() -> Health {
    Health {
        status: UNKNOWN.to_string(),
    }
}

impl Condition {
    fn health(&self) -> Health {
        if self.reason == ERROR {
            Health {
                status: UNHEALTHY.to_string(),
            }
        } else if self.ready() {
            Health {
                status: HEALTHY.to_string(),
            }
        } else {
            Health {
                status: UNKNOWN.to_string(),
            }
        }
    }

    fn ready(&self) -> bool {
        self.condition_type == READY && self.status == TRUE
    }

    /// Creates a new condition with the given message.
    pub fn with_message(&self, message: &str) -> Condition {
        Condition {
            condition_type: self.condition_type.clone(),
            last_transition_time: self.last_transition_time.clone(),
            message: message.to_string(),
            reason: self.reason.clone(),
            status: self.status.clone(),
        }
    }

    /// Creates a new condition with the given reason.
    pub fn with_reason(&self, reason: &str) -> Condition {
        Condition {
            condition_type: self.condition_type.clone(),
            last_transition_time: self.last_transition_time.clone(),
            message: self.message.clone(),
            reason: reason.to_string(),
            status: self.status.clone(),
        }
    }

    /// Creates a new condition with the given status.
    pub fn with_status(&self, status: &str) -> Condition {
        Condition {
            condition_type: self.condition_type.clone(),
            last_transition_time: self.last_transition_time.clone(),
            message: self.message.clone(),
            reason: self.status.clone(),
            status: status.to_string(),
        }
    }

    /// Creates a new condition with the given type.
    pub fn with_type(&self, condition_type: &str) -> Condition {
        Condition {
            condition_type: String::from(condition_type),
            last_transition_time: self.last_transition_time.clone(),
            message: self.message.clone(),
            reason: self.reason.clone(),
            status: self.status.clone(),
        }
    }
}

impl Status {
    fn health(&self) -> Health {
        self.conditions.last().map_or_else(
            || {
                if self.phase == READY {
                    healthy()
                } else {
                    unknown()
                }
            },
            |c| c.health(),
        )
    }

    /// Indicates whether the status is healthy.
    pub fn is_healthy(&self) -> bool {
        self.health.status == HEALTHY
    }

    /// Indicates whether the status is pending.
    pub fn is_pending(&self) -> bool {
        self.phase == PENDING
    }

    /// Indicates whether the status is ready.
    pub fn is_ready(&self) -> bool {
        self.phase == READY
    }

    /// Indicates whether the status is unhealthy.
    pub fn is_unhealthy(&self) -> bool {
        self.health.status == UNHEALTHY
    }

    /// Returns the timestamp of the last condition if it is ready.
    pub fn last_success(&self) -> Option<DateTime<Utc>> {
        self.conditions
            .last()
            .filter(|c| c.ready())
            .and_then(|c| timestamp(&c.last_transition_time))
    }

    /// Creates a new status object with the given condition as the last condition. If the
    /// condition is ready, the phase will become ready as well and the status becomes healthy.
    /// ```rust
    /// use kube_operator_util::status;
    /// use kube_operator_util::status::condition;
    ///
    /// let s = status::status().with_condition(&condition());
    /// assert_eq!(true, s.is_ready() && s.is_healthy())
    /// ```
    pub fn with_condition(&self, condition: &Condition) -> Status {
        Status {
            conditions: add_condition(&self.conditions, condition),
            health: condition.health(),
            phase: if condition.ready() {
                READY.to_string()
            } else {
                PENDING.to_string()
            },
        }
    }

    /// Creates a new status object with an error condition having the given message. The phase
    /// will go to pending and the status to unhealthy.
    /// ```rust
    /// use kube_operator_util::status;
    ///
    /// let s = status::status().with_error("Error");
    /// assert_eq!(true, s.is_pending() && s.is_unhealthy())
    /// ```
    pub fn with_error(&self, message: &str) -> Status {
        self.with_condition(&error_condition(message))
    }

    /// Creates a new status object with the given phase, which can have other values than
    /// `Ready` and `Pending`.
    pub fn with_phase(&self, phase: &str) -> Status {
        Status {
            conditions: self.conditions.clone(),
            health: self.health(),
            phase: phase.to_string(),
        }
    }
}
