//! Deterministic synthetic system data.
//!
//! The demo source serves three purposes:
//!
//! 1. the browser shell (a browser cannot read `/proc`),
//! 2. UI development without a live machine,
//! 3. deterministic tests for the view-model.
//!
//! It is a pure function of a tick counter, so two runs with the same tick are
//! byte-for-byte identical and can be asserted in tests.

use crate::model::{
    BatterySnapshot, BatteryState, CgroupGroup, ComponentSnapshot, CpuSnapshot, DiskUsageSnapshot,
    MemorySnapshot, NetworkInterfaceSnapshot, NetworkSnapshot, PowerSnapshot, PowerSource,
    ProcessKey, ProcessSnapshot, ProcessStatus, SamplingPlan, SamplingProfile, SystemSnapshot,
    Timestamp, INTERACTIVE_INTERVAL_MS,
};
use crate::port::{ControlError, PlatformInfo, ProbeError, SystemProbe};

struct DemoProcess {
    pid: u32,
    name: &'static str,
    command: &'static str,
    user: &'static str,
    base_cpu: f32,
    base_memory: u64,
    status: ProcessStatus,
    threads: u32,
}

/// A representative Kubuntu desktop process list.
const DEMO_PROCESSES: &[DemoProcess] = &[
    DemoProcess {
        pid: 1,
        name: "systemd",
        command: "/sbin/init splash",
        user: "root",
        base_cpu: 0.1,
        base_memory: 12_582_912,
        status: ProcessStatus::Sleeping,
        threads: 1,
    },
    DemoProcess {
        pid: 2,
        name: "kthreadd",
        command: "[kthreadd]",
        user: "root",
        base_cpu: 0.0,
        base_memory: 0,
        status: ProcessStatus::Idle,
        threads: 1,
    },
    DemoProcess {
        pid: 430,
        name: "systemd-journald",
        command: "/usr/lib/systemd/systemd-journald",
        user: "root",
        base_cpu: 0.5,
        base_memory: 29_360_128,
        status: ProcessStatus::Sleeping,
        threads: 3,
    },
    DemoProcess {
        pid: 700,
        name: "dbus-daemon",
        command: "/usr/bin/dbus-daemon --system --address=systemd: --nofork",
        user: "root",
        base_cpu: 0.1,
        base_memory: 8_388_608,
        status: ProcessStatus::Sleeping,
        threads: 1,
    },
    DemoProcess {
        pid: 600,
        name: "NetworkManager",
        command: "/usr/sbin/NetworkManager --no-daemon",
        user: "root",
        base_cpu: 0.3,
        base_memory: 23_068_672,
        status: ProcessStatus::Sleeping,
        threads: 3,
    },
    DemoProcess {
        pid: 1200,
        name: "sshd",
        command: "/usr/sbin/sshd -D",
        user: "root",
        base_cpu: 0.0,
        base_memory: 9_437_184,
        status: ProcessStatus::Sleeping,
        threads: 1,
    },
    DemoProcess {
        pid: 2000,
        name: "bluetoothd",
        command: "/usr/libexec/bluetooth/bluetoothd",
        user: "root",
        base_cpu: 0.1,
        base_memory: 6_291_456,
        status: ProcessStatus::Sleeping,
        threads: 1,
    },
    DemoProcess {
        pid: 2100,
        name: "cupsd",
        command: "/usr/sbin/cupsd -l",
        user: "root",
        base_cpu: 0.0,
        base_memory: 5_242_880,
        status: ProcessStatus::Sleeping,
        threads: 1,
    },
    DemoProcess {
        pid: 2200,
        name: "avahi-daemon",
        command: "avahi-daemon: running [kubuntu.local]",
        user: "avahi",
        base_cpu: 0.0,
        base_memory: 4_194_304,
        status: ProcessStatus::Sleeping,
        threads: 1,
    },
    DemoProcess {
        pid: 420,
        name: "kwin_wayland",
        command: "/usr/bin/kwin_wayland --wayland-fd 7 --socket wayland-0 --xwayland",
        user: "user",
        base_cpu: 4.2,
        base_memory: 188_743_680,
        status: ProcessStatus::Running,
        threads: 12,
    },
    DemoProcess {
        pid: 421,
        name: "plasmashell",
        command: "/usr/bin/plasmashell --no-respawn",
        user: "user",
        base_cpu: 2.1,
        base_memory: 220_200_960,
        status: ProcessStatus::Sleeping,
        threads: 9,
    },
    DemoProcess {
        pid: 1100,
        name: "Xwayland",
        command: "/usr/bin/Xwayland :1 -auth <home>/.Xauthority -displayfd 3",
        user: "user",
        base_cpu: 1.4,
        base_memory: 167_772_160,
        status: ProcessStatus::Sleeping,
        threads: 7,
    },
    DemoProcess {
        pid: 1900,
        name: "kded6",
        command: "/usr/bin/kded6",
        user: "user",
        base_cpu: 0.2,
        base_memory: 41_943_040,
        status: ProcessStatus::Sleeping,
        threads: 2,
    },
    DemoProcess {
        pid: 500,
        name: "pipewire",
        command: "/usr/bin/pipewire",
        user: "user",
        base_cpu: 0.8,
        base_memory: 18_874_368,
        status: ProcessStatus::Sleeping,
        threads: 4,
    },
    DemoProcess {
        pid: 501,
        name: "wireplumber",
        command: "/usr/bin/wireplumber",
        user: "user",
        base_cpu: 0.4,
        base_memory: 14_680_064,
        status: ProcessStatus::Sleeping,
        threads: 3,
    },
    DemoProcess {
        pid: 800,
        name: "firefox",
        command: "/usr/lib/firefox/firefox --new-window",
        user: "user",
        base_cpu: 18.4,
        base_memory: 1_998_489_600,
        status: ProcessStatus::Running,
        threads: 64,
    },
    DemoProcess {
        pid: 810,
        name: "code",
        command: "/usr/share/code/code --no-sandbox --unity-launch",
        user: "user",
        base_cpu: 9.7,
        base_memory: 1_261_289_472,
        status: ProcessStatus::Sleeping,
        threads: 48,
    },
    DemoProcess {
        pid: 900,
        name: "rustc",
        command: "<home>/.rustup/toolchains/stable/bin/rustc --crate-name tm_core",
        user: "user",
        base_cpu: 22.3,
        base_memory: 440_401_920,
        status: ProcessStatus::Running,
        threads: 8,
    },
    DemoProcess {
        pid: 901,
        name: "cargo",
        command: "<home>/.cargo/bin/cargo build --workspace",
        user: "user",
        base_cpu: 1.2,
        base_memory: 99_614_720,
        status: ProcessStatus::Sleeping,
        threads: 6,
    },
    // R2-DEMO: the built-in "高 CPU" preset keeps rows at or above 50 %. Add
    // two deterministic rows above that threshold. `base_cpu` is the minimum
    // value of the jittered CPU reading, so they stay above 50 % on every tick
    // (including tick 0 and u64::MAX) and the preset is never empty in demo mode.
    DemoProcess {
        pid: 2500,
        name: "ffmpeg",
        command: "/usr/bin/ffmpeg -i recording.mkv -c:v libx264 out.mp4",
        user: "user",
        base_cpu: 72.5,
        base_memory: 512_000_000,
        status: ProcessStatus::Running,
        threads: 8,
    },
    DemoProcess {
        pid: 2600,
        name: "rust-analyzer",
        command: "<home>/.cargo/bin/rust-analyzer",
        user: "user",
        base_cpu: 55.0,
        base_memory: 298_000_000,
        status: ProcessStatus::Running,
        threads: 12,
    },
    DemoProcess {
        pid: 4242,
        name: "gpui-task-manager",
        command: "<home>/.../gpui-task-manager",
        user: "user",
        base_cpu: 6.8,
        base_memory: 188_743_680,
        status: ProcessStatus::Running,
        threads: 14,
    },
    DemoProcess {
        pid: 1300,
        name: "tm-agent",
        command: "/usr/bin/tm-agent --listen 127.0.0.1:8787",
        user: "user",
        base_cpu: 0.2,
        base_memory: 12_582_912,
        status: ProcessStatus::Sleeping,
        threads: 3,
    },
    DemoProcess {
        pid: 1400,
        name: "spectacle",
        command: "/usr/bin/spectacle --background",
        user: "user",
        base_cpu: 0.1,
        base_memory: 73_400_320,
        status: ProcessStatus::Sleeping,
        threads: 5,
    },
    DemoProcess {
        pid: 1500,
        name: "python3",
        command: "/usr/bin/python3 -m http.server 8000",
        user: "user",
        base_cpu: 0.4,
        base_memory: 26_214_400,
        status: ProcessStatus::Sleeping,
        threads: 1,
    },
    DemoProcess {
        pid: 1600,
        name: "bash",
        command: "/usr/bin/bash",
        user: "user",
        base_cpu: 0.0,
        base_memory: 7_340_032,
        status: ProcessStatus::Sleeping,
        threads: 1,
    },
    DemoProcess {
        pid: 1700,
        name: "sleep",
        command: "/usr/bin/sleep 600",
        user: "user",
        base_cpu: 0.0,
        base_memory: 3_145_728,
        status: ProcessStatus::Sleeping,
        threads: 1,
    },
    DemoProcess {
        pid: 1800,
        name: "konsole",
        command: "/usr/bin/konsole --new-tab",
        user: "user",
        base_cpu: 0.6,
        base_memory: 89_128_960,
        status: ProcessStatus::Sleeping,
        threads: 8,
    },
    DemoProcess {
        pid: 2300,
        name: "zombie-proc",
        command: "[defunct]",
        user: "user",
        base_cpu: 0.0,
        base_memory: 0,
        status: ProcessStatus::Zombie,
        threads: 1,
    },
    DemoProcess {
        pid: 2400,
        name: "tail",
        command: "/usr/bin/tail -f /var/log/syslog",
        user: "user",
        base_cpu: 0.0,
        base_memory: 3_407_872,
        status: ProcessStatus::Stopped,
        threads: 1,
    },
];

