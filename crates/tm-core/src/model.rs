//! Domain value objects shared by every shell.
//!
//! This module has no I/O, no clock and no platform dependency, so it compiles
//! for native targets and for `wasm32-unknown-unknown` alike. Every field is
//! plain data: the platform adapter produces it, the view-model consumes it.

use std::fmt;

use crate::sanitize::sanitize_display;

/// Stable identity of a process.
///
/// A PID alone is not enough: the kernel reuses PIDs. Pairing the PID with the
/// process start time (seconds since the Unix epoch) makes the identity stable
/// and lets the UI refuse to signal a process that was replaced by another one
/// while the confirmation dialog was open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProcessKey {
    /// Kernel process id.
    pub pid: u32,
    /// Process start time, seconds since the Unix epoch.
    pub start_time_secs: u64,
}

impl ProcessKey {
    /// Creates a process identity from a PID and its start time.
    pub const fn new(pid: u32, start_time_secs: u64) -> Self {
        Self {
            pid,
            start_time_secs,
        }
    }

    /// Returns the numeric PID.
    pub const fn pid(self) -> u32 {
        self.pid
    }
}

impl fmt::Display for ProcessKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.pid, self.start_time_secs)
    }
}

/// Wall-clock timestamp used for sampling and for the wire protocol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Timestamp {
    /// Milliseconds since the Unix epoch.
    pub unix_millis: u64,
}

impl Timestamp {
    /// Builds a timestamp from whole seconds.
    pub const fn from_secs(secs: u64) -> Self {
        Self {
            unix_millis: secs.saturating_mul(1_000),
        }
    }

    /// Builds a timestamp from milliseconds.
    pub const fn from_millis(millis: u64) -> Self {
        Self {
            unix_millis: millis,
        }
    }

    /// Whole seconds since the Unix epoch.
    pub const fn as_secs(self) -> u64 {
        self.unix_millis / 1_000
    }

    /// Difference between the two timestamps' whole-second fields (saturating).
    ///
    /// This truncates each value to seconds *before* subtracting, so it can
    /// differ by one second from `(self.unix_millis - earlier.unix_millis) /
    /// 1000` when the interval crosses a second boundary (e.g. 1900 ms →
    /// 2100 ms yields 1 here, but 0 whole elapsed seconds). Callers needing
    /// elapsed milliseconds should use [`Timestamp::unix_millis`] directly.
    pub fn elapsed_secs_since(self, earlier: Timestamp) -> u64 {
        self.as_secs().saturating_sub(earlier.as_secs())
    }
}

/// Scheduler state reported by the operating system.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ProcessStatus {
    /// Runnable or currently executing.
    Running,
    /// Sleeping in an interruptible wait.
    Sleeping,
    /// Idle kernel thread.
    Idle,
    /// Stopped by a signal.
    Stopped,
    /// Zombie waiting to be reaped.
    Zombie,
    /// Being traced by a debugger.
    Tracing,
    /// Unknown or platform-specific state.
    ///
    /// Unknown wire values from a newer agent degrade to this variant instead
    /// of rejecting the whole snapshot (R42 wire audit).
    #[default]
    #[cfg_attr(feature = "serde", serde(other))]
    Unknown,
}

impl ProcessStatus {
    /// Short Japanese label used by the UI.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Running => "実行中",
            Self::Sleeping => "待機",
            Self::Idle => "アイドル",
            Self::Stopped => "停止",
            Self::Zombie => "ゾンビ",
            Self::Tracing => "トレース",
            Self::Unknown => "不明",
        }
    }

    /// English identifier used in the wire protocol.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Sleeping => "sleeping",
            Self::Idle => "idle",
            Self::Stopped => "stopped",
            Self::Zombie => "zombie",
            Self::Tracing => "tracing",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// CPU utilisation for the whole machine and for each logical core.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CpuSnapshot {
    /// Machine-wide utilisation in percent (`0.0..=100.0`).
    pub total_percent: f32,
    /// Per-logical-core utilisation in percent.
    pub per_core_percent: Vec<f32>,
    /// 1/5/15 minute load averages (Linux semantics; zeros elsewhere).
    pub load_average: [f32; 3],
    /// Seconds since boot.
    pub uptime_secs: u64,
    /// Current average core frequency in MHz, when the platform reports it.
    pub frequency_mhz: Option<u64>,
}

impl CpuSnapshot {
    /// Number of logical cores reported by the platform.
    pub fn core_count(&self) -> usize {
        self.per_core_percent.len()
    }
}

/// Physical and swap memory usage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MemorySnapshot {
    /// Installed RAM in bytes.
    pub total_bytes: u64,
    /// Used RAM in bytes.
    pub used_bytes: u64,
    /// RAM available for new allocations in bytes.
    pub available_bytes: u64,
    /// Swap capacity in bytes.
    pub swap_total_bytes: u64,
    /// Used swap in bytes.
    pub swap_used_bytes: u64,
}

impl MemorySnapshot {
    /// RAM utilisation in percent.
    pub fn used_percent(&self) -> f32 {
        percent_of(self.used_bytes, self.total_bytes)
    }

    /// Swap utilisation in percent.
    pub fn swap_used_percent(&self) -> f32 {
        percent_of(self.swap_used_bytes, self.swap_total_bytes)
    }
}

