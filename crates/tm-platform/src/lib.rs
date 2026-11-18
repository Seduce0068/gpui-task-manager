//! Native [`tm_core::SystemProbe`] implementation.
//!
//! This crate is the only place in the workspace that talks to the operating
//! system directly:
//!
//! - [`sysinfo`] 0.40 collects CPU, memory, process, network and disk data.
//! - `rustix` sends POSIX signals with its safe API on Unix, so the crate
//!   stays free of `unsafe` code.
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
    is_virtual_interface, ComponentSnapshot, CpuSnapshot, DiskUsageSnapshot, MemorySnapshot,
    NetworkInterfaceSnapshot, NetworkSnapshot, ProcessKey, ProcessSnapshot, ProcessStatus,
    SystemSnapshot, Timestamp,
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
        let now = Instant::now();
        let elapsed_ms = self
            .last_sample_at
            .map(|last| now.duration_since(last).as_millis())
            .map(|millis| u64::try_from(millis).unwrap_or(u64::MAX))
            .unwrap_or(0);
        self.refresh();
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
                        let (available, used) = disk_usage_values(total, disk.available_space());
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
}

/// Derives the `(available, used)` byte pair for one filesystem (R2X-D5).
///
/// Neither sysinfo 0.40.3 nor 0.40.4 exposes `Disk::used_space()`, and the
/// pinned version only publishes `total_space()` and `available_space()`.
/// Reading `statvfs.f_bfree` (which excludes filesystem-reserved blocks from
/// `used`) would need a new dependency and `unsafe` FFI, so this is the
/// closest equivalent derivable within the current dependency set. The
/// existing contract is preserved: `available` is never reported above
/// `total`, and `used` is the remainder.
///
/// `checked_sub` makes an anomalous `available > total` reading explicit:
/// the filesystem is then reported as entirely free (`total`, `0`) rather
/// than silently saturating inside the subtraction.
fn disk_usage_values(total_space: u64, available_space: u64) -> (u64, u64) {
    match total_space.checked_sub(available_space) {
        Some(used) => (available_space, used),
        None => (total_space, 0),
    }
}

/// Orders disks for display: largest capacity first, then by mount point.
///
/// The summary cards only show the first few entries, so sorting by mount point
/// alone would hide a large data volume behind a small system partition such as
/// `/boot/efi` (Runtime Proof: on the development host `/boot/efi` sorted before
/// `/mnt/timeshift` and the 439 GiB volume was never displayed).
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
            true,
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
        Err(err) => Err(ControlError::Failed(format!("kill: {err}"))),
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

    let output = command
        .output()
        .map_err(|err| ControlError::Failed(format!("taskkill の実行に失敗: {err}")))?;
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
/// access refusal is [`PermissionDenied`], and anything else stays [`Failed`]
/// with the original output attached for diagnostics. Both streams are scanned
/// because Windows versions differ in which one carries the error, and
/// `exit_code` is kept even for unmapped failures so it is not lost.
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

    let detail = output.trim();
    let code = exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    if detail.is_empty() {
        ControlError::Failed(format!("taskkill が失敗しました (exit={code})"))
    } else {
        ControlError::Failed(format!("taskkill が失敗しました (exit={code}): {detail}"))
    }
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

        for process in &snapshot.processes {
            let threads = process.threads.expect("Linux must report thread counts");
            let expected = probe
                .system
                .process(Pid::from_u32(process.key.pid))
                .and_then(|sysinfo_process| sysinfo_process.tasks())
                .map_or(1, |tasks| {
                    tasks.len().saturating_add(1).min(u32::MAX as usize) as u32
                });
            assert_eq!(
                threads, expected,
                "thread count of pid {} must match its task set",
                process.key.pid
            );
            assert!(threads >= 1, "a listed process always has its own thread");
        }
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
        // Mirrors the development host: /boot/efi is tiny and would otherwise
        // sort first (before /mnt/timeshift) and hide the large data volume.
        let mut disks = vec![
            disk("/boot/efi", 96 * 1_024 * 1_024),
            disk("/", 587 * 1_024 * 1_024 * 1_024),
            disk("/mnt/timeshift", 440 * 1_024 * 1_024 * 1_024),
        ];
        sort_disks_for_display(&mut disks);
        let order: Vec<&str> = disks.iter().map(|d| d.mount_point.as_str()).collect();
        assert_eq!(order, vec!["/", "/mnt/timeshift", "/boot/efi"]);
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

    /// R2X-D5 regression: with no `Disk::used_space()` in the pinned sysinfo,
    /// the equivalent value must be derived from `total` and `available`
    /// without wrapping, even at the u64 boundaries.
    #[test]
    fn disk_usage_values_match_the_used_space_equivalent() {
        let total = 1_099_511_627_776u64; // 1 TiB
        let available = 429_496_729_600u64; // 400 GiB
        assert_eq!(
            disk_usage_values(total, available),
            (available, total - available)
        );

        assert_eq!(disk_usage_values(1_024, 0), (0, 1_024));
        assert_eq!(disk_usage_values(1_024, 1_024), (1_024, 0));
        assert_eq!(disk_usage_values(0, 0), (0, 0));
        assert_eq!(disk_usage_values(u64::MAX, 0), (0, u64::MAX));
        assert_eq!(disk_usage_values(u64::MAX, u64::MAX), (u64::MAX, 0));
    }

    /// R2X-D5 regression: an inconsistent collector reading must be clamped
    /// instead of wrapping. This preserves the pre-existing `min` +
    /// `saturating_sub` outcome while making the check explicit.
    #[test]
    fn disk_usage_values_clamp_available_above_total() {
        assert_eq!(disk_usage_values(1_024, 1_025), (1_024, 0));
        assert_eq!(disk_usage_values(100, u64::MAX), (100, 0));
        assert_eq!(disk_usage_values(0, u64::MAX), (0, 0));
    }

    /// R2X-D5 regression: the three snapshot fields must stay consistent for
    /// every input, so `used_percent()` can rely on `used <= total`.
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
            let (available, used) = disk_usage_values(total, reported);
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

    /// R2X-D5 regression: the derived values must survive the real sampling
    /// path, not just the pure helper.
    #[test]
    fn sampled_disks_keep_the_total_available_used_invariant() {
        let mut probe = NativeProbe::new().expect("native probe");
        let snapshot = probe.sample().expect("sample");
        for disk in &snapshot.disks {
            assert!(
                disk.available_bytes <= disk.total_bytes,
                "{} reported available above total",
                disk.mount_point
            );
            assert_eq!(
                disk.used_bytes,
                disk.total_bytes - disk.available_bytes,
                "{} has an inconsistent used value",
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

    #[test]
    fn taskkill_unknown_failure_stays_failed_with_diagnostics() {
        let error = classify_taskkill_failure(Some(1), b"", b"ERROR: Invalid argument.");
        assert!(
            matches!(error, ControlError::Failed(ref message) if message.contains("exit=1") && message.contains("Invalid argument"))
        );
        // Preserve the exit code even when no message was written.
        let silent = classify_taskkill_failure(Some(5), b"", b"");
        assert!(matches!(silent, ControlError::Failed(ref message) if message.contains("exit=5")));
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
}
