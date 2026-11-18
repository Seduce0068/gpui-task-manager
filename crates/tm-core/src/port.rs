//! Ports: the boundary between the pure core and the operating system.
//!
//! The UI depends on these traits, never on `sysinfo`, `procfs`, `rustix` or
//! any other platform crate. That keeps the shells testable with a deterministic
//! demo source and keeps the wasm build free of native dependencies.

use std::fmt;

use crate::model::{ProcessKey, SystemSnapshot};

/// Capability-gated section of a shell (R5-I04).
///
/// Shells ask this instead of reading the raw flags so a capability cannot be
/// honoured in one widget and ignored in another. The mapping stays in the core
/// because it is part of the backend contract, not a presentation detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    /// Network counters in the overview.
    Network,
    /// Mounted filesystems in the overview.
    Disks,
    /// Process-control actions in the header / details panel.
    ProcessControl,
}

/// Description of the backend behind a [`SystemProbe`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlatformInfo {
    /// Human-readable operating system name.
    pub os: String,
    /// Whether [`SystemProbe::control`] can signal processes.
    pub supports_process_control: bool,
    /// Whether network counters are sampled.
    pub supports_network: bool,
    /// Whether mounted filesystems are sampled.
    pub supports_disks: bool,
    /// `true` when the data is synthetic (browser demo mode).
    pub is_demo: bool,
}

impl PlatformInfo {
    /// Describes a native backend.
    ///
    /// `supports_network` / `supports_disks` mirror what the native probe could
    /// actually collect: they are `true` only when the corresponding collector
    /// was created, and `false` when it could not be created (in which case
    /// that section is empty in the snapshot too). This makes "supported but
    /// currently empty" distinguishable from "collection unavailable" for
    /// callers. This constructor describes the fully available native case;
    /// a backend whose collector failed must report `false` for the affected
    /// flag. Process control is always available on a native probe.
    pub fn native(os: impl Into<String>) -> Self {
        Self {
            os: os.into(),
            supports_process_control: true,
            supports_network: true,
            supports_disks: true,
            is_demo: false,
        }
    }

    /// Describes a native backend where the platform cannot signal processes.
    ///
    /// Used by the browser shell, which reads a snapshot over HTTP and has no
    /// process-control API by design.
    pub fn native_read_only(os: impl Into<String>) -> Self {
        Self {
            os: os.into(),
            supports_process_control: false,
            supports_network: true,
            supports_disks: true,
            is_demo: false,
        }
    }

    /// Describes the deterministic demo backend used by tests and the browser.
    pub fn demo() -> Self {
        Self {
            os: "Linux (demo)".to_string(),
            supports_process_control: false,
            supports_network: true,
            supports_disks: true,
            is_demo: true,
        }
    }

    /// A short Japanese label for the status bar.
    pub fn label(&self) -> String {
        if self.is_demo {
            format!("{} / デモデータ", self.os)
        } else {
            self.os.clone()
        }
    }

    /// Whether `section` can present live data or actions on this backend.
    ///
    /// `native_read_only` and `demo` still provide network/disk snapshots, so
    /// only [`Section::ProcessControl`] is gated for them. The flags already
    /// reflect whether the native probe could create each collector (see
    /// [`Self::native`]), so this method reads only the stored capability and
    /// deliberately does not look at the current snapshot.
    pub const fn supports(&self, section: Section) -> bool {
        match section {
            Section::Network => self.supports_network,
            Section::Disks => self.supports_disks,
            Section::ProcessControl => self.supports_process_control,
        }
    }

    /// Whether this backend can deliver `signal` right now.
    ///
    /// This is the single answer shells should use for individual signal
    /// buttons. A read-only backend refuses every signal; a native Windows
    /// backend refuses only `Stop` / `Continue`, because it can still terminate
    /// processes. Section-level capability is included so a shell cannot show a
    /// signal button when process control as a whole is unavailable.
    pub fn supports_signal(&self, signal: Signal) -> bool {
        self.supports_process_control && signal.supported_on_native_os(&self.os)
    }
}

/// Process control signal exposed by the UI.
///
/// The set is intentionally small. Task managers should not offer "run this
/// command as root"; they should offer the four POSIX signals a user actually
/// needs, with an explicit confirmation step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Signal {
    /// Graceful termination (`SIGTERM`).
    Terminate,
    /// Forced termination (`SIGKILL`).
    Kill,
    /// Suspend the process (`SIGSTOP`).
    Stop,
    /// Resume the process (`SIGCONT`).
    Continue,
}