/// Normalises a temperature reading for wire/export output.
///
/// Unlike CPU percentages, temperatures are not bounded by 100: a hot sensor
/// may legitimately report 120 °C and a cold one -20 °C. Only non-finite
/// values (which JSON cannot represent) collapse to zero (R32 audit).
pub fn normalise_temperature(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn percent_of(part: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }
    ((part as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
}

/// One process as sampled at a point in time.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProcessSnapshot {
    /// Stable identity (PID + start time).
    pub key: ProcessKey,
    /// Executable name (usually `comm`, at most 15 characters on Linux).
    pub name: String,
    /// Full command line, already sanitised for display by the adapter.
    pub command: String,
    /// Owning user name, when it can be resolved.
    pub user: Option<String>,
    /// Scheduler state.
    pub status: ProcessStatus,
    /// Process CPU usage in percent; may exceed 100 on multicore systems.
    pub cpu_percent: f32,
    /// Resident memory in bytes.
    pub memory_bytes: u64,
    /// Virtual memory in bytes.
    pub virtual_memory_bytes: u64,
    /// Thread count, when the platform reports it.
    pub threads: Option<u32>,
    /// Parent PID, when the platform reports it.
    pub parent_pid: Option<u32>,
}

/// Cumulative byte counters for one network interface.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NetworkInterfaceSnapshot {
    /// Interface name such as `eth0` or `wlan0`.
    pub name: String,
    /// Bytes received since boot.
    pub received_bytes: u64,
    /// Bytes transmitted since boot.
    pub transmitted_bytes: u64,
}

/// Network counters for every interface known to the platform.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NetworkSnapshot {
    /// Interfaces in platform order.
    pub interfaces: Vec<NetworkInterfaceSnapshot>,
}

impl NetworkSnapshot {
    /// Sums the cumulative counters of all interfaces.
    pub fn totals(&self) -> (u64, u64) {
        self.interfaces.iter().fold((0, 0), |(rx, tx), iface| {
            (
                rx.saturating_add(iface.received_bytes),
                tx.saturating_add(iface.transmitted_bytes),
            )
        })
    }

    /// Sums the counters of the interfaces that carry real network traffic.
    ///
    /// Loopback and virtual interfaces are excluded: their traffic is already
    /// counted on the physical interface underneath, so adding them would
    /// double-count the overview rate (Runtime Proof: a VPN interface reported
    /// tens of MB received while the physical link reported hundreds of GB,
    /// i.e. the VPN traffic was counted twice).
    pub fn physical_totals(&self) -> (u64, u64) {
        self.interfaces
            .iter()
            .filter(|iface| !is_virtual_interface(&iface.name))
            .fold((0, 0), |(rx, tx), iface| {
                (
                    rx.saturating_add(iface.received_bytes),
                    tx.saturating_add(iface.transmitted_bytes),
                )
            })
    }
}

/// Returns `true` for loopback, bridge, container and VPN interfaces whose
/// counters duplicate traffic already seen on the physical interface.
///
/// The list is name-based because it has to work identically across the native
/// and browser probes, and deliberately errs towards treating an unknown
/// interface as physical (a missing interface in the summary is worse than a
/// rarely over-counted one).
pub fn is_virtual_interface(name: &str) -> bool {
    const VIRTUAL_PREFIXES: [&str; 11] = [
        "br-",
        "docker",
        "veth",
        "virbr",
        "lxcbr",
        "tailscale",
        "vpn",  // generic VPN interfaces (e.g. vpn0)
        "wg",   // WireGuard (same tunnel-double-count as a VPN interface)
        "utun", // macOS IKEv2 / WireGuard tunnels
        "tun",
        "tap",
    ];
    // Linux "lo", macOS "lo0". Do not use starts_with("lo"): that would
    // misclassify physical names such as "local0".
    if name == "lo" || name == "lo0" {
        return true;
    }
    VIRTUAL_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Disk usage for one mounted filesystem.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DiskUsageSnapshot {
    /// Mount point, for example `/` or `/home`.
    pub mount_point: String,
    /// Filesystem capacity in bytes.
    pub total_bytes: u64,
    /// Free bytes available to unprivileged users.
    pub available_bytes: u64,
    /// Used bytes.
    pub used_bytes: u64,
}

impl DiskUsageSnapshot {
    /// Filesystem utilisation in percent.
    pub fn used_percent(&self) -> f32 {
        percent_of(self.used_bytes, self.total_bytes)
    }
}

/// Temperature reading from a hardware component (sensor), when available.
///
/// Platforms that cannot enumerate components report an empty list. Values are
/// best-effort and may be missing on some drivers/virtual machines.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ComponentSnapshot {
    /// Human-readable sensor/component label.
    pub label: String,
    /// Current temperature in degrees Celsius, when reported.
    pub temperature_c: Option<f32>,
    /// Maximum observed temperature in degrees Celsius, when reported.
    pub max_c: Option<f32>,
    /// Critical/halt threshold in degrees Celsius, when reported.
    pub critical_c: Option<f32>,
}

