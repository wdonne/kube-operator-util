//! Utility functions.
use crate::status::{GetStatus, is_not_ready};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Api;
use kube::Resource;
use kube::runtime::events::{Event, EventType};
use kube::runtime::{Config, Controller, watcher};
use kube_core::params::PatchParams;
use log::{error, info};
use serde::de::DeserializeOwned;
use std::env;
use std::error::Error;
use std::fmt::Debug;
use std::hash::Hash;

const WATCH_NAMESPACES: &str = "WATCH_NAMESPACES";

pub trait GetObjectMeta {
    fn object_meta(&self) -> &ObjectMeta;
}

pub fn error_event(error: &str, action: &str) -> Event
{
    Event {
        type_: EventType::Warning,
        reason: "Error".to_string(),
        note: Some(error.to_string()),
        action: action.to_string(),
        secondary: None,
    }
}

/// Indicates whether the last update was performed by the given field manager.
pub fn is_own_update(metadata: &ObjectMeta, field_manager: &str) -> bool {
    let mut entries = metadata
        .managed_fields
        .as_ref()
        .map_or_else(Vec::new, |e| e.clone());

    entries.sort_by_key(|e| e.time.clone());

    entries
        .last()
        .and_then(|e| e.manager.clone())
        .map(|m| m == field_manager)
        .unwrap_or(false)
}

pub fn report_reconciliation<T, E>(
    resource: Result<T, kube::runtime::controller::Error<E, kube::runtime::watcher::Error>>,
) where
    E: Error,
    T: Debug,
{
    match resource {
        Ok(o) => info!("Reconciled {o:?}"),
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

fn source_message<E>(error: &E) -> String
where
    E: Error,
{
    error
        .source()
        .map_or_else(|| error.to_string(), |s| s.to_string())
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
