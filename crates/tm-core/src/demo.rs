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
    ComponentSnapshot, CpuSnapshot, DiskUsageSnapshot, MemorySnapshot, NetworkInterfaceSnapshot,
    NetworkSnapshot, ProcessKey, ProcessSnapshot, ProcessStatus, SystemSnapshot, Timestamp,
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
        command: "/usr/bin/Xwayland :1 -auth /home/user/.Xauthority -displayfd 3",
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
        command: "/home/user/.rustup/toolchains/stable/bin/rustc --crate-name tm_core",
        user: "user",
        base_cpu: 22.3,
        base_memory: 440_401_920,
        status: ProcessStatus::Running,
        threads: 8,
    },
    DemoProcess {
        pid: 901,
        name: "cargo",
        command: "/home/user/.cargo/bin/cargo build --workspace",
        user: "user",
        base_cpu: 1.2,
        base_memory: 99_614_720,
        status: ProcessStatus::Sleeping,
        threads: 6,
    },
    DemoProcess {
        pid: 4242,
        name: "gpui-task-manager",
        command: "/home/user/.../gpui-task-manager",
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

    let processes = DEMO_PROCESSES
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
        .collect();

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

    SystemSnapshot {
        sampled_at: Timestamp::from_secs(DEMO_BASE_UNIX_SECS.saturating_add(tick)),
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
        disks: vec![
            DiskUsageSnapshot {
                mount_point: "/".to_string(),
                total_bytes: 512 * 1_024 * 1_024 * 1_024,
                available_bytes: 94 * 1_024 * 1_024 * 1_024,
                used_bytes: 418 * 1_024 * 1_024 * 1_024,
            },
            DiskUsageSnapshot {
                mount_point: "/home".to_string(),
                total_bytes: 1_024 * 1_024 * 1_024 * 1_024,
                available_bytes: 384 * 1_024 * 1_024 * 1_024,
                used_bytes: 640 * 1_024 * 1_024 * 1_024,
            },
        ],
        components,
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

    /// Creates a demo probe starting at an arbitrary tick (used by tests).
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
    fn demo_snapshot_is_deterministic() {
        assert_eq!(demo_snapshot(7), demo_snapshot(7));
        assert_ne!(demo_snapshot(7), demo_snapshot(8));
    }

    #[test]
    fn demo_snapshot_has_a_realistic_process_list() {
        let snapshot = demo_snapshot(0);
        assert!(snapshot.process_count() >= 20);
        assert!(snapshot.process(ProcessKey::new(4242, 0)).is_none());
        assert!(snapshot.processes.iter().any(|p| p.key.pid == 4242));
        assert_eq!(snapshot.self_pid, Some(4242));
        assert!(
            snapshot
                .processes
                .iter()
                .any(|p| p.status == ProcessStatus::Zombie)
        );
        assert!(
            snapshot
                .processes
                .iter()
                .any(|p| p.status == ProcessStatus::Stopped)
        );
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
        assert!(
            snapshot
                .cpu
                .per_core_percent
                .iter()
                .all(|percent| (0.0..=100.0).contains(percent))
        );
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
}