/// A complete point-in-time view of the machine.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SystemSnapshot {
    /// When the sample was taken (wall clock, for display only).
    pub sampled_at: Timestamp,
    /// Monotonic time since the previous sample. Used for rate calculation:
    /// wall-clock deltas can go backwards under NTP/VM resume (R49 audit).
    #[cfg_attr(feature = "serde", serde(default))]
    pub sampled_elapsed: std::time::Duration,
    /// Host name, when available.
    pub hostname: Option<String>,
    /// Operating system description.
    pub os_name: Option<String>,
    /// Kernel version, when available.
    pub kernel_version: Option<String>,
    /// CPU metrics.
    pub cpu: CpuSnapshot,
    /// Memory metrics.
    pub memory: MemorySnapshot,
    /// Process list in platform order.
    pub processes: Vec<ProcessSnapshot>,
    /// Network counters.
    pub network: NetworkSnapshot,
    /// Mounted filesystems.
    pub disks: Vec<DiskUsageSnapshot>,
    /// Hardware component temperatures, when the platform reports them.
    #[cfg_attr(feature = "serde", serde(default))]
    pub components: Vec<ComponentSnapshot>,
    /// Aggregate power source and enumerated batteries (R1-I10).
    ///
    /// `serde(default)` keeps the wire compatible with agents that predate this
    /// field: an older snapshot deserialises to an honest `Unknown` source with
    /// no batteries, never to a fabricated AC/battery claim.
    #[cfg_attr(feature = "serde", serde(default))]
    pub power: PowerSnapshot,
    /// Cadence recommendation selected for this sample (R1-I11).
    #[cfg_attr(feature = "serde", serde(default))]
    pub sampling: SamplingPlan,
    /// Processes grouped by normalised cgroup label (R1-I12).
    #[cfg_attr(feature = "serde", serde(default))]
    pub cgroups: Vec<CgroupGroup>,
    /// PID of the application itself, used to protect it from being killed.
    pub self_pid: Option<u32>,
}

impl SystemSnapshot {
    /// Looks a process up by its stable identity.
    pub fn process(&self, key: ProcessKey) -> Option<&ProcessSnapshot> {
        self.processes.iter().find(|process| process.key == key)
    }

    /// Number of processes in the snapshot.
    pub fn process_count(&self) -> usize {
        self.processes.len()
    }

    /// Cumulative network totals for the whole machine.
    ///
    /// Excludes loopback and virtual interfaces so the overview rate is not
    /// double-counted (see [`NetworkSnapshot::physical_totals`]).
    pub fn network_totals(&self) -> (u64, u64) {
        self.network.physical_totals()
    }
}

// --- R1-I10 / R1-I11 / R1-I12 -------------------------------------------------
//
// The three data families below are additive: `SystemSnapshot` gained `power`,
// `sampling` and `cgroups` fields with `serde(default)`, so older wire snapshots
// still deserialise. Every producer (native probe, demo probe), the redact /
// sanitize passes and the JSON exporter register the new fields, and both shells
// display them without changing any pre-existing wire, DOM or redact contract.

/// Aggregate power source reported by the platform (R1-I10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PowerSource {
    /// The machine is running on external power.
    Ac,
    /// The machine is running on a battery.
    Battery,
    /// The platform exposes no power-supply information.
    ///
    /// This is the honest default for desktops without a battery and for every
    /// non-Linux target of the pinned `sysinfo` version, which has no battery
    /// API. It must never be rendered as "charging".
    ///
    /// Unknown wire values from a newer agent degrade to this variant instead
    /// of rejecting the whole snapshot (R42 wire audit, R2-DATA). serde requires
    /// `serde(other)` on the last variant; unit enums are serialised by name, so
    /// declaring the default last leaves the wire representation unchanged.
    #[default]
    #[cfg_attr(feature = "serde", serde(other))]
    Unknown,
}

impl PowerSource {
    /// Short Japanese label used by a status widget.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "不明",
            Self::Ac => "AC 電源",
            Self::Battery => "バッテリー",
        }
    }

    /// Stable English identifier used in the wire protocol.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Ac => "ac",
            Self::Battery => "battery",
        }
    }
}

/// Charge/discharge state of one battery (R1-I10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BatteryState {
    /// The battery is charging.
    Charging,
    /// The battery is discharging.
    Discharging,
    /// The battery is fully charged.
    Full,
    /// The battery is present but is not charging (for example at a threshold).
    NotCharging,
    /// The platform did not report a usable state.
    ///
    /// Unknown wire values degrade here rather than rejecting the snapshot
    /// (R42 wire audit, R2-DATA); serde requires `serde(other)` on the last
    /// variant. Serialisation is by variant name, so the wire format is stable.
    #[default]
    #[cfg_attr(feature = "serde", serde(other))]
    Unknown,
}

impl BatteryState {
    /// Short Japanese label used by a status widget.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "不明",
            Self::Charging => "充電中",
            Self::Discharging => "放電中",
            Self::Full => "満充電",
            Self::NotCharging => "充電停止",
        }
    }

    /// Stable English identifier used in the wire protocol.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Charging => "charging",
            Self::Discharging => "discharging",
            Self::Full => "full",
            Self::NotCharging => "not_charging",
        }
    }
}

/// One battery the platform could enumerate (R1-I10).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BatterySnapshot {
    /// Sanitised, short battery name (for example `BAT0`).
    ///
    /// This value comes from the OS and must have passed through
    /// [`crate::sanitize::sanitize_display`] at the probe boundary.
    pub name: String,
    /// State of charge in percent (`0..=100`), or `None` when unreported.
    #[cfg_attr(feature = "serde", serde(default))]
    pub capacity_percent: Option<u8>,
    /// Reported charge/discharge state.
    pub state: BatteryState,
}

/// Power state of the machine at one point in time (R1-I10).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PowerSnapshot {
    /// Aggregate source derived from the enumerated supplies.
    pub source: PowerSource,
    /// Enumerated batteries, sorted by name. Empty when unsupported.
    #[cfg_attr(feature = "serde", serde(default))]
    pub batteries: Vec<BatterySnapshot>,
}

