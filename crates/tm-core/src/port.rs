//! Ports: the boundary between the pure core and the operating system.
//!
//! The UI depends on these traits, never on `sysinfo`, `procfs`, `rustix` or
//! any other platform crate. That keeps the shells testable with a deterministic
//! demo source and keeps the wasm build free of native dependencies.

use std::fmt;

use crate::model::{ProcessKey, SystemSnapshot};

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
    /// `supports_network` / `supports_disks` describe what the *host* provides,
    /// not whether this particular probe managed to enumerate them: on Linux
    /// both are expected, and an enumeration failure is reported through the
    /// (empty) snapshot data instead of a wrong capability flag. Callers that
    /// need to know whether data is present must look at the snapshot.
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
}