impl Signal {
    /// Full Japanese label for menus and dialogs.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Terminate => "終了 (SIGTERM)",
            Self::Kill => "強制終了 (SIGKILL)",
            Self::Stop => "一時停止 (SIGSTOP)",
            Self::Continue => "再開 (SIGCONT)",
        }
    }

    /// Short label for buttons.
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Terminate => "終了",
            Self::Kill => "強制終了",
            Self::Stop => "一時停止",
            Self::Continue => "再開",
        }
    }

    /// Returns `true` for signals that cannot be caught by the target.
    pub const fn is_destructive(self) -> bool {
        matches!(self, Self::Kill)
    }

    /// Whether a native backend on `os` can deliver this signal.
    ///
    /// Windows has no POSIX stop/continue: `tm-platform` maps `Terminate` and
    /// `Kill` to `taskkill /F` and returns `ControlError::Unsupported` for
    /// `Stop` / `Continue`. Shells must ask this before rendering a button that
    /// can never work instead of letting every click fail (R2X-P2).
    pub fn supported_on_native_os(self, os: &str) -> bool {
        match self {
            Self::Terminate | Self::Kill => true,
            Self::Stop | Self::Continue => !is_windows(os),
        }
    }

    /// Whether `Terminate` is a graceful request on `os`.
    ///
    /// On Windows both `Terminate` and `Kill` become `taskkill /F`, so the
    /// "終了 (SIGTERM)" label alone would imply a graceful shutdown that the
    /// backend cannot perform (R2X-P4). The label stays stable for DOM hooks;
    /// shells may use this to show an extra warning.
    pub fn terminate_is_graceful_on(os: &str) -> bool {
        !is_windows(os)
    }
}

/// Whether a platform label refers to Windows.
///
/// `tm-platform` passes `System::long_os_version()` (for example
/// "Windows 11 Pro") to [`PlatformInfo::native`]. A case-insensitive substring
/// check keeps the mapping in the core, so every shell sees the same answer.
fn is_windows(os: &str) -> bool {
    os.to_ascii_lowercase().contains("windows")
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Error returned while sampling the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// The backend cannot sample this platform.
    Unsupported(String),
    /// The operating system returned an I/O error.
    Io(String),
    /// Reading the data requires privileges the process does not have.
    Permission(String),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(message) => write!(f, "未対応: {message}"),
            Self::Io(message) => write!(f, "読み取り失敗: {message}"),
            Self::Permission(message) => write!(f, "権限不足: {message}"),
        }
    }
}

impl std::error::Error for ProbeError {}

/// Error returned while controlling a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlError {
    /// The process disappeared before the signal was sent.
    NotFound,
    /// The OS refused the signal (EPERM).
    PermissionDenied,
    /// The request was rejected by a safety rule (PID 1, the app itself, ...).
    Protected,
    /// The backend is read-only (browser demo mode).
    Unsupported,
    /// Any other OS failure.
    Failed(String),
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("プロセスが見つかりません"),
            Self::PermissionDenied => f.write_str("権限がありません"),
            Self::Protected => f.write_str("保護対象のプロセスは操作できません"),
            Self::Unsupported => f.write_str("この環境ではプロセス操作は無効です"),
            Self::Failed(message) => write!(f, "操作に失敗しました: {message}"),
        }
    }
}

impl std::error::Error for ControlError {}

/// Source of system snapshots.
///
/// Implementations must be `Send` because the native shell samples on a worker
/// thread. The browser shell uses the same trait with a demo/replay source.
pub trait SystemProbe: Send {
    /// Describes this backend.
    fn platform_info(&self) -> PlatformInfo;

    /// Takes one sample.
    fn sample(&mut self) -> Result<SystemSnapshot, ProbeError>;