impl PowerSnapshot {
    /// Whether the machine is currently drawing from a battery.
    ///
    /// Both the aggregate source and the individual battery states are checked:
    /// a supply that reports `Discharging` must not be masked by a stale
    /// aggregate value coming from an older sample.
    pub fn on_battery(&self) -> bool {
        self.source == PowerSource::Battery
            || self
                .batteries
                .iter()
                .any(|battery| battery.state == BatteryState::Discharging)
    }

    /// Lowest reported state of charge across all batteries.
    ///
    /// `None` when no battery reported a capacity: the sampling policy must not
    /// treat missing data as an empty (and therefore low) battery.
    pub fn min_capacity_percent(&self) -> Option<u8> {
        self.batteries
            .iter()
            .filter_map(|battery| battery.capacity_percent)
            .min()
    }
}

/// Whether the application window is visible to the user (R1-I11).
///
/// The shell owns this value; it lives in the core so the sampling policy is a
/// pure, platform-independent function that can be regression-tested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WindowVisibility {
    /// The window is minimised, on another workspace or otherwise hidden.
    Hidden,
    /// The window is on screen and the user is expected to read it.
    ///
    /// Unknown wire values degrade here (R42 wire audit, R2-DATA). serde
    /// requires `serde(other)` on the last variant; the wire spelling stays the
    /// same because unit variants are serialised by name.
    #[default]
    #[cfg_attr(feature = "serde", serde(other))]
    Visible,
}

/// Which sampling policy produced a [`SamplingPlan`] (R1-I11).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SamplingProfile {
    /// Reduced cadence while running on battery.
    Battery,
    /// Slow cadence when the battery is low.
    BatterySaver,
    /// Slow cadence while the window is hidden.
    Background,
    /// Normal interactive cadence (mains power, visible window).
    ///
    /// Unknown wire values degrade here rather than rejecting the snapshot
    /// (R42 wire audit, R2-DATA). serde requires `serde(other)` on the last
    /// variant; serialisation by name keeps the wire representation stable.
    #[default]
    #[cfg_attr(feature = "serde", serde(other))]
    Interactive,
}

impl SamplingProfile {
    /// Short Japanese label used by a status widget.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Interactive => "通常",
            Self::Battery => "バッテリー節約",
            Self::BatterySaver => "低バッテリー",
            Self::Background => "バックグラウンド",
        }
    }

    /// Stable English identifier used in the wire protocol.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Battery => "battery",
            Self::BatterySaver => "battery_saver",
            Self::Background => "background",
        }
    }
}

/// Sampling interval in milliseconds used when the window is visible on mains power (R1-I11).
pub const INTERACTIVE_INTERVAL_MS: u64 = 1_000;
/// Sampling interval in milliseconds used while the window is hidden (R1-I11).
pub const BACKGROUND_INTERVAL_MS: u64 = 5_000;
/// Sampling interval in milliseconds used while running on battery (R1-I11).
pub const BATTERY_INTERVAL_MS: u64 = 2_500;
/// Sampling interval in milliseconds used when the battery is low (R1-I11).
pub const BATTERY_SAVER_INTERVAL_MS: u64 = 5_000;
/// State of charge at or below which the battery-saver profile is selected.
pub const LOW_BATTERY_PERCENT: u8 = 20;

/// The sampling interval chosen for the current power state and window visibility (R1-I11).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SamplingPlan {
    /// Recommended delay between two samples, in milliseconds.
    pub interval_ms: u64,
    /// Policy that selected `interval_ms`, for diagnostics and display.
    pub profile: SamplingProfile,
}

impl SamplingPlan {
    /// The recommended delay as a `Duration`.
    pub fn duration(self) -> std::time::Duration {
        std::time::Duration::from_millis(self.interval_ms)
    }

    /// Applies this plan as a *floor* on a user-configured refresh interval.
    ///
    /// A fast interactive plan leaves the configured cadence untouched (the
    /// default 1 s keeps its exact behaviour), while a battery / saver /
    /// background plan slows a too-fast setting down to the recommended delay.
    /// A deliberately slower user setting is always respected.
    pub fn effective_interval(self, configured: std::time::Duration) -> std::time::Duration {
        configured.max(self.duration())
    }
}

/// Chooses a sampling interval from power state and window visibility (R1-I11).
///
/// The policy is deliberately total and side-effect free, so a shell can apply
/// the same answer on every backend (native, demo or agent-backed):
///
/// 1. A hidden window always uses [`BACKGROUND_INTERVAL_MS`], whatever the power
///    state. This is the headline behaviour of R1-I11 and takes precedence.
/// 2. A visible window on battery uses [`BATTERY_INTERVAL_MS`], or
///    [`BATTERY_SAVER_INTERVAL_MS`] when the lowest reported charge is at or
///    below [`LOW_BATTERY_PERCENT`].
/// 3. Everything else (including [`PowerSource::Unknown`]) uses the normal
///    interactive cadence. Missing power data must never silently throttle the
///    visible UI.
pub fn recommended_sampling_plan(
    power: &PowerSnapshot,
    visibility: WindowVisibility,
) -> SamplingPlan {
    if visibility == WindowVisibility::Hidden {
        return SamplingPlan {
            interval_ms: BACKGROUND_INTERVAL_MS,
            profile: SamplingProfile::Background,
        };
    }
    if power.on_battery() {
        if power
            .min_capacity_percent()
            .is_some_and(|percent| percent <= LOW_BATTERY_PERCENT)
        {
            return SamplingPlan {
                interval_ms: BATTERY_SAVER_INTERVAL_MS,
                profile: SamplingProfile::BatterySaver,
            };
        }
        return SamplingPlan {
            interval_ms: BATTERY_INTERVAL_MS,
            profile: SamplingProfile::Battery,
        };
    }
    SamplingPlan {
        interval_ms: INTERACTIVE_INTERVAL_MS,
        profile: SamplingProfile::Interactive,
    }
}

