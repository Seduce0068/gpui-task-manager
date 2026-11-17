//! `tm-core` — pure domain model for the GPUI task manager.
//!
//! The crate is deliberately dependency-light and platform-free:
//!
//! - no I/O, no clock, no threads, no OS APIs,
//! - no `unsafe` (forbidden at the crate root),
//! - compiles for native targets and for `wasm32-unknown-unknown`,
//! - the only optional dependency is `serde`, used by the wire protocol.
//!
//! Layering:
//!
//! ```text
//! tm-core      domain types, metrics, query, ports   (this crate)
//!   ^
//! tm-viewmodel presentation state machine             (UI-independent)
//!   ^
//! app / web    thin GPUI shells (native / wasm)
//!   ^
//! tm-platform  native `SystemProbe` implementation (sysinfo + rustix)
//! ```
//!
//! Keeping the platform crate *outside* the dependency path of the shells is
//! what lets the same UI code run in the browser with a demo or agent-backed
//! probe instead of `sysinfo`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod demo;
pub mod format;
pub mod metrics;
pub mod model;
pub mod port;
pub mod query;
pub mod sanitize;

pub use metrics::{CpuCounters, MetricHistory, NetworkCounters, RingBuffer};
pub use model::{
    ComponentSnapshot, CpuSnapshot, DiskUsageSnapshot, MemorySnapshot, NetworkInterfaceSnapshot,
    NetworkSnapshot, ProcessKey, ProcessSnapshot, ProcessStatus, SystemSnapshot, Timestamp,
};
pub use port::{ControlError, PlatformInfo, ProbeError, Signal, SystemProbe};
pub use query::{
    filter_and_sort, flatten_process_tree, normalise_percent, Filter, ProcessTreeRow,
    SortDirection, SortKey, SortSpec,
};

/// Version of the JSON wire schema used between the native agent and the
/// browser shell. Bump it whenever a field is removed or its meaning changes.
pub const WIRE_SCHEMA_VERSION: u32 = 1;
