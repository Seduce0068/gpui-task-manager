//! Sorting and filtering for the process table.
//!
//! The UI keeps the process list immutable and works with indices, so sorting
//! and filtering never clone process rows. The view-model caches the resulting
//! index vector and recomputes it only when the snapshot, the sort spec or the
//! filter actually changes.

use std::cmp::Ordering;

use crate::model::{ProcessSnapshot, ProcessStatus};

/// Column the process table is sorted by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SortKey {
    /// CPU usage.
    #[default]
    Cpu,
    /// Resident memory.
    Memory,
    /// Numeric PID.
    Pid,
    /// Process name (case-insensitive).
    Name,
    /// Owning user (case-insensitive).
    User,
    /// Scheduler state.
    Status,
    /// Thread count.
    Threads,
    /// Process start time.
    StartTime,
}

impl SortKey {
    /// Japanese column label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "メモリ",
            Self::Pid => "PID",
            Self::Name => "名前",
            Self::User => "ユーザー",
            Self::Status => "状態",
            Self::Threads => "スレッド",
            Self::StartTime => "起動時刻",
        }
    }

    /// Direction that makes sense as the first click for this column.
    pub const fn default_direction(self) -> SortDirection {
        match self {
            Self::Cpu | Self::Memory => SortDirection::Descending,
            _ => SortDirection::Ascending,
        }
    }
}

/// Sort direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SortDirection {
    /// Smallest first.
    #[default]
    Ascending,
    /// Largest first.
    Descending,
}

impl SortDirection {
    /// Returns the opposite direction.
    pub const fn flipped(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }

    /// Arrow glyph for headers.
    pub const fn arrow(self) -> &'static str {
        match self {
            Self::Ascending => "▲",
            Self::Descending => "▼",
        }
    }
}

/// Which column and direction the table is sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SortSpec {
    /// Active column.
    pub key: SortKey,
    /// Active direction.
    pub direction: SortDirection,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            key: SortKey::Cpu,
            direction: SortDirection::Descending,
        }
    }
}

impl SortSpec {
    /// Builds a spec using the column's default direction.
    pub fn new(key: SortKey) -> Self {
        Self {
            key,
            direction: key.default_direction(),
        }
    }

    /// Click behaviour: same column flips direction, another column resets to
    /// that column's default direction.
    pub fn toggle(&mut self, key: SortKey) {
        if self.key == key {
            self.direction = self.direction.flipped();
        } else {
            *self = Self::new(key);
        }
    }
}

/// Process table filter.
///
/// `text` is a whitespace-separated AND query matched against name, command
/// line, user name and PID, case-insensitively. Empty fields mean "no filter".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    /// Free-text query.
    pub text: String,
    /// Exact user name (case-insensitive).
    pub user: Option<String>,
    /// Exact scheduler state.
    pub status: Option<ProcessStatus>,
    /// Minimum CPU percentage.
    pub min_cpu_percent: Option<f32>,
    /// Minimum resident memory in bytes.
    pub min_memory_bytes: Option<u64>,
}

impl Filter {
    /// Returns `true` when any filter field is set.
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty()
            || self.user.is_some()
            || self.status.is_some()
            || self.min_cpu_percent.is_some()
            || self.min_memory_bytes.is_some()
    }

    /// Returns `true` when the process passes every active filter.
    pub fn matches(&self, process: &ProcessSnapshot) -> bool {
        if let Some(min_cpu) = self.min_cpu_percent {
            if process.cpu_percent < min_cpu {
                return false;
            }
        }
        if let Some(min_memory) = self.min_memory_bytes {
            if process.memory_bytes < min_memory {
                return false;
            }
        }
        if let Some(status) = self.status {
            if process.status != status {
                return false;
            }
        }
        if let Some(user) = &self.user {
            if !process
                .user
                .as_deref()
                .is_some_and(|process_user| eq_ignore_case(process_user, user))
            {
                return false;
            }
        }

        let query = self.text.trim();
        if query.is_empty() {
            return true;
        }
        let pid = process.key.pid.to_string();
        query.split_whitespace().all(|token| {
            contains_ignore_case(&process.name, token)
                || contains_ignore_case(&process.command, token)
                || process
                    .user
                    .as_deref()
                    .is_some_and(|user| contains_ignore_case(user, token))
                || pid.contains(token)
        })
    }
}

/// Filters and sorts a process list, returning indices into `processes`.
///
/// The tie-breaker is always the numeric PID ascending, which makes the table
/// order deterministic even when many rows share the same CPU percentage.
pub fn filter_and_sort(
    processes: &[ProcessSnapshot],
    filter: &Filter,
    sort: SortSpec,
) -> Vec<usize> {
    let mut indices: Vec<usize> = processes
        .iter()
        .enumerate()
        .filter(|(_, process)| filter.matches(process))
        .map(|(index, _)| index)
        .collect();

    indices.sort_by(|&left, &right| {
        let ordering = compare(&processes[left], &processes[right], sort.key);
        let ordering = match sort.direction {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        };
        ordering.then_with(|| processes[left].key.pid.cmp(&processes[right].key.pid))
    });
    indices
}