const DEMO_CPU_CORES: usize = 12;
const DEMO_MEMORY_TOTAL: u64 = 16 * 1_024 * 1_024 * 1_024;
const DEMO_MEMORY_USED_BASE: u64 = 9 * 1_024 * 1_024 * 1_024;
const DEMO_SWAP_TOTAL: u64 = 2 * 1_024 * 1_024 * 1_024;
const DEMO_SWAP_USED: u64 = 256 * 1_024 * 1_024;
/// Reference wall-clock time used by the synthetic data.
pub const DEMO_BASE_UNIX_SECS: u64 = 1_760_000_000;

/// Start time of a demo process (within one day before [`DEMO_BASE_UNIX_SECS`]).
pub fn demo_process_start_time(pid: u32) -> u64 {
    DEMO_BASE_UNIX_SECS.saturating_sub((pid as u64 * 37) % 86_400)
}

/// Stable identity of a demo process, if the PID exists in the demo table.
pub fn demo_process_key(pid: u32) -> Option<ProcessKey> {
    DEMO_PROCESSES
        .iter()
        .find(|demo| demo.pid == pid)
        .map(|demo| ProcessKey::new(demo.pid, demo_process_start_time(demo.pid)))
}

/// Returns a deterministic snapshot for the given tick.
///
/// The same tick always produces the same snapshot, including process CPU
/// values, network counters and timestamps.
pub fn demo_snapshot(tick: u64) -> SystemSnapshot {
    let jitter = |salt: u64| ((tick.wrapping_mul(31).wrapping_add(salt) % 100) as f32) / 100.0;

    let mut processes: Vec<ProcessSnapshot> = DEMO_PROCESSES
        .iter()
        .enumerate()
        .map(|(index, demo)| {
            let noise = jitter(demo.pid as u64 * 17 + index as u64 * 7);
            let cpu_percent = if demo.base_cpu == 0.0 {
                0.0
            } else {
                (demo.base_cpu + noise * 6.0).min(399.0)
            };
            // A zero base means kernel threads / zombies: keep RSS and virtual
            // memory at 0 for every tick, just like `base_cpu == 0.0` above.
            // `noise` is in 0.0..1.0, so cast it to u64 after scaling.
            let (memory_bytes, virtual_memory_bytes) = if demo.base_memory == 0 {
                (0, 0)
            } else {
                let jitter_bytes = (noise * 4_096.0) as u64;
                let resident = demo
                    .base_memory
                    .saturating_add(jitter_bytes.saturating_mul((index as u64 % 5) + 1));
                (
                    resident,
                    resident.saturating_mul(3).saturating_add(1_048_576),
                )
            };
            let start_time_secs = demo_process_start_time(demo.pid);
            ProcessSnapshot {
                key: ProcessKey::new(demo.pid, start_time_secs),
                name: demo.name.to_string(),
                command: demo.command.to_string(),
                user: Some(demo.user.to_string()),
                status: demo.status,
                cpu_percent,
                memory_bytes,
                virtual_memory_bytes,
                threads: Some(demo.threads),
                parent_pid: if demo.pid <= 2 {
                    None
                } else if demo.pid < 400 {
                    Some(1)
                } else {
                    Some(421)
                },
            }
        })
        .collect::<Vec<ProcessSnapshot>>();
    // Match the native probe's (pid, start_time_secs) order. The table sorts
    // itself, but --export writes the snapshot order directly (R42 audit).
    processes.sort_by_key(|process| (process.key.pid, process.key.start_time_secs));

    let per_core_percent = (0..DEMO_CPU_CORES)
        .map(|core| {
            let noise = jitter(core as u64 * 101 + 11);
            (8.0 + core as f32 * 1.9 + noise * 35.0).clamp(0.0, 100.0)
        })
        .collect::<Vec<_>>();
    let total_percent = per_core_percent.iter().sum::<f32>() / DEMO_CPU_CORES as f32;

    let memory_used = DEMO_MEMORY_USED_BASE
        .saturating_add((jitter(7) * (512.0 * 1_024.0 * 1_024.0)) as u64)
        .min(DEMO_MEMORY_TOTAL);

    let network = NetworkSnapshot {
        interfaces: vec![
            NetworkInterfaceSnapshot {
                name: "wlan0".to_string(),
                received_bytes: 42_000_000_000u64.saturating_add(tick.saturating_mul(1_234_567)),
                transmitted_bytes: 7_500_000_000u64.saturating_add(tick.saturating_mul(321_456)),
            },
            NetworkInterfaceSnapshot {
                name: "lo".to_string(),
                received_bytes: 1_000_000u64.saturating_add(tick.saturating_mul(4_096)),
                transmitted_bytes: 1_000_000u64.saturating_add(tick.saturating_mul(4_096)),
            },
        ],
    };

    let components = vec![
        ComponentSnapshot {
            label: "CPU Package".to_string(),
            temperature_c: Some(42.0 + jitter(41) * 18.0),
            max_c: Some(68.0),
            critical_c: Some(100.0),
        },
        ComponentSnapshot {
            label: "NVMe Composite".to_string(),
            temperature_c: Some(38.0 + jitter(42) * 10.0),
            max_c: Some(52.0),
            critical_c: Some(85.0),
        },
    ];

    // Deterministic demo power/cgroup data (R1-I10..I12). A full battery on AC
    // selects the interactive plan, whose floor is now 0.5 s so the advertised
    // 0.5 s preset is effective. The default configured 1 s is still honoured,
    // so the demo tick and every CDP timing constant stay unchanged; two static
    // groups exercise the grouping display.
    let power = PowerSnapshot {
        source: PowerSource::Ac,
        batteries: vec![BatterySnapshot {
            name: "DEMO0".to_string(),
            capacity_percent: Some(88),
            state: BatteryState::Full,
        }],
    };
    let cgroups = vec![
        CgroupGroup {
            label: "demo.slice/system.service".to_string(),
            members: processes
                .iter()
                .take(3)
                .map(|process| process.key)
                .collect(),
        },
        CgroupGroup {
            label: "demo.slice/user.service".to_string(),
            members: processes
                .iter()
                .skip(3)
                .take(4)
                .map(|process| process.key)
                .collect(),
        },
    ];

    SystemSnapshot {
        sampled_at: Timestamp::from_secs(DEMO_BASE_UNIX_SECS.saturating_add(tick)),
        sampled_elapsed: std::time::Duration::from_secs(1),
        hostname: Some("kubuntu-demo".to_string()),
        os_name: Some("Linux".to_string()),
        kernel_version: Some("6.11.0-demo".to_string()),
        cpu: CpuSnapshot {
            total_percent,
            per_core_percent,
            load_average: [
                0.42 + jitter(1) * 0.6,
                0.38 + jitter(2) * 0.4,
                0.31 + jitter(3) * 0.2,
            ],
            uptime_secs: 259_200_u64.saturating_add(tick),
            frequency_mhz: Some(3_600 + (tick % 7) * 100),
        },
        memory: MemorySnapshot {
            total_bytes: DEMO_MEMORY_TOTAL,
            used_bytes: memory_used,
            available_bytes: DEMO_MEMORY_TOTAL.saturating_sub(memory_used),
            swap_total_bytes: DEMO_SWAP_TOTAL,
            swap_used_bytes: DEMO_SWAP_USED,
        },
        processes,
        network,
        // Keep the same order as the native probe's `sort_disks_for_display`
        // (largest capacity first): the summary shows only the first rows.
        disks: vec![
            DiskUsageSnapshot {
                mount_point: "/home".to_string(),
                total_bytes: 1_024 * 1_024 * 1_024 * 1_024,
                available_bytes: 384 * 1_024 * 1_024 * 1_024,
                used_bytes: 640 * 1_024 * 1_024 * 1_024,
            },
            DiskUsageSnapshot {
                mount_point: "/".to_string(),
                total_bytes: 512 * 1_024 * 1_024 * 1_024,
                available_bytes: 94 * 1_024 * 1_024 * 1_024,
                used_bytes: 418 * 1_024 * 1_024 * 1_024,
            },
        ],
        components,
        power,
        sampling: SamplingPlan {
            interval_ms: INTERACTIVE_INTERVAL_MS,
            profile: SamplingProfile::Interactive,
        },
        cgroups,
        self_pid: Some(4_242),
    }
}