/// Maximum number of characters kept from a cgroup label.
pub const CGROUP_LABEL_MAX_CHARS: usize = 96;

/// Normalises a raw cgroup path into a short, display-safe grouping label (R1-I12).
///
/// A `/proc/<pid>/cgroup` path can contain user names, container IDs and other
/// host-specific identifiers. Only the last two non-empty path segments are
/// kept (for example `app.slice/worker.service`), which is enough to group
/// related processes without publishing the whole host path. The result is
/// passed through [`crate::sanitize::sanitize_display`], so control characters,
/// ANSI sequences and bidi controls never reach a widget. A missing or root
/// path yields `"/"`.
///
/// Both a bare path (`/a.slice/b.service`) and a full `/proc` line
/// (`0::/a.slice/b.service`) are accepted, so callers cannot accidentally
/// publish the hierarchy prefix as a group label.
pub fn cgroup_label(raw_path: &str) -> String {
    let trimmed = raw_path.trim();
    let path = if trimmed.starts_with('/') {
        trimmed
    } else {
        // Drop the `hierarchy:controllers:` prefix of a raw /proc line.
        trimmed.rsplit_once(':').map_or(trimmed, |(_, path)| path)
    };
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let label = if segments.is_empty() {
        "/".to_string()
    } else {
        let start = segments.len().saturating_sub(2);
        segments[start..].join("/")
    };
    sanitize_display(&label, CGROUP_LABEL_MAX_CHARS)
}

/// Read-only cgroup membership of one sampled process (R1-I12).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProcessCgroup {
    /// Stable identity of the process (PID + start time).
    pub key: ProcessKey,
    /// Normalised, display-safe label from [`cgroup_label`].
    pub label: String,
}

/// Processes that share one cgroup label (R1-I12).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CgroupGroup {
    /// Normalised label shared by every member.
    pub label: String,
    /// Member identities, sorted by PID and start time.
    #[cfg_attr(feature = "serde", serde(default))]
    pub members: Vec<ProcessKey>,
}