fn compare(left: &ProcessSnapshot, right: &ProcessSnapshot, key: SortKey) -> Ordering {
    match key {
        SortKey::Cpu => left.cpu_percent.total_cmp(&right.cpu_percent),
        SortKey::Memory => left.memory_bytes.cmp(&right.memory_bytes),
        SortKey::Pid => left.key.pid.cmp(&right.key.pid),
        SortKey::Name => cmp_ignore_case(&left.name, &right.name),
        SortKey::User => cmp_ignore_case(
            left.user.as_deref().unwrap_or(""),
            right.user.as_deref().unwrap_or(""),
        ),
        SortKey::Status => left.status.id().cmp(right.status.id()),
        SortKey::Threads => left.threads.unwrap_or(0).cmp(&right.threads.unwrap_or(0)),
        SortKey::StartTime => left.key.start_time_secs.cmp(&right.key.start_time_secs),
    }
}

fn cmp_ignore_case(left: &str, right: &str) -> Ordering {
    let mut left_chars = left.chars().flat_map(char::to_lowercase);
    let mut right_chars = right.chars().flat_map(char::to_lowercase);
    loop {
        match (left_chars.next(), right_chars.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_char), Some(right_char)) => match left_char.cmp(&right_char) {
                Ordering::Equal => {}
                other => return other,
            },
        }
    }
}

fn eq_ignore_case(left: &str, right: &str) -> bool {
    cmp_ignore_case(left, right) == Ordering::Equal
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// A row in a flattened process tree view.
///
/// `index` points into the original process slice. `depth` is 0 for root
/// processes (no parent present in the selected snapshot subset).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessTreeRow {
    /// Index into the source process slice.
    pub index: usize,
    /// Nesting depth from a root process.
    pub depth: usize,
    /// Whether this process has child processes in the flattened tree.
    pub has_children: bool,
}

