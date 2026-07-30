//! Utility functions.

use crate::status::{GetStatus, is_not_ready};
use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ManagedFieldsEntry, ObjectMeta};
use k8s_openapi::jiff::Timestamp;
use kube::Resource;
use kube::runtime::events::{Event, EventType};
use kube::runtime::{Config, Controller, watcher};
use kube::{Api, Client};
use kube_core::params::{PatchParams, PostParams};
use log::{error, info};
use serde::de::DeserializeOwned;
use std::cmp::Ordering;
use std::cmp::Ordering::{Equal, Greater, Less};
use std::env;
use std::error::Error;
use std::fmt::Debug;
use std::hash::Hash;
use std::time::Duration;

pub const DEFAULT_BACK_OFF: Duration = Duration::from_secs(5);
pub const DEFAULT_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(60);
const WATCH_NAMESPACES: &str = "WATCH_NAMESPACES";

pub trait GetObjectMeta {
    fn object_meta(&self) -> &ObjectMeta;
}

pub fn error_event(error: &str, action: &str) -> Event {
    let mut note = error.to_string();

    note.truncate(1024);

    Event {
        type_: EventType::Warning,
        reason: "Error".to_string(),
        note: Some(error.to_string()),
        action: action.to_string(),
        secondary: None,
    }
}

fn give_precedence_to_own(
    e1: &ManagedFieldsEntry,
    e2: &ManagedFieldsEntry,
    field_manager: &str,
) -> Ordering {
    if is_own(e1, field_manager) && !is_own(e2, field_manager) {
        Greater
    } else if !is_own(e1, field_manager) && is_own(e2, field_manager) {
        Less
    } else {
        Equal
    }
}

/// Indicates whether the last update performed by the given field manager occurred at
/// least the given duration ago.
pub fn has_done_update_since_at_least(
    metadata: &ObjectMeta,
    field_manager: &str,
    duration: Duration,
) -> bool {
    last_own_update(metadata, field_manager)
        .and_then(|u| update_age(&u))
        .is_some_and(|age| age > duration)
}

fn is_own(entry: &ManagedFieldsEntry, field_manager: &str) -> bool {
    entry.manager.as_ref().is_some_and(|m| m == field_manager)
}

/// Indicates whether the last update was performed by the given field manager.
pub fn is_own_update(metadata: &ObjectMeta, field_manager: &str) -> bool {
    last_update_if_owned(metadata, field_manager).is_some()
}

/// Returns the last update the given field manager ever did.
pub fn last_own_update(metadata: &ObjectMeta, field_manager: &str) -> Option<ManagedFieldsEntry> {
    last_update_with_condition(metadata, |e| is_own(e, field_manager))
}

/// Returns the last update to the resource.
pub fn last_update(metadata: &ObjectMeta) -> Option<ManagedFieldsEntry> {
    last_update_with_condition(metadata, |_| true)
}

/// Returns the last update to the resource if it was performed by the given field manager.
pub fn last_update_if_owned(
    metadata: &ObjectMeta,
    field_manager: &str,
) -> Option<ManagedFieldsEntry> {
    metadata
        .managed_fields
        .as_ref()
        .and_then(|v| {
            v.iter().cloned().max_by(|e1, e2| {
                e1.time
                    .cmp(&e2.time)
                    .then(give_precedence_to_own(e1, e2, field_manager))
            })
        })
        .filter(|m| {
            m.manager
                .as_ref()
                .map(|f| f == field_manager)
                .unwrap_or(false)
        })
}

/// Returns the last update to the resource that meets the condition.
pub fn last_update_with_condition<F>(
    metadata: &ObjectMeta,
    condition: F,
) -> Option<ManagedFieldsEntry>
where
    F: FnMut(&ManagedFieldsEntry) -> bool,
{
    let mut cond = condition;

    metadata
        .managed_fields
        .as_ref()
        .and_then(|v| {
            v.iter()
                .filter(|c| cond(c))
                .max_by(|e1, e2| e1.time.cmp(&e2.time))
        })
        .cloned()
}

/// Produces a log entry about either success of failure.
pub fn report_reconciliation<T, E>(
    result: Result<T, kube::runtime::controller::Error<E, kube::runtime::watcher::Error>>,
) where
    E: Error,
    T: Debug,
{
    match result {
        Ok(o) => info!("Reconciled {0:?}", o),
        Err(e) => error!("Reconciliation failed: {}", source_message(&e)),
    }
}

pub fn serial_controller<T>(resources: &Api<T>) -> Controller<T>
where
    T: Clone + Resource + DeserializeOwned + Debug + Send + Sync + 'static,
    T::DynamicType: Eq + Hash + Clone + Default,
{
    Controller::new(resources.clone(), watcher::Config::default())
        .with_config(Config::default().concurrency(1))
        .shutdown_on_signal()
}

/// Says a reconciliation is needed if the resource is not ready or was updated by someone else.
pub fn should_reconcile<T>(obj: &T, field_manager: &str) -> bool
where
    T: GetObjectMeta + GetStatus,
{
    is_not_ready(obj.status()) || !is_own_update(obj.object_meta(), field_manager)
}

pub fn simple_patch_params(field_manager: &str) -> PatchParams {
    PatchParams {
        dry_run: false,
        force: false,
        field_manager: Some(field_manager.to_string()),
        field_validation: None,
    }
}

pub fn simple_post_params(field_manager: &str) -> PostParams {
    PostParams {
        dry_run: false,
        field_manager: Some(field_manager.to_string()),
    }
}

fn source_message<E>(error: &E) -> String
where
    E: Error,
{
    error
        .source()
        .map_or_else(|| error.to_string(), |s| s.to_string())
}

pub fn update_age(entry: &ManagedFieldsEntry) -> Option<Duration> {
    entry.time.as_ref().map(|t| t.0).map(|t| {
        Duration::from_millis((Timestamp::now().as_millisecond() - t.as_millisecond()) as u64)
    })
}

pub fn watch_namespaced<T>(client: Client) -> Vec<Api<T>>
where
    T: Resource<Scope = NamespaceResourceScope>,
    T::DynamicType: Default,
{
    let namespaces = watch_namespaces();

    if namespaces.is_empty() || (namespaces.len() == 1 && namespaces[0] == "*") {
        info!("Watching at cluster scope");
        Vec::from([Api::<T>::all(client)])
    } else {
        namespaces
            .iter()
            .map(|n| Api::<T>::namespaced(client.clone(), n))
            .collect()
    }
}

/// Picks up the namespaces to watch from the `WATCH_NAMESPACES` environment variable. Its value
/// is comma-separated. The value `*` indicates all namespaces.
pub fn watch_namespaces() -> Vec<String> {
    match env::var_os(WATCH_NAMESPACES) {
        Some(v) => v
            .to_str()
            .map_or_else(Vec::new, |s| s.split(",").map(str::to_string).collect()),
        None => Vec::new(),
    }
}
