//! Native [`tm_core::SystemProbe`] implementation.
//!
//! This crate is the only place in the workspace that talks to the operating
//! system directly:
//!
//! - [`sysinfo`] 0.40 collects CPU, memory, process, network and disk data.
//! - `rustix` sends POSIX signals with its safe API on Unix and reads the
//!   filesystem's `f_bfree` counter through its safe `fs::statvfs` wrapper, so
//!   the crate stays free of `unsafe` code.
//!
//! # Platform support
//!
//! - **Unix (Linux, macOS, ...)**: `sysinfo` samples the machine and `rustix`
//!   delivers `SIGTERM` / `SIGKILL` / `SIGSTOP` / `SIGCONT`. Thread counts are
//!   only collected on Linux (`sysinfo` exposes no task list elsewhere), so
//!   macOS and other Unix targets report `threads: None`.
//! - **Windows**: `sysinfo` samples the machine and `taskkill /F` (invoked by
//!   absolute path under `%SystemRoot%\System32`, never through `PATH`)
//!   terminates processes. Windows has no POSIX signals, so `Terminate` and
//!   `Kill` both end the process forcefully and `Stop` / `Continue` return
//!   [`ControlError::Unsupported`].
//!
//! The safety rules (never PID 0, never PID 1 on Unix where it is the init
//! system, never the app itself; re-check the start time immediately before
//! signalling) are applied before the platform-specific branch. PID 1 is an
//! ordinary, legitimately controllable process on Windows (R2X-P6).
//!
//! The UI never depends on this crate; it only sees [`tm_core::model`] value
//! objects. Keeping the platform code behind the [`SystemProbe`] port is what
//! lets the same view-model run against the native probe, the deterministic
//! demo probe, or (in the browser) an agent-backed probe.
//!
//! # Process list
//!
//! Requesting thread information during a full (`ProcessesToUpdate::All`)
//! refresh makes `sysinfo` insert every Linux thread as an additional process
//! entry. The probe avoids that: the full refresh runs with tasks disabled and
//! a separate, batched `ProcessesToUpdate::Some` refresh collects each
//! process's task ID set. Threads are therefore never inserted into the process
//! map, the snapshot stays one row per process, and the thread count is still
//! reported in [`tm_core::model::ProcessSnapshot::threads`]. A userland-thread
//! filter remains as a defensive backstop.
//!
//! # Process identity
//!
//! PIDs are reused by the kernel. Every [`tm_core::model::ProcessKey`] therefore
//! stores the PID **and** the process start time. [`NativeProbe::control`]
//! re-reads the start time immediately before signalling and refuses the
//! request when it no longer matches, so a recycled PID can never be signalled
//! by mistake.
//!
//! # CPU sampling
//!
//! `sysinfo` computes CPU usage from the delta between two refreshes, so the
//! first sample of a process's lifetime reports `0.0`. Callers that want
//! meaningful numbers should sample periodically (the UI does this once per
//! refresh interval).
//!
//! Per-process CPU percentages are **not** taken from `Process::cpu_usage()`.
//! That value is derived from the global CPU delta at the end of the process
//! refresh; when a process refresh takes longer than
//! `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL` (common on machines with thousands of
//! processes) the delta collapses and every busy process saturates at
//! `cores * 100`. Instead the probe keeps `Process::accumulated_cpu_time()`
//! (CPU-milliseconds) per `ProcessKey` and divides the delta by the measured
//! wall-clock interval, which is stable regardless of how long the refresh
//! takes.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::collections::HashMap;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use rustix::io::Errno;
#[cfg(unix)]
use rustix::process::{kill_process, Pid as RustixPid, Signal as RustixSignal};
use sysinfo::{
    Components, CpuRefreshKind, Disks, MemoryRefreshKind, Networks, Pid, ProcessRefreshKind,
    ProcessStatus as SysinfoProcessStatus, ProcessesToUpdate, RefreshKind, System, ThreadKind,
    UpdateKind, Users,
};
use tm_core::model::{
    cgroup_label, group_by_cgroup, is_virtual_interface, recommended_sampling_plan,
    BatterySnapshot, BatteryState, CgroupGroup, ComponentSnapshot, CpuSnapshot, DiskUsageSnapshot,
    MemorySnapshot, NetworkInterfaceSnapshot, NetworkSnapshot, PowerSnapshot, PowerSource,
    ProcessCgroup, ProcessKey, ProcessSnapshot, ProcessStatus, SamplingPlan, SystemSnapshot,
    Timestamp, WindowVisibility,
};
use tm_core::port::{ControlError, PlatformInfo, ProbeError, Signal, SystemProbe};
use tm_core::sanitize::{sanitize_display, MAX_COMMAND_CHARS, MAX_NAME_CHARS};

/// Native [`SystemProbe`] backed by `sysinfo` and `rustix`.
///
/// The probe keeps one `sysinfo::System` alive across samples: `sysinfo` derives
/// CPU percentages from the difference between two refreshes, so reusing the
/// handle is required for correct values.
pub struct NativeProbe {
    system: System,
    networks: Option<Networks>,
    disks: Option<Disks>,
    users: Option<Users>,
    hostname: Option<String>,
    os_name: Option<String>,
    kernel_version: Option<String>,
    os_label: String,
    /// Accumulated CPU time (CPU-milliseconds) of every process at the previous
    /// sample, keyed by PID + start time.
    previous_cpu_ms: HashMap<ProcessKey, u64>,
    /// When the previous sample was taken, used to turn CPU-time deltas into
    /// percentages.
    last_sample_at: Option<Instant>,
}