/// Groups cgroup memberships deterministically for read-only display (R1-I12).
///
/// Memberships with an empty label are ignored. Groups are ordered by label and
/// their members by [`ProcessKey`], so the output is stable across samples even
/// though the underlying `/proc` enumeration order is not.
pub fn group_by_cgroup(memberships: &[ProcessCgroup]) -> Vec<CgroupGroup> {
    let mut grouped = std::collections::BTreeMap::<String, Vec<ProcessKey>>::new();
    for membership in memberships {
        if membership.label.is_empty() {
            continue;
        }
        grouped
            .entry(membership.label.clone())
            .or_default()
            .push(membership.key);
    }
    grouped
        .into_iter()
        .map(|(label, mut members)| {
            members.sort_unstable();
            members.dedup();
            CgroupGroup { label, members }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "serde")]
    fn unknown_process_status_degrades_to_unknown() {
        let status: ProcessStatus = serde_json::from_str("\"Suspended\"")
            .expect("unknown status must not reject the whole snapshot");
        assert_eq!(status, ProcessStatus::Unknown);
        let known: ProcessStatus = serde_json::from_str("\"Running\"").expect("known status");
        assert_eq!(known, ProcessStatus::Running);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn unknown_wire_enum_values_degrade_without_rejecting_the_snapshot() {
        // R42 wire audit / R2-DATA: every unit wire enum must follow the
        // ProcessStatus policy so a newer agent cannot invalidate a whole
        // snapshot by adding one variant.
        let mut snapshot = SystemSnapshot {
            power: PowerSnapshot {
                source: PowerSource::Battery,
                batteries: vec![BatterySnapshot {
                    name: "BAT0".into(),
                    capacity_percent: Some(50),
                    state: BatteryState::Discharging,
                }],
            },
            sampling: SamplingPlan {
                interval_ms: BATTERY_INTERVAL_MS,
                profile: SamplingProfile::Battery,
            },
            ..SystemSnapshot::default()
        };
        snapshot.processes.push(ProcessSnapshot {
            status: ProcessStatus::Running,
            ..ProcessSnapshot::default()
        });

        // Patch the exact wire payload rather than hand-writing the envelope:
        // this keeps the test in sync with the model's field set.
        let mut wire = serde_json::to_value(&snapshot).expect("snapshot must serialise");
        wire["power"]["source"] = serde_json::json!("FutureSource");
        wire["power"]["batteries"][0]["state"] = serde_json::json!("FutureState");
        wire["sampling"]["profile"] = serde_json::json!("FutureProfile");
        wire["processes"][0]["status"] = serde_json::json!("FutureStatus");

        let restored: SystemSnapshot = serde_json::from_value(wire)
            .expect("unknown enum values must not reject the whole snapshot");
        assert_eq!(restored.power.source, PowerSource::Unknown);
        assert_eq!(restored.power.batteries[0].state, BatteryState::Unknown);
        assert_eq!(restored.sampling.profile, SamplingProfile::Interactive);
        assert_eq!(restored.processes[0].status, ProcessStatus::Unknown);
        // Non-enum data in the same snapshot survives the degradation.
        assert_eq!(restored.power.batteries[0].capacity_percent, Some(50));
        assert_eq!(restored.sampling.interval_ms, BATTERY_INTERVAL_MS);

        // WindowVisibility is shell-owned and not a snapshot field, so exercise
        // its fallthrough directly.
        let visibility: WindowVisibility = serde_json::from_str("\"FutureVisibility\"")
            .expect("unknown visibility must not fail deserialisation");
        assert_eq!(visibility, WindowVisibility::Visible);
        // Known values still deserialise to their exact variants.
        let known: SamplingProfile = serde_json::from_str("\"Background\"").expect("known profile");
        assert_eq!(known, SamplingProfile::Background);
    }

    #[test]
    fn process_status_labels_and_wire_ids_are_stable() {
        // The label is shown in the UI and the id is written to the wire
        // protocol; both are asserted by the CDP proof hook, so pin every
        // variant here.
        let cases = [
            (ProcessStatus::Running, "実行中", "running"),
            (ProcessStatus::Sleeping, "待機", "sleeping"),
            (ProcessStatus::Idle, "アイドル", "idle"),
            (ProcessStatus::Stopped, "停止", "stopped"),
            (ProcessStatus::Zombie, "ゾンビ", "zombie"),
            (ProcessStatus::Tracing, "トレース", "tracing"),
            (ProcessStatus::Unknown, "不明", "unknown"),
        ];
        for (status, label, id) in cases {
            assert_eq!(status.label(), label);
            assert_eq!(status.id(), id);
        }
    }

    #[test]
    fn process_key_is_ordered_by_pid_then_start_time() {
        assert!(ProcessKey::new(1, 10) < ProcessKey::new(2, 0));
        assert!(ProcessKey::new(1, 10) < ProcessKey::new(1, 11));
    }

    #[test]
    fn process_key_display_includes_pid_and_start_time() {
        // The signal path logs this form when reusing a PID; both halves of the
        // identity must be visible for diagnostics.
        assert_eq!(
            ProcessKey::new(4242, 1_700_000_000).to_string(),
            "4242@1700000000"
        );
        assert_eq!(ProcessKey::new(4242, 1_700_000_001).pid(), 4242);
    }

    #[test]
    fn timestamp_math_is_saturating() {
        let now = Timestamp::from_secs(100);
        let earlier = Timestamp::from_secs(140);
        assert_eq!(now.elapsed_secs_since(earlier), 0);
        assert_eq!(Timestamp::from_millis(1_500).as_secs(), 1);
        // Whole-second fields are subtracted, not elapsed milliseconds:
        // 1900 ms -> 2100 ms is 200 ms of real time but 1 in second fields.
        assert_eq!(
            Timestamp::from_millis(2_100).elapsed_secs_since(Timestamp::from_millis(1_900)),
            1
        );
    }

    #[test]
    fn timestamp_from_extreme_secs_saturates() {
        // A corrupt snapshot could carry an out-of-range sampled_at; converting
        // it must saturate rather than wrap into a plausible past timestamp.
        let timestamp = Timestamp::from_secs(u64::MAX);
        assert_eq!(timestamp.unix_millis, u64::MAX);
        assert_eq!(timestamp.as_secs(), u64::MAX / 1_000);
    }

    #[test]
    fn memory_percent_handles_zero_total() {
        let memory = MemorySnapshot::default();
        assert_eq!(memory.used_percent(), 0.0);
        assert_eq!(memory.swap_used_percent(), 0.0);
    }

    #[test]
    fn memory_percent_is_clamped() {
        let memory = MemorySnapshot {
            total_bytes: 100,
            used_bytes: 150,
            ..MemorySnapshot::default()
        };
        assert_eq!(memory.used_percent(), 100.0);
    }

    #[test]
    fn network_totals_saturate() {
        let network = NetworkSnapshot {
            interfaces: vec![
                NetworkInterfaceSnapshot {
                    name: "a".into(),
                    received_bytes: u64::MAX,
                    transmitted_bytes: 1,
                },
                NetworkInterfaceSnapshot {
                    name: "b".into(),
                    received_bytes: 10,
                    transmitted_bytes: 2,
                },
            ],
        };
        assert_eq!(network.totals(), (u64::MAX, 3));
    }

    #[test]
    fn virtual_interfaces_are_detected_by_name() {
        for name in [
            "lo",
            "docker0",
            "br-19b60f1f19dc",
            "veth1234",
            "virbr0",
            "lxcbr0",
            "vpn0",
            "wg0",
            "utun3",
            "tun0",
            "tap0",
            "lo0", // macOS loopback
        ] {
            assert!(is_virtual_interface(name), "{name} must be virtual");
        }
        for name in ["eth0", "eth1", "wlan0", "wlp3s0", "en0", "wlan1", "local0"] {
            assert!(!is_virtual_interface(name), "{name} must be physical");
        }
    }

    #[test]
    fn physical_totals_exclude_loopback_and_virtual_interfaces() {
        // The VPN interface duplicates traffic that is also counted on eth0.
        let network = NetworkSnapshot {
            interfaces: vec![
                NetworkInterfaceSnapshot {
                    name: "eth0".into(),
                    received_bytes: 100_000_000_000,
                    transmitted_bytes: 20_000_000_000,
                },
                NetworkInterfaceSnapshot {
                    name: "vpn0".into(),
                    received_bytes: 50_000_000,
                    transmitted_bytes: 20_000_000,
                },
                NetworkInterfaceSnapshot {
                    name: "docker0".into(),
                    received_bytes: 0,
                    transmitted_bytes: 0,
                },
                NetworkInterfaceSnapshot {
                    name: "lo".into(),
                    received_bytes: 1_000,
                    transmitted_bytes: 2_000,
                },
            ],
        };
        assert_eq!(network.physical_totals(), (100_000_000_000, 20_000_000_000));
        // The all-interface total still double-counts, which is why the
        // overview uses physical_totals().
        assert_eq!(network.totals(), (100_050_001_000, 20_020_002_000));
    }

    // --- R1-I10: power state -------------------------------------------------

    #[test]
    fn power_snapshot_default_never_claims_battery_data() {
        let power = PowerSnapshot::default();
        assert_eq!(power.source, PowerSource::Unknown);
        assert!(!power.on_battery());
        assert_eq!(power.min_capacity_percent(), None);
        assert!(power.batteries.is_empty());
    }

    #[test]
    fn discharging_battery_overrides_a_stale_aggregate_source() {
        let power = PowerSnapshot {
            source: PowerSource::Ac,
            batteries: vec![BatterySnapshot {
                name: "BAT0".into(),
                capacity_percent: Some(55),
                state: BatteryState::Discharging,
            }],
        };
        assert!(
            power.on_battery(),
            "a discharging supply must win over a stale AC aggregate"
        );
        assert_eq!(power.min_capacity_percent(), Some(55));
    }

    #[test]
    fn min_capacity_ignores_batteries_without_a_reading() {
        let power = PowerSnapshot {
            source: PowerSource::Battery,
            batteries: vec![
                BatterySnapshot {
                    name: "BAT0".into(),
                    capacity_percent: None,
                    state: BatteryState::Unknown,
                },
                BatterySnapshot {
                    name: "BAT1".into(),
                    capacity_percent: Some(42),
                    state: BatteryState::Charging,
                },
            ],
        };
        assert_eq!(power.min_capacity_percent(), Some(42));
    }

    // --- R1-I11: adaptive sampling -------------------------------------------

    #[test]
    fn hidden_window_always_uses_the_background_profile() {
        let hidden_low_battery = PowerSnapshot {
            source: PowerSource::Battery,
            batteries: vec![BatterySnapshot {
                name: "BAT0".into(),
                capacity_percent: Some(5),
                state: BatteryState::Discharging,
            }],
        };
        let plan = recommended_sampling_plan(&hidden_low_battery, WindowVisibility::Hidden);
        assert_eq!(
            plan,
            SamplingPlan {
                interval_ms: BACKGROUND_INTERVAL_MS,
                profile: SamplingProfile::Background,
            }
        );
        assert_eq!(
            plan.duration().as_millis(),
            u128::from(BACKGROUND_INTERVAL_MS)
        );
    }

    #[test]
    fn visible_window_on_battery_uses_the_battery_profile() {
        let power = PowerSnapshot {
            source: PowerSource::Battery,
            batteries: vec![BatterySnapshot {
                name: "BAT0".into(),
                capacity_percent: Some(50),
                state: BatteryState::Discharging,
            }],
        };
        assert_eq!(
            recommended_sampling_plan(&power, WindowVisibility::Visible),
            SamplingPlan {
                interval_ms: BATTERY_INTERVAL_MS,
                profile: SamplingProfile::Battery,
            }
        );
    }

    #[test]
    fn low_battery_switches_to_the_saver_profile_at_the_threshold() {
        for (percent, expected) in [
            (LOW_BATTERY_PERCENT, SamplingProfile::BatterySaver),
            (LOW_BATTERY_PERCENT - 1, SamplingProfile::BatterySaver),
            (LOW_BATTERY_PERCENT + 1, SamplingProfile::Battery),
        ] {
            let power = PowerSnapshot {
                source: PowerSource::Battery,
                batteries: vec![BatterySnapshot {
                    name: "BAT0".into(),
                    capacity_percent: Some(percent),
                    state: BatteryState::Discharging,
                }],
            };
            let plan = recommended_sampling_plan(&power, WindowVisibility::Visible);
            assert_eq!(plan.profile, expected, "percent {percent}");
            assert_eq!(
                plan.interval_ms,
                if expected == SamplingProfile::BatterySaver {
                    BATTERY_SAVER_INTERVAL_MS
                } else {
                    BATTERY_INTERVAL_MS
                }
            );
        }
    }

    #[test]
    fn missing_power_data_never_throttles_a_visible_window() {
        for source in [PowerSource::Unknown, PowerSource::Ac] {
            let power = PowerSnapshot {
                source,
                batteries: Vec::new(),
            };
            assert_eq!(
                recommended_sampling_plan(&power, WindowVisibility::Visible),
                SamplingPlan {
                    interval_ms: INTERACTIVE_INTERVAL_MS,
                    profile: SamplingProfile::Interactive,
                }
            );
        }
        // A battery with no capacity reading must not look like an empty one.
        let unknown_capacity = PowerSnapshot {
            source: PowerSource::Battery,
            batteries: vec![BatterySnapshot {
                name: "BAT0".into(),
                capacity_percent: None,
                state: BatteryState::Discharging,
            }],
        };
        assert_eq!(
            recommended_sampling_plan(&unknown_capacity, WindowVisibility::Visible).profile,
            SamplingProfile::Battery
        );
    }

    #[test]
    fn power_and_sampling_labels_are_stable() {
        for (value, label, id) in [
            (PowerSource::Unknown, "不明", "unknown"),
            (PowerSource::Ac, "AC 電源", "ac"),
            (PowerSource::Battery, "バッテリー", "battery"),
        ] {
            assert_eq!(value.label(), label);
            assert_eq!(value.id(), id);
        }
        for (value, label, id) in [
            (BatteryState::Unknown, "不明", "unknown"),
            (BatteryState::Charging, "充電中", "charging"),
            (BatteryState::Discharging, "放電中", "discharging"),
            (BatteryState::Full, "満充電", "full"),
            (BatteryState::NotCharging, "充電停止", "not_charging"),
        ] {
            assert_eq!(value.label(), label);
            assert_eq!(value.id(), id);
        }
        for (value, label, id) in [
            (SamplingProfile::Interactive, "通常", "interactive"),
            (SamplingProfile::Battery, "バッテリー節約", "battery"),
            (
                SamplingProfile::BatterySaver,
                "低バッテリー",
                "battery_saver",
            ),
            (
                SamplingProfile::Background,
                "バックグラウンド",
                "background",
            ),
        ] {
            assert_eq!(value.label(), label);
            assert_eq!(value.id(), id);
        }
    }

    // --- R1-I12: cgroup grouping ---------------------------------------------

    #[test]
    fn cgroup_label_drops_the_host_path_and_keeps_the_leaf_units() {
        assert_eq!(
            cgroup_label(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/worker.service"
            ),
            "app.slice/worker.service"
        );
        assert_eq!(
            cgroup_label("1:name=systemd:/system.slice/docker-abc123.scope"),
            "system.slice/docker-abc123.scope"
        );
        assert_eq!(cgroup_label("0::/"), "/");
        assert_eq!(cgroup_label("   "), "/");
    }

    #[test]
    fn cgroup_label_is_display_safe_and_bounded() {
        let hostile = format!("0::/x.slice/{}\u{001b}[31mw.service", "A".repeat(500));
        let label = cgroup_label(&hostile);
        assert!(
            !label.chars().any(char::is_control),
            "control character leaked into a cgroup label: {label:?}"
        );
        assert!(!label.contains("[31m"), "ANSI payload leaked: {label:?}");
        assert!(
            label.chars().count() <= CGROUP_LABEL_MAX_CHARS + 1, // +1 for '…'
            "label was not truncated: {} chars",
            label.chars().count()
        );
    }

    #[test]
    fn cgroup_groups_are_sorted_and_members_deduplicated() {
        let memberships = vec![
            ProcessCgroup {
                key: ProcessKey::new(200, 20),
                label: "b.slice/app.service".into(),
            },
            ProcessCgroup {
                key: ProcessKey::new(100, 10),
                label: "a.slice/app.service".into(),
            },
            ProcessCgroup {
                key: ProcessKey::new(100, 10),
                label: "a.slice/app.service".into(),
            },
            ProcessCgroup {
                key: ProcessKey::new(300, 30),
                label: "b.slice/app.service".into(),
            },
        ];
        let groups = group_by_cgroup(&memberships);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].label, "a.slice/app.service");
        assert_eq!(groups[0].members, vec![ProcessKey::new(100, 10)]);
        assert_eq!(groups[1].label, "b.slice/app.service");
        assert_eq!(
            groups[1].members,
            vec![ProcessKey::new(200, 20), ProcessKey::new(300, 30)]
        );
    }

    #[test]
    fn cgroup_grouping_ignores_empty_labels() {
        let groups = group_by_cgroup(&[ProcessCgroup {
            key: ProcessKey::new(1, 0),
            label: String::new(),
        }]);
        assert!(
            groups.is_empty(),
            "an unreadable cgroup must not create a group"
        );
    }

    // --- R1-I10 / R1-I11 / R1-I12: snapshot integration -----------------------

    #[test]
    fn default_snapshot_carries_honest_power_and_empty_groups() {
        let snapshot = SystemSnapshot::default();
        assert_eq!(snapshot.power.source, PowerSource::Unknown);
        assert!(snapshot.power.batteries.is_empty());
        assert_eq!(
            snapshot.sampling,
            SamplingPlan::default(),
            "a snapshot without power data must not throttle"
        );
        assert!(snapshot.cgroups.is_empty());
    }

    #[test]
    fn sampling_plan_is_a_floor_not_an_accelerator() {
        let configured = std::time::Duration::from_millis(1_000);
        // Interactive: the existing 1 s cadence is byte-for-byte unchanged.
        assert_eq!(
            SamplingPlan {
                interval_ms: INTERACTIVE_INTERVAL_MS,
                profile: SamplingProfile::Interactive,
            }
            .effective_interval(configured),
            configured
        );
        // Battery and background plans slow a too-fast setting down.
        for (interval_ms, profile) in [
            (BATTERY_INTERVAL_MS, SamplingProfile::Battery),
            (BATTERY_SAVER_INTERVAL_MS, SamplingProfile::BatterySaver),
            (BACKGROUND_INTERVAL_MS, SamplingProfile::Background),
        ] {
            assert_eq!(
                SamplingPlan {
                    interval_ms,
                    profile,
                }
                .effective_interval(configured),
                std::time::Duration::from_millis(interval_ms)
            );
        }
        // A deliberately slower user setting is never sped up.
        let slow = std::time::Duration::from_secs(10);
        assert_eq!(
            SamplingPlan {
                interval_ms: BATTERY_INTERVAL_MS,
                profile: SamplingProfile::Battery,
            }
            .effective_interval(slow),
            slow
        );
    }
}
