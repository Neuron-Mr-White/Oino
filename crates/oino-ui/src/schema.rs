//! Exports the normative JSON Schema of the protocol (Appendix A) plus the
//! registry envelope schema (§6.1, §4.7, §9.2). The files are checked in under
//! `schema/`; `tests/schema_parity.rs` asserts that `node.schema.json` stays
//! identical to the Appendix A block in the spec document, and that the fixture
//! corpus validates against it — the Rust types are thereby locked to the spec.

use serde_json::Value as Json;

/// The node schema (ui-protocol v2 Appendix A), verbatim.
pub const NODE_SCHEMA: &str = include_str!("../schema/node.schema.json");

/// The registry envelope schema (`UiEntry`, `UiSnapshot`, `UiPatch`,
/// `RegistryOp`, `NodeOp`, §6.1 / §4.7 / §9.2).
pub const REGISTRY_SCHEMA: &str = include_str!("../schema/registry.schema.json");

/// The node schema, parsed.
///
/// # Errors
/// When the checked-in file is not valid JSON (a build-time test catches this).
pub fn node_schema() -> Result<Json, String> {
    serde_json::from_str(NODE_SCHEMA).map_err(|e| e.to_string())
}

/// The registry envelope schema, parsed.
///
/// # Errors
/// When the checked-in file is not valid JSON (a build-time test catches this).
pub fn registry_schema() -> Result<Json, String> {
    serde_json::from_str(REGISTRY_SCHEMA).map_err(|e| e.to_string())
}

/// Protocol version the exported schemas describe.
pub const PROTOCOL_VERSION: u8 = 2;