impl NativeProbe {
    /// Creates a probe and takes the initial sample that `sysinfo` needs before
    /// it can compute CPU deltas.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeError::Io`] when `sysinfo` cannot initialise its view of
    /// the machine (for example on an unsupported target). Network and disk
    /// enumeration failures are non-fatal: those sections are then reported as
    /// empty instead of failing the whole sample.
    pub fn new() -> Result<Self, ProbeError> {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything())
                .with_processes(process_refresh_kind()),
        )
        .map_err(|err| ProbeError::Io(format!("sysinfo の初期化に失敗: {err}")))?;

        let hostname = System::host_name()
            .ok()
            .map(|host| sanitize_display(&host, MAX_NAME_CHARS))
            .filter(|host| !host.is_empty());
        // Sanitise every OS-provided string: a compromised host value must not
        // inject control characters into the status bar or logs (R2X-D2).
        let os_name = System::name()
            .ok()
            .map(|value| sanitize_display(&value, MAX_NAME_CHARS))
            .filter(|value| !value.is_empty());
        let kernel_version = System::kernel_version()
            .ok()
            .map(|value| sanitize_display(&value, MAX_NAME_CHARS))
            .filter(|value| !value.is_empty());
        let os_label = System::long_os_version()
            .ok()
            .or_else(|| os_name.clone())
            .unwrap_or_else(|| std::env::consts::OS.to_string());
        let os_label = sanitize_display(&os_label, MAX_NAME_CHARS);

        Ok(Self {
            system,
            networks: Networks::new_with_refreshed_list().ok(),
            disks: Disks::new_with_refreshed_list().ok(),
            users: Users::new_with_refreshed_list().ok(),
            hostname,
            os_name,
            kernel_version,
            os_label,
            previous_cpu_ms: HashMap::new(),
            last_sample_at: None,
        })
    }

    /// Takes one sample without going through the [`SystemProbe`] trait.
    ///
    /// This is a convenience for callers that hold a concrete `NativeProbe`.
    ///
    /// # Errors
    ///
    /// Currently never fails; the `Result` mirrors [`SystemProbe::sample`] so
    /// callers can stay generic over the port.
    pub fn sample_native(&mut self) -> Result<SystemSnapshot, ProbeError> {
        self.refresh();
        // Take the timestamp after refresh: the CPU counters are read during
        // `build_snapshot`, so a pre-refresh `Instant` makes the denominator
        // shorter than the counter interval whenever refresh time varies
        // (R49 audit). This also keeps `sampled_elapsed` consistent.
        let now = Instant::now();
        let elapsed_ms = self
            .last_sample_at
            .map(|last| now.duration_since(last).as_millis())
            .map(|millis| u64::try_from(millis).unwrap_or(u64::MAX))
            .unwrap_or(0);
        let (snapshot, next_cpu_ms) = self.build_snapshot(elapsed_ms);
        self.previous_cpu_ms = next_cpu_ms;
        self.last_sample_at = Some(now);
        Ok(snapshot)
    }

    /// Refreshes every data source held by the probe.
    ///
    /// CPU usage is refreshed first so per-core values describe the interval
    /// since the previous sample. Process CPU percentages are **not** taken
    /// from `sysinfo`'s `Process::cpu_usage()`: that value is derived from the
    /// global CPU delta at the end of the process refresh, and a process
    /// refresh that takes longer than
    /// `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL` makes the delta collapse and the
    /// value saturate at `cores * 100`. See `sample_native` for the
    /// accumulated-CPU-time based calculation instead.
    fn refresh(&mut self) {
        self.system
            .refresh_cpu_specifics(CpuRefreshKind::everything());
        self.system
            .refresh_memory_specifics(MemoryRefreshKind::everything());
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            process_refresh_kind(),
        );
        // Collect task IDs in a second pass so sysinfo never inserts thread
        // entries into its process map (R4-A01). `Some(&pids)` mode keeps the
        // task set on each process but clears the thread list it would have
        // added to the map.
        #[cfg(target_os = "linux")]
        refresh_thread_counts(&mut self.system);
        if let Some(networks) = self.networks.as_mut() {
            networks.refresh(true);
        }
        if let Some(disks) = self.disks.as_mut() {
            disks.refresh(true);
        }
        // The uid -> name mapping is not static: users and groups can be added
        // or renamed while the app runs. `Users` was only populated once in
        // `new()`, so without this refresh the user column goes stale (R2X-P7).
        if let Some(users) = self.users.as_mut() {
            users.refresh();
        }
    }

    /// Builds the domain snapshot and the CPU-time bookkeeping for the next
    /// sample.
    fn build_snapshot(&self, elapsed_ms: u64) -> (SystemSnapshot, HashMap<ProcessKey, u64>) {
        let (processes, next_cpu_ms) = self.process_snapshots(elapsed_ms);
        // One power read feeds both the snapshot and its sampling plan: two
        // sysfs reads could straddle a state change and disagree.
        let power = self.power_snapshot();
        let sampling = recommended_sampling_plan(&power, WindowVisibility::Visible);
        let snapshot = SystemSnapshot {
            sampled_at: now_millis(),
            sampled_elapsed: std::time::Duration::from_millis(elapsed_ms),
            hostname: self.hostname.clone(),
            os_name: self.os_name.clone(),
            kernel_version: self.kernel_version.clone(),
            cpu: self.cpu_snapshot(),
            memory: self.memory_snapshot(),
            processes,
            network: self.network_snapshot(),
            disks: self.disk_snapshots(),
            components: self.component_snapshots(),
            power,
            sampling,
            // Membership collection only reads /proc: no sysinfo refresh, so
            // the one-row-per-process and threads contracts stay intact.
            cgroups: group_by_cgroup(&self.cgroup_memberships()),
            self_pid: Some(std::process::id()),
        };
        (snapshot, next_cpu_ms)
    }

    fn component_snapshots(&self) -> Vec<ComponentSnapshot> {
        let Ok(components) = Components::new_with_refreshed_list() else {
            return Vec::new();
        };
        components
            .iter()
            .map(|component| {
                let label = sanitize_display(component.label(), MAX_NAME_CHARS);
                ComponentSnapshot {
                    label,
                    temperature_c: component.temperature(),
                    max_c: component.max(),
                    critical_c: component.critical(),
                }
            })
            .collect()
    }

    fn cpu_snapshot(&self) -> CpuSnapshot {
        let cpus = self.system.cpus();
        let per_core_percent = cpus
            .iter()
            .map(|cpu| cpu.usage().clamp(0.0, 100.0))
            .collect::<Vec<_>>();
        // `global_cpu_usage()` is refreshed again at the end of the process
        // refresh, so it can describe a much shorter window than the per-core
        // values. The mean of the per-core values describes the sample
        // interval consistently.
        let total_percent = if per_core_percent.is_empty() {
            0.0
        } else {
            (per_core_percent.iter().sum::<f32>() / per_core_percent.len() as f32).clamp(0.0, 100.0)
        };
        let load_average = System::load_average()
            .map(|load| [load.one as f32, load.five as f32, load.fifteen as f32])
            .unwrap_or([0.0; 3]);
        let frequency_mhz = average_frequency_mhz(cpus);
        CpuSnapshot {
            total_percent,
            per_core_percent,
            load_average,
            uptime_secs: System::uptime().unwrap_or(0),
            frequency_mhz,
        }
    }

    fn memory_snapshot(&self) -> MemorySnapshot {
        MemorySnapshot {
            total_bytes: self.system.total_memory(),
            used_bytes: self.system.used_memory(),
            available_bytes: self.system.available_memory(),
            swap_total_bytes: self.system.total_swap(),
            swap_used_bytes: self.system.used_swap(),
        }
    }

    /// Builds the process list and records the accumulated CPU time of every
    /// process for the next sample.
    ///
    /// `cpu_percent` is `delta(accumulated_cpu_ms) / delta(wall_ms) * 100`, so
    /// it is `0.0` for the first sample of a process (matching `sysinfo`'s
    /// documented first-sample behaviour) and can exceed 100 on multicore
    /// systems.
    fn process_snapshots(
        &self,
        elapsed_ms: u64,
    ) -> (Vec<ProcessSnapshot>, HashMap<ProcessKey, u64>) {
        let mut next_cpu_ms = HashMap::with_capacity(self.system.processes().len());
        let mut processes = self
            .system
            .processes()
            .iter()
            // The full refresh now runs with tasks disabled and thread counts
            // are collected separately (see `refresh_thread_counts`), so
            // userland thread entries should never appear here. Keep the
            // filter as a backstop: a task manager table must show one row per
            // process, and kernel threads stay because they are real
            // processes. `threads` below reports the thread count instead.
            // On other platforms `thread_kind()` returns `None`, so this is a
            // no-op.
            .filter(|(_, process)| process.thread_kind() != Some(ThreadKind::Userland))
            .map(|(pid, process)| {
                let raw_name = process.name().to_string_lossy();
                let name = sanitize_display(&raw_name, MAX_NAME_CHARS);
                let raw_command = process
                    .cmd()
                    .iter()
                    .map(|part| part.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" ");
                let command = if raw_command.trim().is_empty() {
                    // Kernel threads have no argv; showing the name is more
                    // useful than an empty command column.
                    sanitize_display(&raw_name, MAX_COMMAND_CHARS)
                } else {
                    sanitize_display(&raw_command, MAX_COMMAND_CHARS)
                };
                let user = process
                    .user_id()
                    .and_then(|uid| self.users.as_ref()?.get_user_by_id(uid))
                    .map(|user| sanitize_display(user.name(), MAX_NAME_CHARS))
                    .filter(|user| !user.is_empty());
                let key = ProcessKey::new(pid.as_u32(), process.start_time());
                let cpu_ms = process.accumulated_cpu_time();
                let cpu_percent = match self.previous_cpu_ms.get(&key) {
                    Some(previous) if elapsed_ms > 0 => {
                        let delta = cpu_ms.saturating_sub(*previous);
                        (delta as f64 / elapsed_ms as f64 * 100.0).max(0.0) as f32
                    }
                    _ => 0.0,
                };
                next_cpu_ms.insert(key, cpu_ms);
                ProcessSnapshot {
                    key,
                    name,
                    command,
                    user,
                    status: map_status(process.status()),
                    cpu_percent,
                    memory_bytes: process.memory(),
                    virtual_memory_bytes: process.virtual_memory(),
                    // `sysinfo` excludes the process's own (main) thread from
                    // `tasks()`, so a single-threaded process reports one
                    // thread rather than zero. Only Linux collects tasks, so
                    // other platforms report `None` (shown as "—").
                    threads: thread_count(process),
                    parent_pid: process.parent().map(Pid::as_u32),
                    swap_bytes: process_swap_bytes(pid.as_u32()),
                }
            })
            .collect::<Vec<_>>();
        // `sysinfo` stores processes in a `HashMap`; sorting by PID keeps the
        // wire format and the UI table deterministic between samples.
        processes.sort_by_key(|process| (process.key.pid, process.key.start_time_secs));
        (processes, next_cpu_ms)
    }

    fn network_snapshot(&self) -> NetworkSnapshot {
        let mut interfaces = self
            .networks
            .as_ref()
            .map(|networks| {
                networks
                    .iter()
                    .filter_map(|(name, data)| {
                        let name = sanitize_display(&name.to_string_lossy(), MAX_NAME_CHARS);
                        // Loopback/virtual traffic duplicates counters already
                        // seen on physical NICs. Use the shared pure predicate
                        // so macOS "lo0" and WireGuard tunnels are excluded too.
                        if is_virtual_interface(&name) {
                            return None;
                        }
                        Some(NetworkInterfaceSnapshot {
                            name,
                            received_bytes: data.total_received(),
                            transmitted_bytes: data.total_transmitted(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        interfaces.sort_by(|left, right| left.name.cmp(&right.name));
        NetworkSnapshot { interfaces }
    }

    fn disk_snapshots(&self) -> Vec<DiskUsageSnapshot> {
        let mut disks = self
            .disks
            .as_ref()
            .map(|disks| {
                disks
                    .iter()
                    .filter(|disk| disk.total_space() > 0)
                    .map(|disk| {
                        let total = disk.total_space();
                        let available_space = disk.available_space();
                        // R2X-D5B: sysinfo は f_bavail ベースの available と
                        // f_blocks ベースの total しか公開しないため、rustix の
                        // safe statvfs API で f_bfree を読み、予約ブロックを
                        // used に算入しない真の使用量を導出する（Unix のみ）。
                        let true_used = fbfree_used_bytes(disk.mount_point());
                        let (available, used) =
                            disk_usage_values(total, available_space, true_used);
                        DiskUsageSnapshot {
                            mount_point: sanitize_display(
                                &disk.mount_point().to_string_lossy(),
                                MAX_COMMAND_CHARS,
                            ),
                            total_bytes: total,
                            available_bytes: available,
                            used_bytes: used,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        sort_disks_for_display(&mut disks);
        disks
    }

    /// Reads the machine's battery and power state (R1-I10).
    ///
    /// The pinned `sysinfo` 0.40.3 has no battery API, so Linux reads sysfs
    /// directly with `std::fs` (read-only, no new dependency and no `unsafe`).
    /// On every other target the result is [`PowerSource::Unknown`] with no
    /// batteries: an honest "unavailable" instead of an invented AC/battery
    /// claim. A probe failure (missing `/sys/class/power_supply`, or an
    /// unreadable/unparsable `online` attribute on a mains supply) is likewise
    /// reported as [`PowerSource::Unknown`] unless a battery actually reports
    /// `Discharging`: a failed `online` read must never be mistaken for
    /// "mains offline" and therefore for battery operation (R2-S5).
    pub fn power_snapshot(&self) -> PowerSnapshot {
        platform_power_snapshot()
    }

    /// Collects the cgroup label of every currently sampled process (R1-I12).
    ///
    /// This reads `/proc/<pid>/cgroup` with `std::fs` and never calls
    /// `refresh_processes_specifics`, so it cannot insert Linux threads into
    /// `sysinfo`'s process map. The one-row-per-process and `threads` contracts
    /// therefore stay intact (see the module-level "Process list" section).
    /// Non-Linux targets return an empty vector: `sysinfo` 0.40.3 exposes only
    /// cgroup *limits*, not per-process membership, on those platforms.
    pub fn cgroup_memberships(&self) -> Vec<ProcessCgroup> {
        platform_cgroup_memberships(&self.system)
    }

    /// Groups the current cgroup memberships for read-only display (R1-I12).
    ///
    /// Deterministic ordering comes from [`group_by_cgroup`] in `tm-core`.
    pub fn cgroup_groups(&self) -> Vec<CgroupGroup> {
        group_by_cgroup(&self.cgroup_memberships())
    }

    /// Chooses the sampling interval for the current power state (R1-I11).
    ///
    /// The shell supplies the window visibility it observes; the policy itself
    /// is the pure [`recommended_sampling_plan`] function in `tm-core`. Samples
    /// embed the visible-window plan; a shell that observes a hidden window can
    /// ask for that plan explicitly here.
    pub fn sampling_plan(&self, visibility: WindowVisibility) -> SamplingPlan {
        recommended_sampling_plan(&self.power_snapshot(), visibility)
    }
}

/// Result of probing the mains `online` sysfs attribute (R2-S5).
///
/// [`AcOnlineProbe::Unavailable`] is deliberately distinct from
/// [`AcOnlineProbe::Offline`]: a failed read says nothing about the power
/// source, while a readable `0` is real evidence that mains power is absent.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AcOnlineProbe {
    /// At least one mains supply reported a readable value greater than zero.
    Online,
    /// Every readable mains supply reported `0`, and none failed to read.
    Offline,
    /// No usable reading: no supply, no readable value, or at least one failed
    /// read. Must not be treated as "mains offline".
    Unavailable,
}

/// Aggregates one `online` reading per enumerated mains supply (R2-S5).
///
/// A positive reading is conclusive; a failure downgrades an otherwise all-zero
/// result to [`AcOnlineProbe::Unavailable`] so a partial sysfs failure cannot be
/// read as battery operation.
#[cfg(any(target_os = "linux", test))]
fn ac_online_from_readings(readings: impl IntoIterator<Item = Option<u64>>) -> AcOnlineProbe {
    let mut positive = false;
    let mut zero = false;
    let mut unreadable = false;
    for reading in readings {
        match reading {
            Some(value) if value > 0 => positive = true,
            Some(_) => zero = true,
            None => unreadable = true,
        }
    }
    if positive {
        AcOnlineProbe::Online
    } else if zero && !unreadable {
        AcOnlineProbe::Offline
    } else {
        AcOnlineProbe::Unavailable
    }
}

/// Derives the aggregate power source from the mains probe and battery states.
///
/// A `Discharging` state is positive evidence of battery operation and is the
/// only battery signal used when a mains read failed. `Charging`, `Full`,
/// `NotCharging` and `Unknown` states never justify [`PowerSource::Battery`] on
/// their own: with an unavailable mains read and no discharging battery the
/// result is [`PowerSource::Unknown`] (R2-S5).
#[cfg(any(target_os = "linux", test))]
fn power_source_from_probes(ac: AcOnlineProbe, batteries: &[BatterySnapshot]) -> PowerSource {
    let discharging = batteries
        .iter()
        .any(|battery| battery.state == BatteryState::Discharging);
    if discharging {
        return PowerSource::Battery;
    }
    match ac {
        AcOnlineProbe::Online => PowerSource::Ac,
        AcOnlineProbe::Offline if !batteries.is_empty() => PowerSource::Battery,
        AcOnlineProbe::Offline | AcOnlineProbe::Unavailable => PowerSource::Unknown,
    }
}

/// Reads the power supplies exposed by the Linux sysfs (R1-I10).
#[cfg(target_os = "linux")]
fn platform_power_snapshot() -> PowerSnapshot {
    const POWER_SUPPLY: &str = "/sys/class/power_supply";
    let Ok(entries) = std::fs::read_dir(POWER_SUPPLY) else {
        return PowerSnapshot::default();
    };

    let mut batteries = Vec::new();
    let mut ac_readings = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = std::fs::read_to_string(path.join("type")) else {
            continue;
        };
        match kind.trim() {
            "Battery" => {
                // Every OS-provided string follows the existing redact
                // contract: sanitise at the probe boundary (R2X-D2).
                let name = sanitize_display(&entry.file_name().to_string_lossy(), MAX_NAME_CHARS);
                let capacity_percent = read_sysfs_u64(&path.join("capacity"))
                    .and_then(|value| u8::try_from(value).ok())
                    .map(|value| value.min(100));
                let state = std::fs::read_to_string(path.join("status"))
                    .map(|status| battery_state_from_status(&status))
                    .unwrap_or(BatteryState::Unknown);
                batteries.push(BatterySnapshot {
                    name,
                    capacity_percent,
                    state,
                });
            }
            "Mains" | "AC" => {
                // Keep read success and failure separate: `None` means
                // "indeterminate", not the numeric zero that used to be read
                // as "mains offline" and hence as battery operation (R2-S5).
                ac_readings.push(read_sysfs_u64(&path.join("online")));
            }
            _ => {}
        }
    }
    batteries.sort_unstable_by(|left, right| left.name.cmp(&right.name));

    let ac_online = ac_online_from_readings(ac_readings);
    let source = power_source_from_probes(ac_online, &batteries);
    PowerSnapshot { source, batteries }
}

/// Fallback for targets without a battery API in the pinned dependencies (R1-I10).
#[cfg(not(target_os = "linux"))]
fn platform_power_snapshot() -> PowerSnapshot {
    PowerSnapshot::default()
}

/// Parses a numeric sysfs attribute, returning `None` on any read/parse failure.
#[cfg(target_os = "linux")]
fn read_sysfs_u64(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

/// Parses the `VmSwap:` field of a `/proc/<pid>/status` text into bytes.
///
/// The kernel reports kB, so the parsed value is multiplied by 1 024. A missing
/// field, a non-numeric value or an unreadable entry yields `0`, the honest
/// "unavailable" figure. The multiplication saturates so a corrupt reading
/// cannot wrap into a plausible small number.
fn vm_swap_bytes_from_status(status: &str) -> u64 {
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmSwap:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|kb| kb.parse::<u64>().ok())
        .map_or(0, |kb| kb.saturating_mul(1_024))
}

/// Reads one process's swap usage in bytes from `/proc/<pid>/status` (VmSwap).
///
/// Missing files, permission errors, absent fields and parse failures all yield
/// `0`: an unavailable figure must never be reported as a real measurement.
#[cfg(target_os = "linux")]
fn process_swap_bytes(pid: u32) -> u64 {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return 0;
    };
    vm_swap_bytes_from_status(&status)
}

/// Non-Linux targets have no `/proc/<pid>/status` VmSwap field; report `0`.
#[cfg(not(target_os = "linux"))]
fn process_swap_bytes(_pid: u32) -> u64 {
    0
}

/// Maps a sysfs `status` string onto the domain battery state (R1-I10).
#[cfg(any(target_os = "linux", test))]
fn battery_state_from_status(status: &str) -> BatteryState {
    match status.trim().to_ascii_lowercase().as_str() {
        "charging" => BatteryState::Charging,
        "discharging" => BatteryState::Discharging,
        "full" => BatteryState::Full,
        "not charging" | "not_charging" => BatteryState::NotCharging,
        _ => BatteryState::Unknown,
    }
}

/// Reads per-process cgroup membership from `/proc` without refreshing sysinfo (R1-I12).
#[cfg(target_os = "linux")]
fn platform_cgroup_memberships(system: &System) -> Vec<ProcessCgroup> {
    let mut memberships = system
        .processes()
        .iter()
        // Defensive backstop: keep the one-row-per-process contract even if a
        // future sysinfo version starts exposing task IDs here again.
        .filter(|(_, process)| process.thread_kind() != Some(ThreadKind::Userland))
        .filter_map(|(pid, process)| {
            let raw = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
            let label = cgroup_label(proc_cgroup_path(&raw));
            if label.is_empty() {
                return None;
            }
            Some(ProcessCgroup {
                key: ProcessKey::new(pid.as_u32(), process.start_time()),
                label,
            })
        })
        .collect::<Vec<_>>();
    // `/proc` enumeration order is not stable; sort like the process list so
    // the wire format and the UI table stay deterministic between samples.
    memberships.sort_unstable_by_key(|membership| membership.key);
    memberships
}

/// Fallback for targets without a per-process cgroup membership API (R1-I12).
#[cfg(not(target_os = "linux"))]
fn platform_cgroup_memberships(_system: &System) -> Vec<ProcessCgroup> {
    Vec::new()
}

/// Extracts the cgroup path from one `/proc/<pid>/cgroup` line block (R1-I12).
///
/// A v2 entry (`0::/path`) is preferred; otherwise the first usable line wins.
/// The returned slice borrows from `raw`, so this is an allocation-free parse.
#[cfg(any(target_os = "linux", test))]
fn proc_cgroup_path(raw: &str) -> &str {
    let mut fallback = "";
    for line in raw.lines() {
        let Some((prefix, path)) = line.rsplit_once(':') else {
            continue;
        };
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        if prefix == "0:" {
            return path;
        }
        if fallback.is_empty() {
            fallback = path;
        }
    }
    fallback
}

/// Reads the true, f_bfree-based used byte count for `mount_point` (R2X-D5B).
///
/// `sysinfo` 0.40.3 only reads `statvfs.f_bavail` and `f_blocks`, so its
/// `available_space()` excludes filesystem-reserved blocks while its
/// `total_space()` includes them; deriving `used = total - available` therefore
/// counts reserved blocks as used and overstates utilisation. `rustix` exposes
/// a **safe** `statvfs` wrapper with the `f_bfree` field, already a direct
/// dependency (only the `fs` feature was added, no new crate and no `unsafe`),
/// so the real used value `(f_blocks - f_bfree) * f_frsize` is available.
///
/// Returns `None` when `statvfs` is unavailable (Windows has no POSIX statvfs)
/// or the syscall fails, so the caller falls back to the legacy derivation.
fn fbfree_used_bytes(mount_point: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        let stat = rustix::fs::statvfs(mount_point).ok()?;
        Some(used_from_statvfs_counters(
            stat.f_blocks,
            stat.f_bfree,
            stat.f_frsize,
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = mount_point;
        None
    }
}

/// Pure f_bfree used formula: `(f_blocks - f_bfree) * f_frsize` (R2X-D5B).
///
/// Both operations saturate: an inconsistent statvfs reading can never wrap.
#[cfg(unix)]
fn used_from_statvfs_counters(f_blocks: u64, f_bfree: u64, f_frsize: u64) -> u64 {
    f_blocks.saturating_sub(f_bfree).saturating_mul(f_frsize)
}

/// Derives the `(available, used)` byte pair for one filesystem (R2X-D5/D5B).
///
/// `total_space` and `available_space` come from sysinfo (`f_blocks * f_frsize`
/// and `f_bavail * f_frsize`); `available` is deliberately left on that
/// f_bavail basis. `true_used`, when present, is the f_bfree-based byte count
/// read via rustix's safe `statvfs` wrapper, which excludes reserved blocks.
///
/// Safety contract (preserved for redact/export/UI consumers):
/// - `available <= total` and `used <= total`, so `used_percent()` stays in
///   range and no arithmetic wraps.
/// - `available + used <= total`. With reserved blocks the strict equality
///   `available + used == total` no longer holds: the gap is exactly the
///   filesystem-reserved space, which is neither used nor available to
///   unprivileged users (`df` reports the same three distinct quantities).
///   Equality is retained for the fallback path and for filesystems without
///   reserved blocks.
///
/// `saturating_*` makes an anomalous `available > total` reading explicit: the
/// filesystem is then reported as entirely free (`total`, `0`) rather than
/// silently wrapping inside the subtraction.
fn disk_usage_values(total_space: u64, available_space: u64, true_used: Option<u64>) -> (u64, u64) {
    let available = available_space.min(total_space);
    let free_remainder = total_space.saturating_sub(available);
    let used = match true_used {
        // Clamp to the non-reserved remainder as well: a statvfs read taken a
        // moment apart from sysinfo's must never push `used` above that bound.
        Some(bytes) => bytes.min(free_remainder),
        None => free_remainder,
    };
    (available, used)
}

/// Orders disks for display: largest capacity first, then by mount point.
///
/// The summary cards only show the first few entries, so sorting by mount point
/// alone would hide a large data volume behind a small system partition such as
/// `/small-efi` (Runtime Proof: the tiny EFI partition sorted before
/// `/large-data` and the 440 GiB volume was never displayed).
fn sort_disks_for_display(disks: &mut [DiskUsageSnapshot]) {
    disks.sort_by(|left, right| {
        right
            .total_bytes
            .cmp(&left.total_bytes)
            .then_with(|| left.mount_point.cmp(&right.mount_point))
    });
}

/// Whether `pid` must never be signalled (R2X-P6).
///
/// PID 0 always denotes a special target (the caller's process group on Unix,
/// the System Idle pseudo-process on Windows) and the application must never
/// signal itself, so both rules apply everywhere. PID 1 is the init system only
/// on Unix; on Windows it is a normal process and must stay controllable.
/// `pid_one_is_init` carries that platform decision into this pure function so
/// both branches can be regression-tested on any host.
fn pid_is_protected(pid: u32, self_pid: u32, pid_one_is_init: bool) -> bool {
    pid == 0 || pid == self_pid || (pid_one_is_init && pid == 1)
}

/// Builds the capability description for a native probe (R2X-P8).
///
/// `supports_network` / `supports_disks` mirror whether the corresponding
/// collector could be created. A failed collector therefore reports both an
/// empty section and `false`, which makes "supported but empty" distinguishable
/// from "collection unavailable" for callers. Process-control capability — and
/// with it the per-signal contract in `tm-core` — is always available for a
/// native probe.
fn native_platform_info(os: String, supports_network: bool, supports_disks: bool) -> PlatformInfo {
    PlatformInfo {
        os,
        supports_process_control: true,
        supports_network,
        supports_disks,
        is_demo: false,
    }
}

impl SystemProbe for NativeProbe {
    fn platform_info(&self) -> PlatformInfo {
        // A collector that could not be created yields an empty section in the
        // snapshot. Report that as an unavailable capability as well, so a
        // caller can tell "supported but currently empty" from "no data at all"
        // instead of reading the two as identical (R2X-P8). This only touches
        // network/disk flags; process-control capability (including the
        // per-signal contract in tm-core) stays unchanged.
        native_platform_info(
            self.os_label.clone(),
            self.networks.is_some(),
            self.disks.is_some(),
        )
    }

    fn sample(&mut self) -> Result<SystemSnapshot, ProbeError> {
        self.sample_native()
    }

    fn control(&mut self, key: ProcessKey, signal: Signal) -> Result<(), ControlError> {
        // Safety rules first. `kill(0, ...)` targets the whole process group
        // and PID 1 is the init system on Unix; the app must never signal
        // either, and must never signal itself. PID 1 has no special meaning on
        // Windows, where it is an ordinary process, so that part of the rule is
        // Unix-only (R2X-P6). `cfg!(unix)` makes the decision explicit and
        // testable on every host.
        let pid = key.pid;
        if pid_is_protected(pid, std::process::id(), cfg!(unix)) {
            return Err(ControlError::Protected);
        }
        // A PID that cannot be represented as `pid_t` cannot exist. Windows
        // PIDs are `u32`, so only Unix needs this bound.
        #[cfg(unix)]
        if pid > i32::MAX as u32 {
            return Err(ControlError::NotFound);
        }

        let sysinfo_pid = Pid::from_u32(pid);
        // Re-read the process immediately before signalling. `Some(pid)` makes
        // sysinfo read the process directly and insert the entry when it is
        // not known yet, so this also works for processes sampled after the
        // caller's last snapshot.
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[sysinfo_pid]),
            // This is a targeted re-read for the start-time check, not a full
            // refresh. `true` would tell sysinfo to evict every process not in
            // the list from its cache, destroying the observed process map (and
            // with it the CPU/thread bookkeeping) as a side effect of a control
            // request. Keep the rest of the map intact.
            false,
            // `nothing()` enables tasks by default; the start-time check needs
            // no thread information, so avoid that extra work here too.
            ProcessRefreshKind::nothing().without_tasks(),
        );
        let Some(process) = self.system.process(sysinfo_pid) else {
            return Err(ControlError::NotFound);
        };
        if process.start_time() != key.start_time_secs {
            // The PID now belongs to a different process (or the start time
            // could not be read); refuse to touch it.
            return Err(ControlError::NotFound);
        }

        control_process(process, signal)
    }
}

/// The refresh policy used for the full process list.
///
/// `cmd` and `user` only need to be fetched once per process, while CPU and
/// memory change continuously. Tasks must be disabled explicitly: in sysinfo
/// 0.40 [`ProcessRefreshKind::nothing`] enables `tasks` by default, and
/// combining it with `ProcessesToUpdate::All` inserts every Linux thread into
/// the process map as a full process entry. Thread counts are collected
/// separately by `refresh_thread_counts`.
fn process_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .without_tasks()
        .with_cpu()
        .with_memory()
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_user(UpdateKind::OnlyIfNotSet)
}

/// Refreshes each process's task ID set without inserting thread entries.
///
/// In `ProcessesToUpdate::Some` mode sysinfo still reads `/proc/<pid>/task`
/// and stores the task IDs on the process (available via
/// [`sysinfo::Process::tasks`]), but deliberately clears the list of thread
/// entries it would otherwise add to the process map. Passing all live PIDs in
/// one batched call therefore preserves the thread counts while keeping the
/// process map at one row per process and shortening the refresh that holds
/// the probe's mutex (R4-A01).
#[cfg(target_os = "linux")]
fn refresh_thread_counts(system: &mut System) {
    let pids = system
        .processes()
        .iter()
        .filter(|(_, process)| process.thread_kind() != Some(ThreadKind::Userland))
        .map(|(pid, _)| *pid)
        .collect::<Vec<_>>();
    if pids.is_empty() {
        return;
    }
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        // Drop PIDs that exited between the two refresh passes; otherwise a
        // stale entry with `tasks = None` reaches `build_snapshot` and the UI
        // reports no thread count (R49 flaky-test root cause).
        true,
        ProcessRefreshKind::nothing().with_tasks(),
    );
}

/// Number of threads in `process`, or `None` when the platform cannot report
/// it at all.
///
/// `sysinfo` only fills the task list on Linux; macOS, the BSDs and Windows
/// have no equivalent API, so the UI shows "—" there. On Linux an unreadable
/// `/proc` entry yields `Some(1)` (the main thread), never `None`.
#[cfg(target_os = "linux")]
fn thread_count(process: &sysinfo::Process) -> Option<u32> {
    // `sysinfo` excludes the process's own (main) thread from `tasks()`, so a
    // single-threaded process reports one thread rather than zero.
    // A process always has at least its main thread. Falling back to 1 keeps
    // the snapshot total even if sysinfo could not read /proc this tick.
    Some(process.tasks().map_or(1, |tasks| {
        tasks.len().saturating_add(1).min(u32::MAX as usize) as u32
    }))
}

/// Number of threads in `process`, or `None` when the platform cannot report
/// it.
#[cfg(not(target_os = "linux"))]
fn thread_count(_process: &sysinfo::Process) -> Option<u32> {
    None
}

/// Sends `signal` to `process` with the platform's native mechanism.
///
/// The caller has already validated the PID and re-checked the start time.
#[cfg(unix)]
fn control_process(process: &sysinfo::Process, signal: Signal) -> Result<(), ControlError> {
    let pid = process.pid().as_u32();
    // `from_raw` returns `None` for a PID that does not fit in `pid_t`; such a
    // process cannot exist.
    let raw_pid = RustixPid::from_raw(pid as i32).ok_or(ControlError::NotFound)?;
    match kill_process(raw_pid, map_signal(signal)) {
        Ok(()) => Ok(()),
        Err(err) if err == Errno::PERM => Err(ControlError::PermissionDenied),
        Err(err) if err == Errno::SRCH => Err(ControlError::NotFound),
        // OS error text is untrusted display data too: sanitise it so no control
        // character can reach the status bar through `ControlError`'s `Display`.
        Err(err) => Err(ControlError::Failed(format!(
            "kill: {}",
            sanitize_display(&err.to_string(), MAX_NAME_CHARS)
        ))),
    }
}

/// Sends `signal` to `process` on Windows.
///
/// Windows has no POSIX signals. `Terminate` and `Kill` are both implemented
/// with `taskkill /F` (forceful termination); there is no graceful equivalent.
/// `Stop` / `Continue` have no Windows equivalent and are reported as
/// unsupported.
///
/// `taskkill.exe` is invoked through its absolute path under
/// `%SystemRoot%\System32` instead of relying on `PATH` resolution: a hijacked
/// `PATH` must not be able to substitute an attacker-controlled executable.
/// (sysinfo's `Process::kill_with` uses `Command::new("taskkill.exe")`, which
/// resolves through `PATH`, so it is not used here.)
#[cfg(windows)]
fn control_process(process: &sysinfo::Process, signal: Signal) -> Result<(), ControlError> {
    let pid = process.pid().as_u32();
    match signal {
        Signal::Terminate | Signal::Kill => {}
        Signal::Stop | Signal::Continue => return Err(ControlError::Unsupported),
    }

    let mut command = std::process::Command::new(taskkill_path());
    command.arg("/PID").arg(pid.to_string()).arg("/F");
    // The task manager is a GUI application; do not flash a console window for
    // the helper process.
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command.output().map_err(|err| {
        ControlError::Failed(format!(
            "taskkill の実行に失敗: {}",
            sanitize_display(&err.to_string(), MAX_NAME_CHARS)
        ))
    })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(classify_taskkill_failure(
            output.status.code(),
            &output.stdout,
            &output.stderr,
        ))
    }
}

/// Maps a failed `taskkill` invocation onto a [`ControlError`] (R2X-P5).
///
/// `taskkill` exits with code 1 for every failure, so the localised message is
/// the only discriminator. English and Japanese messages for the two common
/// cases are recognised; a process that no longer exists is [`NotFound`], an
/// access refusal is [`PermissionDenied`], and anything else stays [`Failed`].
///
/// The raw stdout/stderr bytes are consumed **only** for this known-text
/// mapping. They are never stored in [`Failed`]: `taskkill` output can contain
/// internal paths, account names, localised text and control characters, and
/// `ControlError::Failed` is rendered verbatim through `Display` into the
/// status bar. An unmapped failure therefore carries a fixed, sanitised message
/// plus the exit code only. Both streams are scanned because Windows versions
/// differ in which one carries the error.
///
/// [`NotFound`]: ControlError::NotFound
/// [`PermissionDenied`]: ControlError::PermissionDenied
/// [`Failed`]: ControlError::Failed
#[cfg(any(windows, test))]
fn classify_taskkill_failure(exit_code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> ControlError {
    let mut output = String::from_utf8_lossy(stdout).into_owned();
    output.push('\n');
    output.push_str(&String::from_utf8_lossy(stderr));
    let ascii_lower = output.to_ascii_lowercase();

    if ascii_lower.contains("access is denied") || output.contains("アクセスが拒否") {
        return ControlError::PermissionDenied;
    }
    if ascii_lower.contains("not found")
        || ascii_lower.contains("no running instance")
        || output.contains("が見つかりません")
    {
        return ControlError::NotFound;
    }

    // `output` has served its purpose as a classifier input; do not let any of
    // its bytes escape into the error that `Display` renders in the UI.
    let code = exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    ControlError::Failed(format!("taskkill が失敗しました (exit={code})"))
}

/// Absolute path of the Windows `taskkill.exe`.
///
/// Uses `%SystemRoot%\System32\taskkill.exe` and falls back to
/// `C:\Windows\System32\taskkill.exe` when the variable is unset or empty.
/// The absolute path avoids `PATH` hijacking.
#[cfg(windows)]
fn taskkill_path() -> std::path::PathBuf {
    let system_root = std::env::var_os("SystemRoot")
        .filter(|root| !root.is_empty())
        .unwrap_or_else(|| std::ffi::OsString::from(r"C:\Windows"));
    std::path::PathBuf::from(system_root)
        .join("System32")
        .join("taskkill.exe")
}

/// Fallback for platforms that are neither Unix nor Windows: the probe is
/// read-only.
#[cfg(not(any(unix, windows)))]
fn control_process(_process: &sysinfo::Process, _signal: Signal) -> Result<(), ControlError> {
    Err(ControlError::Unsupported)
}

/// Maps the `sysinfo` scheduler state onto the domain enum.
///
/// `sysinfo::ProcessStatus` declares every variant on every platform (the
/// platform documentation on each variant only says which states can actually
/// occur), so one exhaustive match works for all targets.
fn map_status(status: SysinfoProcessStatus) -> ProcessStatus {
    match status {
        SysinfoProcessStatus::Run => ProcessStatus::Running,
        SysinfoProcessStatus::Sleep => ProcessStatus::Sleeping,
        SysinfoProcessStatus::Idle => ProcessStatus::Idle,
        SysinfoProcessStatus::Stop | SysinfoProcessStatus::Suspended => ProcessStatus::Stopped,
        SysinfoProcessStatus::Zombie => ProcessStatus::Zombie,
        SysinfoProcessStatus::Tracing => ProcessStatus::Tracing,
        SysinfoProcessStatus::Dead
        | SysinfoProcessStatus::Wakekill
        | SysinfoProcessStatus::Waking
        | SysinfoProcessStatus::Parked
        | SysinfoProcessStatus::LockBlocked
        | SysinfoProcessStatus::UninterruptibleDiskSleep
        | SysinfoProcessStatus::Unknown(_) => ProcessStatus::Unknown,
    }
}

/// Maps the domain signal onto the POSIX signal used by `rustix`.
#[cfg(unix)]
fn map_signal(signal: Signal) -> RustixSignal {
    match signal {
        Signal::Terminate => RustixSignal::TERM,
        Signal::Kill => RustixSignal::KILL,
        Signal::Stop => RustixSignal::STOP,
        Signal::Continue => RustixSignal::CONT,
    }
}

/// Average non-zero core frequency, or `None` when the platform reports none.
fn average_frequency_mhz(cpus: &[sysinfo::Cpu]) -> Option<u64> {
    let mut sum = 0u64;
    let mut count = 0u64;
    for frequency in cpus.iter().map(sysinfo::Cpu::frequency).filter(|f| *f > 0) {
        sum = sum.saturating_add(frequency);
        count += 1;
    }
    // `checked_div` is `None` exactly when no core reported a frequency.
    sum.checked_div(count)
}

/// Wall-clock timestamp with millisecond precision.
fn now_millis() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    Timestamp::from_millis(millis)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::process::{Child, Command};
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn vmswap_status_parsing_is_best_effort() {
        // kB -> bytes, and every failure mode degrades to an honest zero.
        assert_eq!(
            vm_swap_bytes_from_status("Name:\tx\nVmSwap:\t   1234 kB\n"),
            1_234 * 1_024
        );
        assert_eq!(vm_swap_bytes_from_status("VmSwap:\t0 kB\n"), 0);
        assert_eq!(vm_swap_bytes_from_status("Name:\tx\n"), 0);
        assert_eq!(vm_swap_bytes_from_status("VmSwap:\tbad kB\n"), 0);
        assert_eq!(vm_swap_bytes_from_status("VmSwap:\t-1 kB\n"), 0);
        // Saturation keeps a corrupt huge reading from wrapping.
        assert_eq!(
            vm_swap_bytes_from_status("VmSwap:\t18446744073709551615 kB\n"),
            u64::MAX
        );
    }

    /// Spawns a `sleep 30` child that tests may signal, and guarantees cleanup.
    #[cfg(unix)]
    struct ChildGuard {
        child: Child,
    }

    #[cfg(unix)]
    impl ChildGuard {
        fn spawn() -> Self {
            let child = Command::new("sleep")
                .arg("30")
                .spawn()
                .expect("spawn sleep 30");
            Self { child }
        }

        fn pid(&self) -> u32 {
            self.child.id()
        }

        fn wait_for_exit(&mut self, timeout: Duration) -> bool {
            let deadline = std::time::Instant::now() + timeout;
            loop {
                match self.child.try_wait() {
                    Ok(Some(_)) => return true,
                    Ok(None) if std::time::Instant::now() < deadline => {
                        sleep(Duration::from_millis(25));
                    }
                    _ => return false,
                }
            }
        }
    }

    #[cfg(unix)]
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            // Kill fallback: a test that failed before SIGTERM must not leak a
            // 30 second process.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Samples until the spawned child is visible and returns its identity.
    #[cfg(unix)]
    fn wait_for_process(probe: &mut NativeProbe, pid: u32) -> ProcessKey {
        for _ in 0..100 {
            sleep(Duration::from_millis(25));
            let snapshot = probe.sample().expect("sample");
            if let Some(process) = snapshot.processes.iter().find(|p| p.key.pid == pid) {
                return process.key;
            }
        }
        panic!("child process {pid} did not show up in the snapshot");
    }

    #[test]
    fn sample_reports_a_plausible_machine() {
        let mut probe = NativeProbe::new().expect("native probe");
        let snapshot = probe.sample().expect("sample");

        assert!(
            !snapshot.processes.is_empty(),
            "process list must not be empty"
        );
        assert!(snapshot.cpu.core_count() > 0, "at least one logical core");
        assert!(
            snapshot.memory.total_bytes > 0,
            "total memory must be known"
        );
        assert!(snapshot.sampled_at.as_secs() > 0, "timestamp must be set");
        assert_eq!(snapshot.self_pid, Some(std::process::id()));
        assert!(
            snapshot
                .processes
                .iter()
                .any(|process| process.key.pid == std::process::id()),
            "the sampler must see itself"
        );
        assert!(snapshot.hostname.is_some(), "hostname should be readable");
        assert!(
            snapshot
                .processes
                .iter()
                .all(|process| process.name.chars().count() <= MAX_NAME_CHARS + 1),
            "process names are truncated"
        );
        assert!(
            snapshot
                .processes
                .iter()
                .all(|process| process.command.chars().count() <= MAX_COMMAND_CHARS + 1),
            "commands are truncated"
        );
        assert!(
            snapshot
                .processes
                .iter()
                .all(|process| process.cpu_percent == 0.0),
            "the first sample has no CPU reference yet"
        );
    }

    #[test]
    fn platform_info_describes_the_native_backend() {
        let probe = NativeProbe::new().expect("native probe");
        let info = probe.platform_info();
        assert!(info.supports_process_control);
        // R2X-P8: the flags follow what this probe could actually enumerate.
        assert_eq!(info.supports_network, probe.networks.is_some());
        assert_eq!(info.supports_disks, probe.disks.is_some());
        assert!(!info.is_demo);
        assert!(!info.os.is_empty());
    }

    #[test]
    fn userland_threads_are_not_listed_as_processes() {
        let mut probe = NativeProbe::new().expect("native probe");
        let snapshot = probe.sample().expect("sample");
        let listed = snapshot
            .processes
            .iter()
            .map(|process| process.key.pid)
            .collect::<std::collections::HashSet<_>>();

        for (pid, process) in probe.system.processes() {
            let Some(tasks) = process.tasks() else {
                continue;
            };
            for task in tasks {
                assert!(
                    !listed.contains(&task.as_u32()),
                    "thread {} of process {} must not be listed as a process",
                    task.as_u32(),
                    pid.as_u32()
                );
            }
        }
        assert!(
            snapshot
                .processes
                .iter()
                .all(|process| process.threads.is_none_or(|threads| threads >= 1)),
            "a listed process always has at least its own thread"
        );
    }

    /// R4-A01 regression: the full refresh must never insert Linux threads as
    /// process-map entries, and the one-row-per-process contract must survive
    /// repeated samples.
    #[cfg(target_os = "linux")]
    #[test]
    fn full_refresh_never_inserts_userland_threads() {
        let mut probe = NativeProbe::new().expect("native probe");
        for _ in 0..2 {
            let snapshot = probe.sample().expect("sample");
            let inserted = probe
                .system
                .processes()
                .iter()
                .filter(|(_, process)| process.thread_kind() == Some(ThreadKind::Userland))
                .map(|(pid, _)| pid.as_u32())
                .collect::<Vec<_>>();
            assert!(
                inserted.is_empty(),
                "sysinfo inserted thread entries into the process map: {inserted:?}"
            );
            let pids = snapshot
                .processes
                .iter()
                .map(|process| process.key.pid)
                .collect::<Vec<_>>();
            assert!(
                pids.windows(2).all(|pair| pair[0] <= pair[1]),
                "process list must stay sorted by PID"
            );
            let unique = pids.iter().collect::<std::collections::HashSet<_>>();
            assert_eq!(
                unique.len(),
                pids.len(),
                "process list must not contain duplicate PIDs"
            );
        }
    }

    /// R4-A01 regression: collecting tasks in a separate `Some` refresh must
    /// keep the `threads` column correct for every listed process.
    #[cfg(target_os = "linux")]
    #[test]
    fn split_task_refresh_preserves_thread_counts() {
        let mut probe = NativeProbe::new().expect("native probe");
        let snapshot = probe.sample().expect("sample");
        assert!(
            !snapshot.processes.is_empty(),
            "process list must not be empty"
        );

        // Processes can exit (or change their task set) between the sample and
        // this assertion under parallel test load. Assert the stable contract
        // instead: Linux reports a thread count for at least one process and
        // every reported count is >= 1. The exact split-refresh mapping is
        // covered by the deterministic fixture tests below.
        let mut reported = 0;
        for process in &snapshot.processes {
            if let Some(threads) = process.threads {
                assert!(
                    threads >= 1,
                    "a listed process always has its own thread (pid {})",
                    process.key.pid
                );
                reported += 1;
            }
        }
        assert!(
            reported > 0,
            "Linux must report thread counts for at least one process"
        );
    }

    /// R4-A01 regression: a single-threaded child still reports exactly one
    /// thread after the split refresh.
    #[cfg(unix)]
    #[test]
    fn single_threaded_child_reports_one_thread() {
        let mut probe = NativeProbe::new().expect("native probe");
        let child = ChildGuard::spawn();
        let key = wait_for_process(&mut probe, child.pid());
        let snapshot = probe.sample().expect("sample");
        let process = snapshot
            .processes
            .iter()
            .find(|process| process.key.pid == key.pid)
            .expect("child must be listed");
        #[cfg(target_os = "linux")]
        assert_eq!(process.threads, Some(1), "sleep has exactly one thread");
        #[cfg(not(target_os = "linux"))]
        assert_eq!(process.threads, None, "non-Linux cannot report threads");
    }

    /// The split refresh must keep sanitising OS-provided strings (the
    /// `--redact` boundary contract): no control characters, no unbounded text.
    #[test]
    fn display_strings_stay_sanitised_after_task_split() {
        let mut probe = NativeProbe::new().expect("native probe");
        let snapshot = probe.sample().expect("sample");
        for process in &snapshot.processes {
            for text in [&process.name, &process.command] {
                assert!(
                    !text.chars().any(char::is_control),
                    "control character leaked into a process display string: {text:?}"
                );
            }
            assert!(process.name.chars().count() <= MAX_NAME_CHARS + 1);
            assert!(process.command.chars().count() <= MAX_COMMAND_CHARS + 1);
            if let Some(user) = &process.user {
                assert!(!user.chars().any(char::is_control));
                assert!(user.chars().count() <= MAX_NAME_CHARS + 1);
            }
        }
        if let Some(hostname) = &snapshot.hostname {
            assert!(!hostname.chars().any(char::is_control));
            assert!(hostname.chars().count() <= MAX_NAME_CHARS + 1);
        }
    }

    #[test]
    fn two_samples_keep_cpu_percentages_in_range() {
        let mut probe = NativeProbe::new().expect("native probe");
        let first = probe.sample().expect("first sample");
        // sysinfo needs at least MINIMUM_CPU_UPDATE_INTERVAL between refreshes
        // for a meaningful second value.
        sleep(Duration::from_millis(250));
        let second = probe.sample().expect("second sample");

        for snapshot in [&first, &second] {
            assert!(
                (0.0..=100.0).contains(&snapshot.cpu.total_percent),
                "total CPU percent out of range: {}",
                snapshot.cpu.total_percent
            );
            assert!(
                snapshot
                    .cpu
                    .per_core_percent
                    .iter()
                    .all(|percent| (0.0..=100.0).contains(percent)),
                "per-core CPU percent out of range"
            );
            assert!(snapshot.memory.used_percent() <= 100.0);
        }
        assert_eq!(first.cpu.core_count(), second.cpu.core_count());
        assert!(
            second.cpu.uptime_secs >= first.cpu.uptime_secs,
            "uptime must not go backwards"
        );

        // Process CPU percentages are derived from accumulated CPU time, so
        // they are bounded by the number of cores and their sum by the machine
        // capacity. `sysinfo`'s own `Process::cpu_usage()` used to saturate at
        // `cores * 100` on this machine because a process refresh takes longer
        // than its minimum CPU interval; this guards against a regression.
        let capacity = second.cpu.core_count() as f32 * 100.0;
        let mut process_sum = 0.0f32;
        for process in &second.processes {
            assert!(
                process.cpu_percent.is_finite() && process.cpu_percent >= 0.0,
                "process CPU percent must be finite and non-negative: {}",
                process.cpu_percent
            );
            assert!(
                process.cpu_percent <= capacity,
                "process CPU percent {} exceeds machine capacity {capacity}",
                process.cpu_percent
            );
            process_sum += process.cpu_percent;
        }
        assert!(
            process_sum <= capacity * 2.0,
            "sum of process CPU percentages {process_sum} is implausible for {capacity}% capacity"
        );
    }

    #[test]
    fn control_protects_pid_zero_and_self() {
        let mut probe = NativeProbe::new().expect("native probe");
        for pid in [0, std::process::id()] {
            assert_eq!(
                probe.control(ProcessKey::new(pid, 0), Signal::Terminate),
                Err(ControlError::Protected),
                "pid {pid} must be protected"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn control_protects_pid_one_on_unix() {
        let mut probe = NativeProbe::new().expect("native probe");
        // PID 1 is init on Unix and must never be signalled. On Windows it is
        // an ordinary process (`R2X-P6`); the flag-table test covers that case.
        assert_eq!(
            probe.control(ProcessKey::new(1, 0), Signal::Terminate),
            Err(ControlError::Protected)
        );
    }

    #[cfg(unix)]
    #[test]
    fn control_reports_not_found_for_a_reaped_process() {
        let mut probe = NativeProbe::new().expect("native probe");
        // Spawn and reap a child, then use its old PID with a start time that
        // cannot belong to it. Even if the kernel recycled the PID in between,
        // the start time mismatch must yield NotFound.
        let mut child = Command::new("true").spawn().expect("spawn true");
        let pid = child.id();
        child.wait().expect("reap true");
        assert_eq!(
            probe.control(ProcessKey::new(pid, 0), Signal::Terminate),
            Err(ControlError::NotFound)
        );
    }

    #[test]
    fn control_reports_not_found_for_out_of_range_pid() {
        let mut probe = NativeProbe::new().expect("native probe");
        assert_eq!(
            probe.control(ProcessKey::new(u32::MAX, 0), Signal::Terminate),
            Err(ControlError::NotFound)
        );
    }

    #[cfg(unix)]
    #[test]
    fn control_rejects_a_stale_start_time() {
        let mut probe = NativeProbe::new().expect("native probe");
        let mut child = ChildGuard::spawn();
        let key = wait_for_process(&mut probe, child.pid());
        let stale = ProcessKey::new(key.pid, key.start_time_secs + 1);

        assert_eq!(
            probe.control(stale, Signal::Terminate),
            Err(ControlError::NotFound),
            "a mismatched start time must never be signalled"
        );
        assert!(
            child.child.try_wait().expect("try_wait").is_none(),
            "the child must still be running after a rejected signal"
        );
    }

    /// Regression: the start-time re-check refreshes a single PID with
    /// `ProcessesToUpdate::Some(&[pid])`. Its `remove_not_listed_processes`
    /// argument must be `false`, otherwise sysinfo evicts every other process
    /// from its cache and a control request destroys the observed process map.
    #[test]
    fn control_refresh_preserves_the_process_map() {
        let mut probe = NativeProbe::new().expect("native probe");
        let snapshot = probe.sample().expect("sample");
        let self_pid = std::process::id();
        let target = snapshot
            .processes
            .iter()
            .find(|process| !pid_is_protected(process.key.pid, self_pid, cfg!(unix)))
            .expect("the machine must have a controllable process besides the sampler");
        // A stale start time reaches the re-check refresh but delivers no signal.
        let stale = ProcessKey::new(target.key.pid, target.key.start_time_secs.wrapping_add(1));
        assert_eq!(
            probe.control(stale, Signal::Terminate),
            Err(ControlError::NotFound),
            "the stale key must be rejected"
        );

        // With `true` above, only `target.key.pid` would remain in the map.
        assert!(
            probe.system.process(Pid::from_u32(self_pid)).is_some(),
            "control() evicted the sampler itself from the process map"
        );
        let remaining = probe.system.processes().len();
        assert!(
            remaining > 1,
            "control() collapsed the process map to {remaining} entry/entries"
        );
    }

    #[cfg(unix)]
    #[test]
    fn terminate_ends_a_child_process() {
        let mut probe = NativeProbe::new().expect("native probe");
        let mut child = ChildGuard::spawn();
        let key = wait_for_process(&mut probe, child.pid());

        probe
            .control(key, Signal::Terminate)
            .expect("SIGTERM to own child");
        assert!(
            child.wait_for_exit(Duration::from_secs(5)),
            "child did not exit after SIGTERM"
        );
    }
    #[test]
    fn disks_are_ordered_by_capacity_for_the_summary_card() {
        let disk = |mount_point: &str, total_bytes: u64| DiskUsageSnapshot {
            mount_point: mount_point.to_string(),
            total_bytes,
            available_bytes: total_bytes / 2,
            used_bytes: total_bytes - total_bytes / 2,
        };
        // A tiny EFI partition would otherwise sort first (before the large
        // data volume) and hide it from the summary cards.
        let mut disks = vec![
            disk("/small-efi", 96 * 1_024 * 1_024),
            disk("/", 600 * 1_024 * 1_024 * 1_024),
            disk("/large-data", 440 * 1_024 * 1_024 * 1_024),
        ];
        sort_disks_for_display(&mut disks);
        let order: Vec<&str> = disks.iter().map(|d| d.mount_point.as_str()).collect();
        assert_eq!(order, vec!["/", "/large-data", "/small-efi"]);
    }

    #[test]
    fn disks_with_equal_capacity_are_ordered_by_mount_point() {
        let disk = |mount_point: &str| DiskUsageSnapshot {
            mount_point: mount_point.to_string(),
            total_bytes: 1_024,
            available_bytes: 512,
            used_bytes: 512,
        };
        let mut disks = vec![disk("/z"), disk("/a"), disk("/m")];
        sort_disks_for_display(&mut disks);
        let order: Vec<&str> = disks.iter().map(|d| d.mount_point.as_str()).collect();
        assert_eq!(order, vec!["/a", "/m", "/z"]);
    }

    /// R2X-D5 regression: the fallback path (no f_bfree reading) must keep the
    /// equivalent of `Disk::used_space()`, derived from `total` and `available`
    /// without wrapping, even at the u64 boundaries.
    #[test]
    fn disk_usage_values_match_the_used_space_equivalent() {
        let total = 1_099_511_627_776u64; // 1 TiB
        let available = 429_496_729_600u64; // 400 GiB
        assert_eq!(
            disk_usage_values(total, available, None),
            (available, total - available)
        );

        assert_eq!(disk_usage_values(1_024, 0, None), (0, 1_024));
        assert_eq!(disk_usage_values(1_024, 1_024, None), (1_024, 0));
        assert_eq!(disk_usage_values(0, 0, None), (0, 0));
        assert_eq!(disk_usage_values(u64::MAX, 0, None), (0, u64::MAX));
        assert_eq!(disk_usage_values(u64::MAX, u64::MAX, None), (u64::MAX, 0));
    }

    /// R2X-D5 regression: an inconsistent collector reading must be clamped
    /// instead of wrapping. This preserves the pre-existing `min` +
    /// `saturating_sub` outcome while making the check explicit.
    #[test]
    fn disk_usage_values_clamp_available_above_total() {
        assert_eq!(disk_usage_values(1_024, 1_025, None), (1_024, 0));
        assert_eq!(disk_usage_values(100, u64::MAX, None), (100, 0));
        assert_eq!(disk_usage_values(0, u64::MAX, None), (0, 0));
    }

    /// R2X-D5 regression: on the fallback path the three snapshot fields keep
    /// the strict `available + used == total` equality for every input, so
    /// `used_percent()` can rely on `used <= total`.
    #[test]
    fn disk_usage_values_keep_the_total_contract() {
        for (total, reported) in [
            (0, 0),
            (1, 0),
            (1, 1),
            (1_000, 999),
            (1_000, 1_001),
            (u64::MAX, u64::MAX - 1),
            (u64::MAX, u64::MAX),
        ] {
            let (available, used) = disk_usage_values(total, reported, None);
            assert!(
                available <= total,
                "available above total for {total}/{reported}"
            );
            assert!(used <= total, "used above total for {total}/{reported}");
            assert_eq!(
                available.checked_add(used),
                Some(total),
                "available + used must equal total for {total}/{reported}"
            );
        }
    }

    /// R2X-D5B regression: the pure formula uses `f_bfree`, not `f_bavail`, so
    /// filesystem-reserved blocks are excluded from the `used` value.
    #[cfg(unix)]
    #[test]
    fn fbfree_used_formula_excludes_reserved_blocks() {
        // f_blocks = 1000, f_bfree = 800, f_bavail = 600, f_frsize = 4096.
        // 200 blocks are reserved: true used is 200 * 4096, not 400 * 4096.
        let (f_blocks, f_bfree, f_bavail, frsize) = (1_000u64, 800u64, 600u64, 4_096u64);
        let true_used = used_from_statvfs_counters(f_blocks, f_bfree, frsize);
        assert_eq!(true_used, (f_blocks - f_bfree) * frsize);
        assert_eq!(true_used, 819_200);
        assert!(true_used < (f_blocks - f_bavail) * frsize);

        // Saturating arithmetic at the boundaries: no wrap.
        assert_eq!(used_from_statvfs_counters(0, 1, u64::MAX), 0);
        assert_eq!(used_from_statvfs_counters(u64::MAX, 0, u64::MAX), u64::MAX);
    }

    /// R2X-D5B regression: a true f_bfree reading wins over the legacy
    /// `total - available` derivation, while `available` stays f_bavail-based.
    #[test]
    fn disk_usage_values_prefers_fbfree_used() {
        let total = 1_000u64;
        let available = 600u64; // f_bavail * f_frsize
        let true_used = 200u64; // (f_blocks - f_bfree) * f_frsize
        assert_eq!(
            disk_usage_values(total, available, Some(true_used)),
            (available, true_used)
        );
        // The reserved-block gap is neither used nor available (as in `df`).
        assert!(available + true_used < total);
        // No f_bfree reading (Windows / statvfs failure) keeps the old result.
        assert_eq!(
            disk_usage_values(total, available, None),
            (available, total - available)
        );
    }

    /// R2X-D5B regression: an inconsistent statvfs read must never push used
    /// above the non-reserved remainder, even at the u64 boundaries.
    #[test]
    fn disk_usage_values_clamps_fbfree_above_the_remainder() {
        assert_eq!(disk_usage_values(1_000, 600, Some(1_000)), (600, 400));
        assert_eq!(disk_usage_values(1_000, 600, Some(u64::MAX)), (600, 400));
        assert_eq!(disk_usage_values(1_000, 1_001, Some(999)), (1_000, 0));
        assert_eq!(
            disk_usage_values(u64::MAX, u64::MAX, Some(u64::MAX)),
            (u64::MAX, 0)
        );
    }

    /// R2X-D5B regression: every input keeps the safety invariants that
    /// `used_percent()`, redact and export rely on.
    #[test]
    fn disk_usage_values_fbfree_keeps_safety_bounds() {
        for (total, available, true_used) in [
            (0, 0, Some(0)),
            (1, 0, Some(1)),
            (1_000, 999, Some(1)),
            (1_000, 1_001, Some(u64::MAX)),
            (u64::MAX, 0, Some(u64::MAX)),
            (u64::MAX, u64::MAX - 1, Some(1)),
        ] {
            let (reported_available, used) = disk_usage_values(total, available, true_used);
            assert!(
                reported_available <= total,
                "available above total for {total}/{available}/{true_used:?}"
            );
            assert!(
                used <= total,
                "used above total for {total}/{available}/{true_used:?}"
            );
            assert!(
                reported_available
                    .checked_add(used)
                    .is_some_and(|sum| sum <= total),
                "available + used exceeded total for {total}/{available}/{true_used:?}"
            );
        }
    }

    /// R2X-D5B Runtime Proof: rustix's safe statvfs exposes `f_bfree`, and the
    /// helper derives used from it. Reserved blocks (when present) are neither
    /// used nor available, so the gap below `total - available` is observable.
    #[cfg(unix)]
    #[test]
    fn statvfs_derived_used_excludes_reserved_blocks_on_the_host() {
        let stat = rustix::fs::statvfs(Path::new("/")).expect("statvfs /");
        let derived = used_from_statvfs_counters(stat.f_blocks, stat.f_bfree, stat.f_frsize);
        let total = stat.f_blocks.saturating_mul(stat.f_frsize);
        let available = stat.f_bavail.saturating_mul(stat.f_frsize);
        assert!(derived <= total);
        assert!(derived <= total.saturating_sub(available));
        if stat.f_bfree > stat.f_bavail {
            assert!(
                derived < total.saturating_sub(available),
                "reserved blocks must not be counted as used"
            );
        }
        assert_eq!(fbfree_used_bytes(Path::new("/")), Some(derived));
    }

    /// R2X-D5B regression: the derived values must survive the real sampling
    /// path. With reserved blocks the strict equality becomes an inequality:
    /// `available + used <= total`, with the gap being reserved space.
    #[test]
    fn sampled_disks_keep_the_safe_available_used_contract() {
        let mut probe = NativeProbe::new().expect("native probe");
        let snapshot = probe.sample().expect("sample");
        assert!(
            !snapshot.disks.is_empty(),
            "the host must expose at least one disk"
        );
        for disk in &snapshot.disks {
            assert!(
                disk.available_bytes <= disk.total_bytes,
                "{} reported available above total",
                disk.mount_point
            );
            assert!(
                disk.used_bytes <= disk.total_bytes - disk.available_bytes,
                "{} counted reserved blocks as used",
                disk.mount_point
            );
            assert!(
                disk.used_percent() <= 100.0,
                "{} reported an out-of-range utilisation",
                disk.mount_point
            );
        }
    }

    /// R2X-P6 regression: PID 1 is init only on Unix, never on Windows.
    #[test]
    fn pid_one_protection_is_platform_specific() {
        const SELF: u32 = 4242;
        // Unix: PID 1 is the init system and must stay protected.
        assert!(pid_is_protected(1, SELF, true));
        // Windows: PID 1 is an ordinary process and must be controllable.
        assert!(!pid_is_protected(1, SELF, false));
        // Universal rules must not depend on the PID 1 decision.
        for pid_one_is_init in [true, false] {
            assert!(pid_is_protected(0, SELF, pid_one_is_init), "PID 0");
            assert!(pid_is_protected(SELF, SELF, pid_one_is_init), "self");
            assert!(!pid_is_protected(2, SELF, pid_one_is_init), "ordinary PID");
        }
    }

    /// R2X-P8 regression: capability flags must describe actual enumeration,
    /// while process control and the demo flag stay fixed.
    #[test]
    fn native_capability_flags_distinguish_empty_from_unavailable() {
        for (network, disks) in [(true, true), (true, false), (false, true), (false, false)] {
            let info = native_platform_info("Linux".to_string(), network, disks);
            assert_eq!(info.supports_network, network);
            assert_eq!(info.supports_disks, disks);
            assert!(info.supports_process_control);
            assert!(!info.is_demo);
            // Signal capability is owned by tm-core and must be unaffected by
            // a collector failure (existing contract).
            assert!(info.supports_signal(Signal::Terminate));
            assert!(info.supports_signal(Signal::Stop));
        }
    }

    /// R2X-P5 regression: taskkill's single exit code is not enough; the
    /// localised message must select the domain error.
    #[test]
    fn taskkill_access_denied_maps_to_permission_denied() {
        let english =
            b"ERROR: The process with PID 1234 could not be terminated.\nReason: Access is denied.";
        let japanese =
            "エラー: PID が 1234 のプロセスを終了できません。\n理由: アクセスが拒否されました。"
                .as_bytes();
        for (stdout, stderr) in [
            (b"".as_slice(), english.as_slice()),
            (japanese, b"".as_slice()),
        ] {
            assert_eq!(
                classify_taskkill_failure(Some(1), stdout, stderr),
                ControlError::PermissionDenied
            );
        }
    }

    #[test]
    fn taskkill_not_found_maps_to_not_found() {
        let english = b"ERROR: The process \"1234\" not found.";
        let japanese = "エラー: プロセス \"1234\" が見つかりません。".as_bytes();
        for (stdout, stderr) in [
            (b"".as_slice(), english.as_slice()),
            (japanese, b"".as_slice()),
        ] {
            assert_eq!(
                classify_taskkill_failure(Some(1), stdout, stderr),
                ControlError::NotFound
            );
        }
    }

    /// R2X-P5 regression: raw `taskkill` output must never reach `Display`.
    #[test]
    fn taskkill_unknown_failure_never_leaks_raw_output() {
        let error = classify_taskkill_failure(Some(1), b"", b"ERROR: Invalid argument.");
        assert!(
            matches!(error, ControlError::Failed(ref message) if message.contains("exit=1") && !message.contains("Invalid argument")),
            "unmapped failures carry only fixed text and the exit code: {error:?}"
        );
        // Preserve the exit code even when no message was written.
        let silent = classify_taskkill_failure(Some(5), b"", b"");
        assert!(
            matches!(silent, ControlError::Failed(ref message) if message.contains("exit=5")),
            "the exit code must survive: {silent:?}"
        );

        // An adversarial taskkill message must not smuggle an internal path,
        // an account name or control characters into the status bar through
        // `ControlError`'s `Display`.
        let hostile = b"ERROR: C:\\Users\\alice\\secret could not be terminated for DOMAIN\\bob.\n\x1b[31mRED\x1b[0m";
        let leaked = classify_taskkill_failure(Some(1), hostile, b"");
        let rendered = leaked.to_string();
        assert!(
            !rendered.contains("alice"),
            "internal path leaked: {rendered:?}"
        );
        assert!(
            !rendered.contains("DOMAIN"),
            "account name leaked: {rendered:?}"
        );
        assert!(
            !rendered.chars().any(char::is_control),
            "control character leaked: {rendered:?}"
        );
    }

    /// R2X-P7 regression: refreshing the user list must not drop the uid ->
    /// name mapping used by the user column across repeated samples.
    #[test]
    fn user_column_stays_resolved_across_samples() {
        let mut probe = NativeProbe::new().expect("native probe");
        for _ in 0..2 {
            let snapshot = probe.sample().expect("sample");
            if probe.users.is_none() {
                continue; // no collector on this host; nothing to check
            }
            let own = snapshot
                .processes
                .iter()
                .find(|process| process.key.pid == std::process::id())
                .expect("the sampler must see itself");
            if probe
                .system
                .process(Pid::from_u32(own.key.pid))
                .and_then(|process| process.user_id())
                .is_some()
            {
                assert!(
                    own.user.as_ref().is_some_and(|user| !user.is_empty()),
                    "the user column must survive Users::refresh()"
                );
            }
        }
    }

    // --- R1-I10 / R1-I11 / R1-I12 --------------------------------------------

    #[test]
    fn sysfs_battery_status_maps_to_domain_states() {
        for (raw, expected) in [
            ("Charging\n", BatteryState::Charging),
            ("DISCHARGING", BatteryState::Discharging),
            ("Full", BatteryState::Full),
            ("Not charging", BatteryState::NotCharging),
            ("not_charging", BatteryState::NotCharging),
            ("Unknown", BatteryState::Unknown),
            ("", BatteryState::Unknown),
        ] {
            assert_eq!(battery_state_from_status(raw), expected, "{raw:?}");
        }
    }

    fn battery(name: &str, state: BatteryState) -> BatterySnapshot {
        BatterySnapshot {
            name: name.to_string(),
            capacity_percent: Some(80),
            state,
        }
    }

    #[test]
    fn ac_online_read_failure_is_distinguished_from_a_zero_reading() {
        assert_eq!(
            ac_online_from_readings([Some(1), None]),
            AcOnlineProbe::Online
        );
        assert_eq!(
            ac_online_from_readings([Some(0), None]),
            AcOnlineProbe::Unavailable
        );
        assert_eq!(
            ac_online_from_readings([None, Some(0)]),
            AcOnlineProbe::Unavailable
        );
        assert_eq!(ac_online_from_readings([Some(0)]), AcOnlineProbe::Offline);
        assert_eq!(ac_online_from_readings([]), AcOnlineProbe::Unavailable);
    }

    #[test]
    fn unreadable_ac_online_never_claims_battery_operation() {
        let batteries = vec![
            battery("BAT0", BatteryState::Charging),
            battery("BAT1", BatteryState::Full),
            battery("BAT2", BatteryState::NotCharging),
            battery("BAT3", BatteryState::Unknown),
        ];
        let source = power_source_from_probes(AcOnlineProbe::Unavailable, &batteries);
        assert_eq!(source, PowerSource::Unknown);
        let snapshot = PowerSnapshot { source, batteries };
        assert!(!snapshot.on_battery());
    }

    #[test]
    fn unreadable_ac_online_with_discharging_battery_reports_battery() {
        let batteries = vec![
            battery("BAT0", BatteryState::Charging),
            battery("BAT1", BatteryState::Discharging),
        ];
        let source = power_source_from_probes(AcOnlineProbe::Unavailable, &batteries);
        assert_eq!(source, PowerSource::Battery);
        let snapshot = PowerSnapshot { source, batteries };
        assert!(snapshot.on_battery());
    }

    #[test]
    fn readable_ac_online_values_keep_the_existing_source_contract() {
        let batteries = || vec![battery("BAT0", BatteryState::Charging)];
        assert_eq!(
            power_source_from_probes(AcOnlineProbe::Online, &batteries()),
            PowerSource::Ac
        );
        assert_eq!(
            power_source_from_probes(AcOnlineProbe::Offline, &batteries()),
            PowerSource::Battery
        );
        assert_eq!(
            power_source_from_probes(AcOnlineProbe::Offline, &[]),
            PowerSource::Unknown
        );
        assert_eq!(
            power_source_from_probes(AcOnlineProbe::Unavailable, &[]),
            PowerSource::Unknown
        );
    }

    #[test]
    fn proc_cgroup_parser_prefers_the_v2_hierarchy() {
        let raw = concat!(
            "1:name=systemd:/system.slice/ssh.service\n",
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/worker.service\n",
            "2:memory:/system.slice/ssh.service\n",
        );
        assert_eq!(
            proc_cgroup_path(raw),
            "/user.slice/user-1000.slice/user@1000.service/app.slice/worker.service"
        );
        // Without a v2 entry the first usable line is the fallback; a v2 line
        // with the root path is still a valid (and preferred) entry.
        let legacy = "1:name=systemd:/system.slice/ssh.service\n";
        assert_eq!(proc_cgroup_path(legacy), "/system.slice/ssh.service");
        assert_eq!(proc_cgroup_path("1:name=systemd:/a\n0::/\n"), "/");
        assert_eq!(proc_cgroup_path("garbage\n0::\n"), "");
    }

    #[test]
    fn sampled_power_state_keeps_the_display_and_consistency_contract() {
        let probe = NativeProbe::new().expect("native probe");
        let power = probe.power_snapshot();
        for battery in &power.batteries {
            assert!(
                !battery.name.chars().any(char::is_control),
                "control character in battery name: {:?}",
                battery.name
            );
            assert!(battery.name.chars().count() <= MAX_NAME_CHARS + 1);
            assert!(
                battery
                    .capacity_percent
                    .is_none_or(|percent| percent <= 100),
                "capacity must stay in 0..=100"
            );
        }
        let names = power
            .batteries
            .iter()
            .map(|battery| battery.name.as_str())
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "batteries must be sorted by name");

        assert_eq!(
            power.on_battery(),
            power.source == PowerSource::Battery
                || power
                    .batteries
                    .iter()
                    .any(|battery| battery.state == BatteryState::Discharging)
        );
        if power.batteries.is_empty() {
            assert_ne!(power.source, PowerSource::Battery);
        }
        if power.source == PowerSource::Unknown {
            assert!(power.batteries.is_empty());
        }
    }

    #[test]
    fn build_snapshot_embeds_power_sampling_and_cgroups() {
        use tm_core::model::{SamplingProfile, LOW_BATTERY_PERCENT};

        let mut probe = NativeProbe::new().expect("native probe");
        // Warm up once so CPU deltas are meaningful, then inspect the snapshot
        // that the actual sampling path produces.
        let _ = probe.sample_native().expect("warm-up sample");
        let snapshot = probe.sample_native().expect("sample");

        assert_eq!(
            snapshot.sampling,
            recommended_sampling_plan(&snapshot.power, WindowVisibility::Visible),
            "the embedded plan must be derived from the same power reading"
        );
        let expected_profile = if snapshot.power.on_battery() {
            if snapshot
                .power
                .min_capacity_percent()
                .is_some_and(|percent| percent <= LOW_BATTERY_PERCENT)
            {
                SamplingProfile::BatterySaver
            } else {
                SamplingProfile::Battery
            }
        } else {
            SamplingProfile::Interactive
        };
        assert_eq!(snapshot.sampling.profile, expected_profile);
        // A stable, deduplicated grouping must not invent empty labels.
        for group in &snapshot.cgroups {
            assert!(!group.label.is_empty());
            assert!(!group.members.is_empty());
            let mut sorted = group.members.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(group.members, sorted, "members must be sorted and unique");
        }
        #[cfg(target_os = "linux")]
        assert!(
            !snapshot.cgroups.is_empty(),
            "the Linux probe must see its own cgroup"
        );
    }

    #[test]
    fn native_probe_and_new_domain_types_stay_send() {
        // The native shell samples on a worker thread; adding power/sampling/
        // cgroup data must not weaken that existing thread contract.
        fn assert_send<T: Send>() {}
        assert_send::<NativeProbe>();
        assert_send::<PowerSnapshot>();
        assert_send::<SamplingPlan>();
        assert_send::<ProcessCgroup>();
    }

    #[test]
    fn probe_sampling_plan_follows_the_core_policy() {
        use tm_core::model::{
            SamplingProfile, BACKGROUND_INTERVAL_MS, BATTERY_INTERVAL_MS,
            BATTERY_SAVER_INTERVAL_MS, INTERACTIVE_INTERVAL_MS,
        };

        let probe = NativeProbe::new().expect("native probe");
        let hidden = probe.sampling_plan(WindowVisibility::Hidden);
        assert_eq!(hidden.interval_ms, BACKGROUND_INTERVAL_MS);
        assert_eq!(hidden.profile, SamplingProfile::Background);

        let visible = probe.sampling_plan(WindowVisibility::Visible);
        let expected = match visible.profile {
            SamplingProfile::Interactive => INTERACTIVE_INTERVAL_MS,
            SamplingProfile::Battery => BATTERY_INTERVAL_MS,
            SamplingProfile::BatterySaver => BATTERY_SAVER_INTERVAL_MS,
            SamplingProfile::Background => {
                panic!("a visible window must never select the background profile")
            }
        };
        assert_eq!(visible.interval_ms, expected);
        assert!(visible.interval_ms > 0, "a sampler must never spin at 0 ms");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cgroup_collection_keeps_the_one_row_per_process_and_thread_contract() {
        use tm_core::model::CGROUP_LABEL_MAX_CHARS;

        let mut probe = NativeProbe::new().expect("native probe");
        let snapshot = probe.sample().expect("sample");
        let before = snapshot
            .processes
            .iter()
            .map(|process| (process.key, process.threads))
            .collect::<std::collections::BTreeMap<_, _>>();

        let memberships = probe.cgroup_memberships();

        let after_system = probe
            .system
            .processes()
            .iter()
            .filter(|(_, process)| process.thread_kind() != Some(ThreadKind::Userland))
            .map(|(pid, _)| Pid::from_u32(pid.as_u32()))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            after_system.len(),
            before.len(),
            "reading /proc must not insert new process or thread rows"
        );
        assert!(
            probe
                .system
                .processes()
                .values()
                .all(|process| process.thread_kind() != Some(ThreadKind::Userland)),
            "reading /proc must not insert userland threads into the process map"
        );
        for (key, threads) in before {
            let current = probe
                .system
                .process(Pid::from_u32(key.pid))
                .expect("the process must still be sampled");
            assert_eq!(current.start_time(), key.start_time_secs);
            assert_eq!(
                thread_count(current),
                threads,
                "the threads column must not change while collecting cgroups"
            );
        }
        assert!(!after_system.is_empty(), "the sampler must see processes");

        let keys = memberships
            .iter()
            .map(|membership| membership.key)
            .collect::<Vec<_>>();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "memberships must be sorted by process key");
        assert!(
            memberships
                .iter()
                .any(|membership| membership.key.pid == std::process::id()),
            "the probe must see its own cgroup"
        );
        for membership in &memberships {
            assert!(!membership.label.is_empty());
            assert!(
                !membership.label.chars().any(char::is_control),
                "control character in cgroup label: {:?}",
                membership.label
            );
            assert!(
                membership.label.chars().count() <= CGROUP_LABEL_MAX_CHARS + 1,
                "cgroup label was not truncated"
            );
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn cgroup_collection_is_empty_on_unsupported_platforms() {
        let probe = NativeProbe::new().expect("native probe");
        assert!(probe.cgroup_memberships().is_empty());
        assert!(probe.cgroup_groups().is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cgroup_groups_are_a_deterministic_view_of_memberships() {
        let probe = NativeProbe::new().expect("native probe");
        let memberships = probe.cgroup_memberships();
        let groups = probe.cgroup_groups();
        let labels = groups
            .iter()
            .map(|group| group.label.as_str())
            .collect::<Vec<_>>();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        assert_eq!(labels, sorted, "groups must be sorted by label");
        for group in &groups {
            assert!(!group.members.is_empty());
            let mut members = group.members.clone();
            members.sort_unstable();
            members.dedup();
            assert_eq!(group.members, members);
            assert!(
                memberships
                    .iter()
                    .any(|membership| membership.label == group.label
                        && group.members.contains(&membership.key)),
                "every grouped member must come from a collected membership"
            );
        }
    }
}
