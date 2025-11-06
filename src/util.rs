//! Utility functions.
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use std::env;

const WATCH_NAMESPACES: &str = "WATCH_NAMESPACES";

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