/// [`SystemProbe`] implementation backed by [`demo_snapshot`].
#[derive(Clone, Debug, Default)]
pub struct DemoProbe {
    tick: u64,
}

impl DemoProbe {
    /// Creates a demo probe starting at tick zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a demo probe starting at an arbitrary tick.
    ///
    /// Test configuration helper for deterministic scenarios. There are no
    /// in-tree call sites at present, but this is a public API of a library
    /// crate, so it is deliberately kept available to downstream users instead
    /// of being hidden behind `#[cfg(test)]`.
    pub fn with_tick(tick: u64) -> Self {
        Self { tick }
    }

    /// Number of samples produced so far.
    pub fn tick(&self) -> u64 {
        self.tick
    }
}

impl SystemProbe for DemoProbe {
    fn platform_info(&self) -> PlatformInfo {
        PlatformInfo::demo()
    }

    fn sample(&mut self) -> Result<SystemSnapshot, ProbeError> {
        let snapshot = demo_snapshot(self.tick);
        self.tick = self.tick.saturating_add(1);
        Ok(snapshot)
    }

    fn control(
        &mut self,
        _key: ProcessKey,
        _signal: crate::port::Signal,
    ) -> Result<(), ControlError> {
        Err(ControlError::Unsupported)
    }
}

/// Backend chosen by [`choose_backend`].
///
/// A failed native probe is deliberately *not* a variant: callers report it
/// with a classified exit status instead of substituting synthetic data behind
/// the user's back (R3-S02 / R1-V05 demo-vs-live distinction).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendChoice<P> {
    /// Synthetic data, requested explicitly with a validated `--demo`.
    Demo,
    /// Live data from a successfully created native probe.
    Native(P),
}

