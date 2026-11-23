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
    /// Swap used by this process in bytes (`VmSwap` from `/proc/<pid>/status`
    /// on Linux, converted from kB); `0` when unavailable or on other targets.
    ///
    /// The sum over all processes need not equal
    /// [`MemorySnapshot::swap_used_bytes`] (SwapUsed): shared swapped pages can
    /// be charged to more than one process, some swapped pages are not
    /// attributable to any sampled process, and per-process reads happen at
    /// slightly different instants from the system-wide counter (取值競合,
    /// sampling race). Treat these as independent measurements; do not assert
    /// numeric equality. `serde(default)` keeps older wire snapshots readable.
    #[cfg_attr(feature = "serde", serde(default))]
    pub swap_bytes: u64,
    /// Cumulative block-layer bytes read by this process, taken verbatim from
    /// the `read_bytes` line of Linux `/proc/<pid>/io` (already bytes, unlike
    /// `VmSwap`'s kB). This is **not** `rchar`: cached reads, pipe and network
    /// traffic counted by `rchar` are excluded, so the figure describes actual
    /// block I/O. `0` when unavailable or on other targets; `serde(default)`
    /// keeps older wire snapshots readable.
    #[cfg_attr(feature = "serde", serde(default))]
    pub read_bytes: u64,
    /// Cumulative block-layer bytes written by this process, taken verbatim
    /// from the `write_bytes` line of Linux `/proc/<pid>/io` (already bytes).
    /// Like [`Self::read_bytes`] this excludes `wchar`'s cached writes; `0` when
    /// unavailable or on other targets. `serde(default)` keeps older wire
    /// snapshots readable.
    #[cfg_attr(feature = "serde", serde(default))]
    pub write_bytes: u64,
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

/// Lower bound for the sampling interval when the window is visible on mains power (R1-I11).
///
/// R2-DEMO: this used to be 1 000 ms, which silently raised the advertised
/// 0.5 s refresh preset to 1 s via [`SamplingPlan::effective_interval`]. Both
/// shells already accept a 250 ms floor in their settings, and the 0.5 s
/// preset is shown in the UI, so lowering this floor to 500 ms makes the
/// setting honest. A UI-only "currently limited to 1 s" notice was rejected:
/// it would keep a selectable preset that never takes effect. The default
/// configured interval stays 1 000 ms, so an untouched installation keeps its
/// exact cadence; only users who pick 0.5 s get faster sampling.
pub const INTERACTIVE_INTERVAL_MS: u64 = 500;
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
    /// The interactive floor is [`INTERACTIVE_INTERVAL_MS`] (500 ms), so the
    /// 0.5 s preset now takes effect while the default 1 s keeps its exact
    /// behaviour. A battery / saver / background plan still slows a too-fast
    /// setting down to its recommended delay. A deliberately slower user
    /// setting is always respected.
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

// --- Application (executable-derived) grouping -------------------------------
//
// Additive read-only helper: the grouping key is derived from the executable
// that actually launched the process (argv[0]'s basename, with wrapper commands
// such as `sudo` unwrapped) and falls back to the already-redacted `name`.
// Nothing is written back into `SystemSnapshot`, so the JSON wire envelope,
// the CSV exporter and the redact pass keep their exact byte contracts.
//
// Both `name` and `command` are redacted at the snapshot boundary by
// [`crate::redact::redact_snapshot`]; this function only slices those fields, so
// it cannot introduce a new identifier into a key. Only the executable token is
// inspected: token flags (`--token=…`) and URLs in later arguments never become
// part of a key.

/// Upper bound for an app group key, aligned with the process-name display cap.
const APP_GROUP_KEY_MAX_CHARS: usize = 64;

/// Launcher/wrapper executables whose own name says nothing about the app run.
///
/// Unwrapping them is what keeps many same-`comm` processes (`sudo` appears
/// 112 times in the measured session) from collapsing into a single group.
const EXEC_WRAPPERS: &[&str] = &[
    "sudo", "doas", "su", "sh", "bash", "dash", "zsh", "ksh", "env", "nice", "renice", "nohup",
    "setsid", "time", "ionice", "chrt", "taskset", "busybox",
];

/// Wrapper options whose next token is a value (user/group name), not an app.
const WRAPPER_VALUE_OPTIONS: &[&str] = &["-u", "--user", "-g", "--group", "-C", "--close-from"];