/// Flattens selected process indices into depth-first tree order.
///
/// Parent-child relationships are resolved by matching [`ProcessSnapshot::parent_pid`]
/// to a PID present among `indices` in `processes`. If a parent is missing from
/// the selected subset, the process becomes a root. Sibling order preserves the
/// order of `indices` (typically the result of [`filter_and_sort`]).
pub fn flatten_process_tree(
    processes: &[ProcessSnapshot],
    indices: &[usize],
) -> Vec<ProcessTreeRow> {
    use std::collections::{HashMap, HashSet};

    let selected: HashSet<usize> = indices.iter().copied().collect();
    let pid_to_index: HashMap<u32, usize> = indices
        .iter()
        .copied()
        .filter_map(|idx| processes.get(idx).map(|p| (p.key.pid, idx)))
        .collect();

    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();

    for &idx in indices {
        let Some(process) = processes.get(idx) else {
            continue;
        };
        let parent_idx = process
            .parent_pid
            .and_then(|ppid| pid_to_index.get(&ppid).copied())
            .filter(|&pidx| pidx != idx && selected.contains(&pidx));
        match parent_idx {
            Some(pidx) => children.entry(pidx).or_default().push(idx),
            None => roots.push(idx),
        }
    }

    fn walk(
        idx: usize,
        depth: usize,
        children: &std::collections::HashMap<usize, Vec<usize>>,
        out: &mut Vec<ProcessTreeRow>,
    ) {
        let child_list = children.get(&idx);
        out.push(ProcessTreeRow {
            index: idx,
            depth,
            has_children: child_list.is_some_and(|c| !c.is_empty()),
        });
        if let Some(child_list) = child_list {
            for &child in child_list {
                walk(child, depth + 1, children, out);
            }
        }
    }

    let mut out = Vec::with_capacity(indices.len());
    for root in roots {
        walk(root, 0, &children, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProcessKey;

    fn process(pid: u32, name: &str, cpu: f32, memory: u64) -> ProcessSnapshot {
        ProcessSnapshot {
            key: ProcessKey::new(pid, 1_000 + pid as u64),
            name: name.to_string(),
            command: format!("/usr/bin/{name} --flag"),
            user: Some("user".to_string()),
            status: ProcessStatus::Sleeping,
            cpu_percent: cpu,
            memory_bytes: memory,
            threads: Some(4),
            ..ProcessSnapshot::default()
        }
    }

    fn sample() -> Vec<ProcessSnapshot> {
        vec![
            process(300, "firefox", 12.5, 1_000),
            process(100, "systemd", 0.1, 4_000),
            process(200, "code", 30.0, 2_000),
        ]
    }

    #[test]
    fn default_sort_is_cpu_descending() {
        let sorted = filter_and_sort(&sample(), &Filter::default(), SortSpec::default());
        assert_eq!(sorted, vec![2, 0, 1]);
    }

    #[test]
    fn toggle_same_column_flips_direction() {
        let mut spec = SortSpec::default();
        spec.toggle(SortKey::Cpu);
        assert_eq!(spec.direction, SortDirection::Ascending);
        spec.toggle(SortKey::Cpu);
        assert_eq!(spec.direction, SortDirection::Descending);
    }

    #[test]
    fn toggle_other_column_uses_its_default() {
        let mut spec = SortSpec::default();
        spec.toggle(SortKey::Pid);
        assert_eq!(spec.key, SortKey::Pid);
        assert_eq!(spec.direction, SortDirection::Ascending);
    }

    #[test]
    fn ties_are_broken_by_pid() {
        let processes = vec![process(9, "b", 5.0, 1), process(2, "a", 5.0, 1)];
        let sorted = filter_and_sort(&processes, &Filter::default(), SortSpec::default());
        assert_eq!(sorted, vec![1, 0]);
    }

    #[test]
    fn name_sort_is_case_insensitive() {
        let processes = vec![process(1, "Zsh", 0.0, 1), process(2, "bash", 0.0, 1)];
        let spec = SortSpec::new(SortKey::Name);
        let sorted = filter_and_sort(&processes, &Filter::default(), spec);
        assert_eq!(sorted, vec![1, 0]);
    }

    #[test]
    fn text_filter_matches_name_command_user_and_pid() {
        let processes = sample();
        for query in ["fire", "/usr/bin/code", "user", "100"] {
            let filter = Filter {
                text: query.to_string(),
                ..Filter::default()
            };
            let sorted = filter_and_sort(&processes, &filter, SortSpec::default());
            assert!(!sorted.is_empty(), "query {query} should match");
        }
    }

    #[test]
    fn text_filter_requires_every_token() {
        let filter = Filter {
            text: "firefox code".to_string(),
            ..Filter::default()
        };
        assert!(filter_and_sort(&sample(), &filter, SortSpec::default()).is_empty());
    }

    #[test]
    fn text_filter_is_case_insensitive() {
        let filter = Filter {
            text: "FIREFOX".to_string(),
            ..Filter::default()
        };
        assert_eq!(
            filter_and_sort(&sample(), &filter, SortSpec::default()).len(),
            1
        );
    }

    #[test]
    fn threshold_filters_are_inclusive() {
        let filter = Filter {
            min_cpu_percent: Some(12.5),
            min_memory_bytes: Some(1_000),
            ..Filter::default()
        };
        let sorted = filter_and_sort(&sample(), &filter, SortSpec::default());
        assert_eq!(sorted, vec![2, 0]);
    }

    #[test]
    fn active_detection_ignores_blank_text() {
        assert!(!Filter::default().is_active());
        assert!(!Filter {
            text: "   ".to_string(),
            ..Filter::default()
        }
        .is_active());
        assert!(Filter {
            text: "x".to_string(),
            ..Filter::default()
        }
        .is_active());
    }

    #[test]
    fn flatten_process_tree_preserves_roots_and_children() {
        let mut parent = process(1, "init", 0.0, 1);
        parent.parent_pid = None;
        let mut child = process(2, "bash", 0.0, 1);
        child.parent_pid = Some(1);
        let mut grandchild = process(3, "vim", 0.0, 1);
        grandchild.parent_pid = Some(2);
        let mut other = process(4, "other", 0.0, 1);
        other.parent_pid = None;
        let processes = vec![parent, child, grandchild, other];
        let rows = flatten_process_tree(&processes, &[0, 1, 2, 3]);
        assert_eq!(
            rows,
            vec![
                ProcessTreeRow {
                    index: 0,
                    depth: 0,
                    has_children: true
                },
                ProcessTreeRow {
                    index: 1,
                    depth: 1,
                    has_children: true
                },
                ProcessTreeRow {
                    index: 2,
                    depth: 2,
                    has_children: false
                },
                ProcessTreeRow {
                    index: 3,
                    depth: 0,
                    has_children: false
                },
            ]
        );
    }

    #[test]
    fn flatten_process_tree_treats_missing_parent_as_root() {
        let mut child = process(10, "child", 0.0, 1);
        child.parent_pid = Some(999);
        let processes = vec![child];
        let rows = flatten_process_tree(&processes, &[0]);
        assert_eq!(
            rows,
            vec![ProcessTreeRow {
                index: 0,
                depth: 0,
                has_children: false
            }]
        );
    }

    #[test]
    fn flatten_process_tree_respects_selected_order_for_roots() {
        let mut a = process(1, "a", 0.0, 1);
        a.parent_pid = None;
        let mut b = process(2, "b", 0.0, 1);
        b.parent_pid = None;
        let processes = vec![a, b];
        let rows = flatten_process_tree(&processes, &[1, 0]);
        assert_eq!(rows.iter().map(|r| r.index).collect::<Vec<_>>(), vec![1, 0]);
    }
}