/// Resolves the data source for a shell entry point.
///
/// `requested_demo` must come from a strict parser that rejects inline values
/// (`--demo=yes`) and typos; silently ignoring a malformed value used to start
/// the GUI with live data (R3-S03). `native` is called only when demo data was
/// *not* requested, so `--demo` stays usable on hosts where the native probe
/// cannot be created. A native probe failure is returned to the caller and is
/// never converted into [`BackendChoice::Demo`]: both the GUI and `--export`
/// therefore share one policy — demo is an explicit, validated opt-in and a
/// probe failure is an error (R3-S02).
pub fn choose_backend<P>(
    requested_demo: bool,
    native: impl FnOnce() -> Result<P, ProbeError>,
) -> Result<BackendChoice<P>, ProbeError> {
    if requested_demo {
        Ok(BackendChoice::Demo)
    } else {
        native().map(BackendChoice::Native)
    }
}

/// Standard shell message for a native probe failure.
///
/// Shared by the GUI startup and `--export` so both report the same classified
/// failure instead of one of them quietly switching to demo data (R3-S02).
pub fn probe_failure_message(error: &ProbeError) -> String {
    format!("スナップショット収集を初期化できません: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_process_memory_varies_with_tick() {
        // `noise` used to be cast to u64 before scaling, which is always 0 and
        // froze the memory column in the browser demo (R21 demo review).
        let first = demo_snapshot(0);
        let second = demo_snapshot(1);
        let find = |snapshot: &SystemSnapshot, pid: u32| {
            snapshot
                .processes
                .iter()
                .find(|process| process.key.pid == pid)
                .expect("process present")
                .memory_bytes
        };
        let pid = first.processes[0].key.pid;
        assert_ne!(find(&first, pid), find(&second, pid));
        for tick in [0, 1, 42, u64::MAX] {
            let snapshot = demo_snapshot(tick);
            for process in &snapshot.processes {
                assert!(
                    process.virtual_memory_bytes >= process.memory_bytes,
                    "virtual memory must cover resident memory"
                );
            }
        }
    }

    #[test]
    fn zero_rss_demo_processes_stay_at_zero() {
        for tick in [0, 1, 42, u64::MAX] {
            let snapshot = demo_snapshot(tick);
            for process in &snapshot.processes {
                if process.name == "kthreadd" || process.name == "zombie-proc" {
                    assert_eq!(
                        process.memory_bytes, 0,
                        "{} must have zero RSS",
                        process.name
                    );
                    assert_eq!(
                        process.virtual_memory_bytes, 0,
                        "{} must have zero virtual memory",
                        process.name
                    );
                    assert_eq!(process.cpu_percent, 0.0);
                }
            }
        }
    }

    #[test]
    fn demo_matches_native_probe_ordering_contracts() {
        let snapshot = demo_snapshot(7);
        // The native probe sorts processes by (pid, start_time_secs).
        for window in snapshot.processes.windows(2) {
            let key = (window[0].key.pid, window[0].key.start_time_secs);
            let next = (window[1].key.pid, window[1].key.start_time_secs);
            assert!(
                key <= next,
                "processes must be PID-ascending: {key:?} > {next:?}"
            );
        }
        // `sort_disks_for_display` puts the largest capacity first.
        for window in snapshot.disks.windows(2) {
            assert!(
                window[0].total_bytes >= window[1].total_bytes,
                "disks must be capacity-descending"
            );
        }
    }

    #[test]
    fn demo_exposes_high_cpu_rows_for_the_builtin_preset() {
        // Mirrors tm_viewmodel::PRESET_HIGH_CPU_PERCENT (50.0). tm-core must not
        // depend on the view-model crate, so the threshold is repeated here.
        const HIGH_CPU_PERCENT: f32 = 50.0;
        for tick in [0, 1, 7, 42, u64::MAX] {
            let snapshot = demo_snapshot(tick);
            let high_cpu = snapshot
                .processes
                .iter()
                .filter(|process| process.cpu_percent >= HIGH_CPU_PERCENT)
                .count();
            assert!(
                (1..=2).contains(&high_cpu),
                "tick {tick}: expected 1-2 high-CPU demo rows, got {high_cpu}"
            );
        }
        // CPU-descending sort (the default view) must surface those rows first.
        let mut by_cpu = demo_snapshot(0).processes.clone();
        by_cpu.sort_by(|left, right| right.cpu_percent.total_cmp(&left.cpu_percent));
        assert!(by_cpu[0].cpu_percent >= HIGH_CPU_PERCENT);
        assert!(by_cpu[1].cpu_percent >= HIGH_CPU_PERCENT);
    }

    #[test]
    fn demo_snapshot_is_deterministic() {
        assert_eq!(demo_snapshot(7), demo_snapshot(7));
        assert_ne!(demo_snapshot(7), demo_snapshot(8));
    }

    #[test]
    fn demo_snapshot_exposes_consistent_power_sampling_and_cgroups() {
        use crate::model::{
            recommended_sampling_plan, BatteryState, PowerSource, WindowVisibility,
        };

        let snapshot = demo_snapshot(0);
        assert_eq!(snapshot.power.source, PowerSource::Ac);
        assert_eq!(snapshot.power.batteries.len(), 1);
        assert_eq!(snapshot.power.batteries[0].state, BatteryState::Full);
        assert!(!snapshot.power.on_battery());
        assert_eq!(
            snapshot.sampling,
            recommended_sampling_plan(&snapshot.power, WindowVisibility::Visible)
        );
        assert_eq!(snapshot.sampling.interval_ms, INTERACTIVE_INTERVAL_MS);
        // The 0.5 s preset must survive the interactive floor (R2-DEMO).
        assert_eq!(
            snapshot
                .sampling
                .effective_interval(std::time::Duration::from_millis(500)),
            std::time::Duration::from_millis(500)
        );
        assert_eq!(snapshot.cgroups.len(), 2);
        for group in &snapshot.cgroups {
            assert!(!group.members.is_empty());
            for key in &group.members {
                assert!(snapshot.process(*key).is_some(), "group member must exist");
            }
        }
    }

    #[test]
    fn demo_snapshot_has_a_realistic_process_list() {
        let snapshot = demo_snapshot(0);
        assert!(snapshot.process_count() >= 20);
        assert!(snapshot.process(ProcessKey::new(4242, 0)).is_none());
        assert!(snapshot.processes.iter().any(|p| p.key.pid == 4242));
        assert_eq!(snapshot.self_pid, Some(4242));
        assert!(snapshot
            .processes
            .iter()
            .any(|p| p.status == ProcessStatus::Zombie));
        assert!(snapshot
            .processes
            .iter()
            .any(|p| p.status == ProcessStatus::Stopped));
    }

    #[test]
    fn demo_network_counters_are_monotonic() {
        let (rx0, tx0) = demo_snapshot(0).network_totals();
        let (rx1, tx1) = demo_snapshot(1).network_totals();
        assert!(rx1 > rx0);
        assert!(tx1 > tx0);
    }

    #[test]
    fn demo_cpu_values_are_in_range() {
        let snapshot = demo_snapshot(42);
        assert_eq!(snapshot.cpu.core_count(), DEMO_CPU_CORES);
        assert!((0.0..=100.0).contains(&snapshot.cpu.total_percent));
        assert!(snapshot
            .cpu
            .per_core_percent
            .iter()
            .all(|percent| (0.0..=100.0).contains(percent)));
        assert!(snapshot.memory.used_percent() <= 100.0);
    }

    #[test]
    fn extreme_tick_saturates_sampled_at() {
        // DEMO_BASE_UNIX_SECS + u64::MAX would overflow in debug builds; the
        // demo must stay total so a fuzzed/generated tick cannot panic (R1S-DATA-10).
        let snapshot = demo_snapshot(u64::MAX);
        assert_eq!(snapshot.sampled_at, Timestamp::from_secs(u64::MAX));
    }

    #[test]
    fn demo_snapshot_includes_deterministic_components() {
        let snapshot = demo_snapshot(5);
        assert_eq!(snapshot.components.len(), 2);
        assert_eq!(snapshot.components[0].label, "CPU Package");
        assert!(snapshot.components[0].temperature_c.is_some());
        assert_eq!(snapshot.components[0].max_c, Some(68.0));
        assert_eq!(snapshot.components[0].critical_c, Some(100.0));
        assert_eq!(snapshot.components[1].label, "NVMe Composite");
        assert!(snapshot.components[1].temperature_c.is_some());
        assert_eq!(demo_snapshot(5).components, demo_snapshot(5).components);
        assert_ne!(
            demo_snapshot(5).components[0].temperature_c,
            demo_snapshot(6).components[0].temperature_c
        );
    }

    #[test]
    fn demo_probe_advances_ticks_and_is_read_only() {
        let mut probe = DemoProbe::new();
        let first = probe.sample().expect("demo sample");
        let second = probe.sample().expect("demo sample");
        assert_eq!(probe.tick(), 2);
        assert!(second.sampled_at > first.sampled_at);
        assert_eq!(probe.platform_info(), PlatformInfo::demo());
        assert_eq!(
            probe.control(ProcessKey::new(1, 0), crate::port::Signal::Terminate),
            Err(ControlError::Unsupported)
        );
    }

    #[test]
    fn choose_backend_never_falls_back_to_demo_on_probe_failure() {
        // R3-S02: a failed native probe is an error for every shell, not an
        // implicit switch to synthetic data. `--demo` remains an explicit
        // opt-in that works even when the native backend is unavailable.
        let failure = || Err::<(), _>(ProbeError::Io("sysinfo の初期化に失敗".to_string()));
        assert_eq!(
            choose_backend(false, failure),
            Err(ProbeError::Io("sysinfo の初期化に失敗".to_string()))
        );
        assert_eq!(choose_backend(true, failure), Ok(BackendChoice::Demo));

        let available = || Ok::<_, ProbeError>(42);
        assert_eq!(
            choose_backend(false, available),
            Ok(BackendChoice::Native(42))
        );
        assert_eq!(choose_backend(true, available), Ok(BackendChoice::Demo));
    }

    #[test]
    fn choose_backend_does_not_touch_native_probe_when_demo_is_requested() {
        // A `--demo` run must not depend on native probe availability at all.
        let called = std::cell::Cell::new(false);
        let selection = choose_backend(true, || {
            called.set(true);
            Ok::<_, ProbeError>(())
        });
        assert_eq!(selection, Ok(BackendChoice::Demo));
        assert!(!called.get(), "demo mode must not create a native probe");
    }

    #[test]
    fn probe_failure_message_keeps_the_classified_reason() {
        let message = probe_failure_message(&ProbeError::Io("初期化失敗".to_string()));
        assert!(
            message.starts_with("スナップショット収集を初期化できません:"),
            "{message}"
        );
        assert!(message.contains("初期化失敗"), "{message}");
        let message = probe_failure_message(&ProbeError::Permission("権限不足".to_string()));
        assert!(message.contains("権限不足"), "{message}");
    }
}