/// Extracts the basename of one argv token, ignoring optional shell quotes.
fn executable_basename(token: &str) -> &str {
    let token = token.trim_matches(|ch| ch == '"' || ch == '\'');
    token.rsplit(['/', '\\']).next().unwrap_or(token)
}

/// Rejects tokens that cannot be an executable name.
///
/// Flags, redaction placeholders (`<home>`), scrubbed tokens (`***`) and
/// URL-ish fragments are excluded so a grouping key can never carry an
/// argument value such as `--token=secret` or `https://host/path`.
fn is_usable_app_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= APP_GROUP_KEY_MAX_CHARS
        && !name.starts_with('-')
        && !name.starts_with('<')
        && !name.starts_with('*')
        // Kernel threads and systemd internals show argv[0] like `(sd-pam)`
        // or a bare number. Those are not application names; fall back to
        // the process `comm` instead of showing a meaningless group label.
        && !name.starts_with('(')
        && !name.chars().all(|ch| ch.is_ascii_digit())
        && name.chars().all(|ch| {
            !ch.is_whitespace()
                && !ch.is_control()
                && !matches!(ch, '=' | ':' | '?' | '\\' | '"' | '\'')
        })
}

/// Maps binary-name variants onto the name their parent/children share.
///
/// Firefox ships its real executable as `firefox-bin` while content processes
/// are launched through `firefox`; without this normalisation a parent and its
/// `Isolated Web Co` children would land in two groups. The suffix is generic
/// (any `name-bin`), so the rule does not hard-code one browser.
fn normalise_app_name(basename: &str) -> &str {
    basename
        .strip_suffix("-bin")
        .filter(|stem| !stem.is_empty())
        .unwrap_or(basename)
}

/// Derives a stable, already-redacted application key for one process.
///
/// Priority:
///
/// 1. the basename of `argv[0]` from [`ProcessSnapshot::command`];
/// 2. when `argv[0]` is a launcher (`sudo`, `sh`, `env`, …), the basename of the
///    first real executable token after its flags and assignments;
/// 3. when `command` has no parseable executable, [`ProcessSnapshot::name`].
///
/// Both source fields have passed through [`crate::redact::redact_snapshot`] at
/// the boundary, so the returned slice is redacted. Later arguments are never
/// returned, which keeps `--token=…` values and URLs out of grouping keys and
/// out of the SVG/CSV/wire contracts (the group rows are not serialised).
/// A process with an empty `name` has no trustworthy identity and returns `""`,
/// preserving the established "no group" behaviour for unidentifiable tasks.
pub fn app_group_key(process: &ProcessSnapshot) -> &str {
    if process.name.is_empty() {
        return "";
    }
    let mut argv = process.command.split_whitespace();
    let Some(argv0) = argv.next() else {
        return &process.name;
    };
    let first = executable_basename(argv0);
    if argv0.contains("://") || !is_usable_app_name(first) {
        return &process.name;
    }
    if !EXEC_WRAPPERS.contains(&first) {
        return normalise_app_name(first);
    }
    // Skip wrapper flags, `KEY=value` assignments, and the value consumed by
    // options like `sudo -u root`; otherwise "root" would become the app name.
    let mut skip_option_value = false;
    for token in argv {
        if skip_option_value {
            skip_option_value = false;
            continue;
        }
        let candidate = executable_basename(token);
        if WRAPPER_VALUE_OPTIONS.contains(&candidate) {
            skip_option_value = true;
            continue;
        }
        if candidate.starts_with('-')
            || candidate.starts_with('[')
            || token.contains('=')
            || token.contains("://")
        {
            continue;
        }
        if is_usable_app_name(candidate) {
            return normalise_app_name(candidate);
        }
    }
    // No executable in the wrapper's arguments: fall back to comm, never to a
    // later token that might be a token value or URL.
    &process.name
}

/// Processes sharing one application key, with pre-computed totals (R1-GROUP).
///
/// `key` and `label` are both the redacted [`app_group_key`] used for grouping;
/// they are kept as separate fields so a shell can use `key` as a stable
/// identity for expansion state and `label` as display text without depending
/// on the other's representation.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProcessGroup {
    /// Redacted [`app_group_key`] shared by every member.
    pub key: String,
    /// Display label; identical to `key` (the app name is already display-safe).
    pub label: String,
    /// Indices into the source process slice, ordered by [`ProcessKey`].
    ///
    /// The first index therefore belongs to the member with the smallest PID
    /// (ties broken by start time), which is the identity a group row selects.
    #[cfg_attr(feature = "serde", serde(default))]
    pub members: Vec<usize>,
    /// Sum of the members' CPU percentages, as `f64` then narrowed to `f32`.
    ///
    /// Non-finite and negative readings contribute zero. The value is *not*
    /// clamped to 100: a multicore application can legitimately exceed it.
    pub cpu_percent: f32,
    /// Saturating sum of the members' resident memory in bytes.
    pub memory_bytes: u64,
    /// Number of grouped processes; always `members.len()`.
    pub process_count: usize,
}

