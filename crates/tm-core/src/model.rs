//! Domain value objects shared by every shell.
//!
//! This module has no I/O, no clock and no platform dependency, so it compiles
//! for native targets and for `wasm32-unknown-unknown` alike. Every field is
//! plain data: the platform adapter produces it, the view-model consumes it.

use std::fmt;

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

    /// Elapsed whole seconds between `earlier` and `self` (saturating).
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
    #[default]
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
    /// double-count the overview rate (Runtime Proof: `tailscale0` reported
    /// 79.9 MB received while `enp4s0` reported 124.6 GB, i.e. the VPN traffic
    /// was counted twice).
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
    const VIRTUAL_PREFIXES: [&str; 8] = [
        "br-",
        "docker",
        "veth",
        "virbr",
        "lxcbr",
        "tailscale",
        "tun",
        "tap",
    ];
    if name == "lo" {
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
    /// When the sample was taken.
    pub sampled_at: Timestamp,
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

#[cfg(test)]
mod tests {
    use super::*;

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
            "tailscale0",
            "tun0",
            "tap0",
        ] {
            assert!(is_virtual_interface(name), "{name} must be virtual");
        }
        for name in ["enp4s0", "eth0", "wlan0", "wlp3s0", "en0", "wlan1"] {
            assert!(!is_virtual_interface(name), "{name} must be physical");
        }
    }

    #[test]
    fn physical_totals_exclude_loopback_and_virtual_interfaces() {
        // Mirrors the development host: the VPN interface duplicates traffic
        // that is also counted on enp4s0.
        let network = NetworkSnapshot {
            interfaces: vec![
                NetworkInterfaceSnapshot {
                    name: "enp4s0".into(),
                    received_bytes: 124_641_412_687,
                    transmitted_bytes: 28_381_393_862,
                },
                NetworkInterfaceSnapshot {
                    name: "tailscale0".into(),
                    received_bytes: 79_901_838,
                    transmitted_bytes: 35_192_774,
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
        assert_eq!(network.physical_totals(), (124_641_412_687, 28_381_393_862));
        // The all-interface total still double-counts, which is why the
        // overview uses physical_totals().
        assert_eq!(network.totals(), (124_721_315_525, 28_416_588_636));
    }
}