    /// Sends a signal to a process identified by PID **and** start time.
    ///
    /// The default implementation is read-only. Native backends override it and
    /// must re-check the start time immediately before signalling, so a reused
    /// PID can never be killed by mistake.
    fn control(&mut self, key: ProcessKey, signal: Signal) -> Result<(), ControlError> {
        let _ = (key, signal);
        Err(ControlError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_platform_info_supports_control() {
        let info = PlatformInfo::native("Linux");
        assert!(info.supports_process_control);
        assert!(!info.is_demo);
        assert_eq!(info.label(), "Linux");
    }

    #[test]
    fn demo_platform_info_is_read_only() {
        let info = PlatformInfo::demo();
        assert!(!info.supports_process_control);
        assert!(info.is_demo);
        assert!(info.label().contains("デモ"));
    }

    #[test]
    fn read_only_native_platform_info_cannot_signal() {
        // The browser shell reads snapshots over HTTP, so it must never offer
        // process control even though the host is a native Linux machine.
        let info = PlatformInfo::native_read_only("Linux (エージェント経由)");
        assert!(!info.supports_process_control);
        assert!(!info.is_demo);
        assert!(info.supports_network);
        assert!(info.supports_disks);
        assert_eq!(info.label(), "Linux (エージェント経由)");
    }

    #[test]
    fn section_availability_follows_the_capability_flags() {
        let native = PlatformInfo::native("Linux");
        for section in [Section::Network, Section::Disks, Section::ProcessControl] {
            assert!(native.supports(section), "{section:?}");
        }

        // The browser shells read snapshots but can never signal processes.
        for info in [
            PlatformInfo::native_read_only("Linux (エージェント経由)"),
            PlatformInfo::demo(),
        ] {
            assert!(info.supports(Section::Network));
            assert!(info.supports(Section::Disks));
            assert!(!info.supports(Section::ProcessControl));
        }
    }

    #[test]
    fn section_availability_is_independent_per_flag() {
        // Gating one section must not silently gate the others (R5-I04).
        let mut info = PlatformInfo::native("Linux");
        info.supports_network = false;
        assert!(!info.supports(Section::Network));
        assert!(info.supports(Section::Disks));
        assert!(info.supports(Section::ProcessControl));

        info.supports_disks = false;
        assert!(!info.supports(Section::Network));
        assert!(!info.supports(Section::Disks));
        assert!(info.supports(Section::ProcessControl));

        info.supports_process_control = false;
        assert!(!info.supports(Section::Network));
        assert!(!info.supports(Section::Disks));
        assert!(!info.supports(Section::ProcessControl));
    }

    #[test]
    fn every_signal_has_stable_labels() {
        // The confirmation dialog shows these strings; pin them for all
        // variants so a rename cannot silently change what the user confirms.
        let cases = [
            (Signal::Terminate, "終了 (SIGTERM)", "終了"),
            (Signal::Kill, "強制終了 (SIGKILL)", "強制終了"),
            (Signal::Stop, "一時停止 (SIGSTOP)", "一時停止"),
            (Signal::Continue, "再開 (SIGCONT)", "再開"),
        ];
        for (signal, label, short) in cases {
            assert_eq!(signal.label(), label);
            assert_eq!(signal.short_label(), short);
        }
    }

    #[test]
    fn destructive_signal_is_only_kill() {
        assert!(Signal::Kill.is_destructive());
        assert!(!Signal::Terminate.is_destructive());
    }

    #[test]
    fn windows_native_platform_info_gates_stop_and_continue() {
        // R2X-P2: Windows can terminate via `taskkill /F` but has no POSIX
        // SIGSTOP / SIGCONT, so only those two signals must be hidden/disabled.
        let info = PlatformInfo::native("Windows 11 Pro");
        assert!(info.supports(Section::ProcessControl));
        assert!(info.supports_signal(Signal::Terminate));
        assert!(info.supports_signal(Signal::Kill));
        assert!(!info.supports_signal(Signal::Stop));
        assert!(!info.supports_signal(Signal::Continue));

        // Mixed case and short labels must reach the same answer.
        for label in ["WINDOWS 10", "windows", "Windows Server 2022"] {
            let info = PlatformInfo::native(label);
            assert!(!info.supports_signal(Signal::Stop), "{label}");
            assert!(!info.supports_signal(Signal::Continue), "{label}");
        }
    }

    #[test]
    fn non_windows_native_platform_info_supports_every_signal() {
        let info = PlatformInfo::native("Linux");
        for signal in [
            Signal::Terminate,
            Signal::Kill,
            Signal::Stop,
            Signal::Continue,
        ] {
            assert!(info.supports_signal(signal), "{signal:?}");
        }
    }

    #[test]
    fn read_only_and_demo_backends_refuse_every_signal() {
        // R5-I04: section capability and per-signal capability must agree when
        // the whole backend is read-only.
        for info in [
            PlatformInfo::native_read_only("Linux (エージェント経由)"),
            PlatformInfo::demo(),
        ] {
            for signal in [
                Signal::Terminate,
                Signal::Kill,
                Signal::Stop,
                Signal::Continue,
            ] {
                assert!(!info.supports_signal(signal), "{signal:?}");
            }
        }
    }

    #[test]
    fn per_signal_capability_does_not_change_other_sections() {
        // Gating Stop/Continue on Windows must not hide network/disk sections.
        let info = PlatformInfo::native("Windows 11 Pro");
        assert!(info.supports(Section::Network));
        assert!(info.supports(Section::Disks));
        assert!(info.supports(Section::ProcessControl));
        assert!(!info.supports_signal(Signal::Stop));
    }

    #[test]
    fn windows_terminate_is_reported_as_forceful() {
        // R2X-P4: the stable label says SIGTERM, but Windows really runs
        // `taskkill /F`; shells show an extra warning based on this flag.
        assert!(Signal::terminate_is_graceful_on("Linux"));
        assert!(!Signal::terminate_is_graceful_on("Windows 11 Pro"));
        assert!(!Signal::terminate_is_graceful_on("WINDOWS 10"));
    }
}