/// Groups processes by their executable-derived app key (R1-GROUP).
///
/// The key comes from [`app_group_key`]: the redacted `argv[0]` basename, with
/// launcher wrappers unwrapped so browser children (`Isolated Web Co`,
/// `MainThread`) join their parent's group. Only that one executable token is
/// consulted — differing arguments (including token flags and URLs) neither
/// split a group nor leak into its key. Processes without a usable key are
/// skipped, mirroring [`group_by_cgroup`]'s empty-label rule.
///
/// Groups are ordered by key via `BTreeMap`; members are sorted by [`ProcessKey`]
/// (then by source index for the unlikely case of duplicate identities). CPU is
/// accumulated in `f64` and only then narrowed to `f32`, while memory uses
/// `saturating_add`, so neither a NaN reading nor an overflowing counter can
/// corrupt the totals. The function is pure and allocates only the result.
pub fn group_by_app(processes: &[ProcessSnapshot]) -> Vec<ProcessGroup> {
    let mut grouped = std::collections::BTreeMap::<String, Vec<usize>>::new();
    for (index, process) in processes.iter().enumerate() {
        let key = app_group_key(process);
        if key.is_empty() {
            continue;
        }
        grouped.entry(key.to_string()).or_default().push(index);
    }
    grouped
        .into_iter()
        .map(|(key, mut members)| {
            members.sort_unstable_by(|&left, &right| {
                processes[left]
                    .key
                    .cmp(&processes[right].key)
                    .then_with(|| left.cmp(&right))
            });
            let cpu_total = members.iter().fold(0.0_f64, |sum, &index| {
                let value = processes[index].cpu_percent;
                // Non-finite readings are display-normalised to zero at the
                // same boundary used by `normalise_percent`; negative finite
                // readings are displayed as zero too. No upper clamp: a
                // multicore app can exceed 100%.
                if value.is_finite() {
                    sum + f64::from(value.max(0.0))
                } else {
                    sum
                }
            });
            let cpu_percent = if cpu_total.is_finite() {
                cpu_total as f32
            } else {
                0.0
            };
            let memory_bytes = members.iter().fold(0_u64, |sum, &index| {
                sum.saturating_add(processes[index].memory_bytes)
            });
            let process_count = members.len();
            ProcessGroup {
                label: key.clone(),
                key,
                members,
                cpu_percent,
                memory_bytes,
                process_count,
            }
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
    #[cfg(feature = "serde")]
    fn process_io_bytes_default_to_zero_on_older_wire_snapshots() {
        // C1-IODATA: an agent that predates the cumulative IO fields must still
        // deserialise; the missing fields become honest zeros, not an error.
        let json = r#"{
            "key": {"pid": 42, "start_time_secs": 1},
            "name": "p",
            "command": "p",
            "user": null,
            "status": "Running",
            "cpu_percent": 0.0,
            "memory_bytes": 0,
            "virtual_memory_bytes": 0,
            "threads": null,
            "parent_pid": null,
            "swap_bytes": 0
        }"#;
        let process: ProcessSnapshot =
            serde_json::from_str(json).expect("legacy process must still deserialise");
        assert_eq!(process.key.pid, 42);
        assert_eq!((process.read_bytes, process.write_bytes), (0, 0));
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

    // --- R1-GROUP: application grouping --------------------------------------

    fn named_process(pid: u32, name: &str, cpu: f32, memory: u64) -> ProcessSnapshot {
        ProcessSnapshot {
            key: ProcessKey::new(pid, 1),
            name: name.to_string(),
            // The executable basename still equals `name`; only the trailing
            // argument differs. This keeps the "command-ignoring bug" regression
            // meaningful under the new argv[0]-derived key (R1-GROUP).
            command: format!("/usr/bin/{name} --pid={pid}"),
            cpu_percent: cpu,
            memory_bytes: memory,
            ..ProcessSnapshot::default()
        }
    }

    fn command_process(pid: u32, name: &str, command: &str) -> ProcessSnapshot {
        ProcessSnapshot {
            key: ProcessKey::new(pid, 1),
            name: name.to_string(),
            command: command.to_string(),
            ..ProcessSnapshot::default()
        }
    }

    #[test]
    fn app_group_sums_cpu_and_memory_and_ignores_command() {
        let processes = vec![
            named_process(30, "firefox", 0.25, 300),
            named_process(10, "firefox", 1.5, 100),
            named_process(20, "firefox", 2.25, 200),
        ];
        let groups = group_by_app(&processes);
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.key, "firefox");
        assert_eq!(group.label, "firefox");
        // Golden: 1.5 + 2.25 + 0.25 = exactly 4.0 in f32 as well.
        assert_eq!(group.cpu_percent, 4.0);
        assert_eq!(group.memory_bytes, 600);
        assert_eq!(group.process_count, 3);
        assert_eq!(group.members, vec![1, 2, 0]);
    }

    #[test]
    fn app_group_handles_empty_and_single_member_inputs() {
        assert!(group_by_app(&[]).is_empty());
        let groups = group_by_app(&[named_process(1, "", 5.0, 10)]);
        assert!(
            groups.is_empty(),
            "an empty process name must not create a group"
        );
        let groups = group_by_app(&[named_process(7, "solo", 12.5, 42)]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members, vec![0]);
        assert_eq!(groups[0].process_count, 1);
        assert_eq!(groups[0].cpu_percent, 12.5);
        assert_eq!(groups[0].memory_bytes, 42);
    }

    #[test]
    fn app_group_normalises_non_finite_cpu_and_saturates_memory() {
        let nan = named_process(1, "mixed", f32::NAN, u64::MAX);
        let positive_infinity = named_process(2, "mixed", f32::INFINITY, 1);
        let negative_infinity = named_process(3, "mixed", f32::NEG_INFINITY, 1);
        let negative = named_process(4, "mixed", -3.0, 2);
        let groups = group_by_app(&[nan, positive_infinity, negative_infinity, negative]);
        assert_eq!(
            groups[0].cpu_percent, 0.0,
            "NaN/±Inf and negative readings contribute zero"
        );
        assert_eq!(
            groups[0].memory_bytes,
            u64::MAX,
            "memory additions must saturate, not wrap"
        );
    }

    #[test]
    fn app_group_does_not_clamp_cpu_at_one_hundred() {
        let groups = group_by_app(&[
            named_process(1, "render", 80.0, 1),
            named_process(2, "render", 70.0, 1),
        ]);
        assert_eq!(groups[0].cpu_percent, 150.0);
    }

    #[test]
    fn app_group_order_and_members_are_deterministic() {
        // Reverse input order must still produce identical groups: BTreeMap for
        // keys and ProcessKey for members, just like `group_by_cgroup`.
        let processes = vec![
            named_process(90, "zeta", 1.0, 1),
            named_process(5, "alpha", 2.0, 2),
            named_process(4, "zeta", 3.0, 3),
            named_process(2, "alpha", 4.0, 4),
        ];
        let forward = group_by_app(&processes);
        let reversed = group_by_app(&processes.clone().into_iter().rev().collect::<Vec<_>>());
        // Group keys and aggregates are input-order independent; `members`
        // holds indices into each input slice, so it necessarily differs.
        assert_eq!(
            forward
                .iter()
                .map(|g| (
                    g.key.as_str(),
                    g.cpu_percent,
                    g.memory_bytes,
                    g.process_count
                ))
                .collect::<Vec<_>>(),
            reversed
                .iter()
                .map(|g| (
                    g.key.as_str(),
                    g.cpu_percent,
                    g.memory_bytes,
                    g.process_count
                ))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            forward.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "zeta"]
        );
        // Members are ordered by ProcessKey (smallest PID first), so the first
        // index of each group is the identity a group row selects.
        assert_eq!(forward[0].members, vec![3, 1]);
        assert_eq!(forward[1].members, vec![2, 0]);
        assert_eq!(processes[forward[0].members[0]].key.pid(), 2);
        assert_eq!(processes[forward[1].members[0]].key.pid(), 4);
    }

    // --- C1-GROUPKEY: executable-derived keys --------------------------------

    #[test]
    fn app_group_key_uses_argv0_basename_and_falls_back_to_name() {
        // The browser's children have an unhelpful `comm`; the executable in
        // argv[0] is what identifies their application.
        assert_eq!(
            app_group_key(&command_process(
                1,
                "Isolated Web Co",
                "/usr/lib/firefox/firefox --type=renderer --user-data-dir=/tmp/x"
            )),
            "firefox"
        );
        assert_eq!(
            app_group_key(&command_process(
                2,
                "MainThread",
                "/usr/lib/chromium/chromium --type=renderer"
            )),
            "chromium"
        );
        // Empty/unparseable argv falls back to the redacted name.
        assert_eq!(
            app_group_key(&command_process(3, "worker", "   ")),
            "worker"
        );
        assert_eq!(
            app_group_key(&command_process(4, "worker", "--token=supersecret")),
            "worker",
            "a flag is not an executable"
        );
        assert_eq!(
            app_group_key(&command_process(5, "worker", "https://host/path")),
            "worker",
            "a URL is not an executable"
        );
        // The established empty-comm rule is preserved.
        assert_eq!(app_group_key(&command_process(6, "", "kernel-task")), "");
    }

    #[test]
    fn app_group_key_normalises_the_firefox_binary_suffix() {
        // Measured session: the parent's comm/command is `firefox-bin` while its
        // children are launched through `firefox`. Both must be `firefox`.
        assert_eq!(
            app_group_key(&command_process(
                1,
                "firefox-bin",
                "/usr/lib/firefox/firefox-bin"
            )),
            "firefox"
        );
        assert_eq!(
            app_group_key(&command_process(
                2,
                "Isolated Web Co",
                "/usr/lib/firefox/firefox --type=renderer"
            )),
            "firefox"
        );
    }

    #[test]
    fn browser_children_join_the_parent_app_group() {
        // Fictional but realistic command strings: the child processes carry a
        // generic comm and only argv[0] ties them to the parent application.
        let processes = vec![
            command_process(100, "firefox-bin", "/usr/lib/firefox/firefox-bin"),
            command_process(
                101,
                "Isolated Web Co",
                "/usr/lib/firefox/firefox --type=renderer --user-data-dir=/tmp/profile",
            ),
            command_process(
                102,
                "Isolated Web Co",
                "/usr/lib/firefox/firefox --type=utility --token=supersecret",
            ),
            command_process(200, "chromium", "/usr/lib/chromium/chromium"),
            command_process(
                201,
                "MainThread",
                "/usr/lib/chromium/chromium --type=renderer",
            ),
        ];
        let groups = group_by_app(&processes);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, "chromium");
        assert_eq!(groups[0].process_count, 2);
        assert_eq!(groups[1].key, "firefox");
        assert_eq!(groups[1].label, "firefox");
        assert_eq!(groups[1].process_count, 3);
        assert_eq!(groups[1].members, vec![0, 1, 2]);
    }

    #[test]
    fn launcher_wrappers_do_not_merge_distinct_apps() {
        // Same `comm` (sudo/sh) but different argv must not collapse into one
        // group: use the executable the wrapper actually launched.
        let processes = vec![
            command_process(1, "sudo", "sudo /usr/bin/apt update"),
            command_process(2, "sudo", "sudo /usr/bin/systemctl restart service"),
            command_process(3, "sudo", "sudo -u root /usr/bin/docker ps"),
            command_process(4, "sh", "sh /etc/init.d/ssh start"),
            command_process(5, "sh", "sh -c \"/usr/bin/rsync --daemon\""),
        ];
        let keys: Vec<_> = processes.iter().map(app_group_key).collect();
        assert_eq!(keys, ["apt", "systemctl", "docker", "ssh", "rsync"]);
        let groups = group_by_app(&processes);
        assert_eq!(
            groups.len(),
            5,
            "different argv must stay in separate groups"
        );
        assert!(groups.iter().all(|group| group.process_count == 1));
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
        let interactive = SamplingPlan {
            interval_ms: INTERACTIVE_INTERVAL_MS,
            profile: SamplingProfile::Interactive,
        };
        let configured = std::time::Duration::from_millis(1_000);
        // The default 1 s cadence is byte-for-byte unchanged.
        assert_eq!(interactive.effective_interval(configured), configured);
        // R2-DEMO: the advertised 0.5 s preset must survive the floor, and an
        // even faster raw setting is still raised to 500 ms.
        let fast = std::time::Duration::from_millis(500);
        assert_eq!(interactive.effective_interval(fast), fast);
        assert_eq!(
            interactive.effective_interval(std::time::Duration::from_millis(250)),
            fast
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
