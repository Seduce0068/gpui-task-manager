//! Derived metrics: CPU deltas, network rates and bounded history buffers.
//!
//! The platform adapter owns the raw counters; this module turns them into the
//! numbers a task manager actually displays. Everything here is pure and is
//! covered by deterministic unit tests, which is the main reason the platform
//! layer can stay thin.

use std::collections::VecDeque;
use std::time::Duration;

/// Raw CPU time counters for one core (Linux `/proc/stat` semantics).
///
/// `total` is the sum of all time slices and `busy` is `total - idle - iowait`.
/// The adapter fills these from its platform source; the view-model never sees
/// them, it only sees percentages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuCounters {
    /// Non-idle time slices.
    pub busy: u64,
    /// All time slices.
    pub total: u64,
}

impl CpuCounters {
    /// Creates counters from already aggregated values.
    ///
    /// `busy` is clamped to `total` so malformed platform data cannot produce
    /// percentages above 100.
    pub fn new(busy: u64, total: u64) -> Self {
        Self {
            busy: busy.min(total),
            total,
        }
    }

    /// Builds counters from the eight `/proc/stat` time fields.
    #[allow(clippy::too_many_arguments)]
    pub fn from_linux_times(
        user: u64,
        nice: u64,
        system: u64,
        idle: u64,
        iowait: u64,
        irq: u64,
        softirq: u64,
        steal: u64,
    ) -> Self {
        let total = user
            .saturating_add(nice)
            .saturating_add(system)
            .saturating_add(idle)
            .saturating_add(iowait)
            .saturating_add(irq)
            .saturating_add(softirq)
            .saturating_add(steal);
        let idle_total = idle.saturating_add(iowait);
        Self::new(total.saturating_sub(idle_total), total)
    }

    /// Percentage of CPU time spent busy between `previous` and `self`.
    ///
    /// Returns `0.0` when no time elapsed or when the counters went backwards
    /// (which can happen on CPU hotplug or when a container hides counters).
    pub fn percent_since(self, previous: Self) -> f32 {
        let total_delta = self.total.saturating_sub(previous.total);
        if total_delta == 0 {
            return 0.0;
        }
        let busy_delta = self.busy.saturating_sub(previous.busy);
        ((busy_delta as f64 / total_delta as f64) * 100.0).clamp(0.0, 100.0) as f32
    }
}

/// Cumulative network byte counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetworkCounters {
    /// Bytes received since boot.
    pub received: u64,
    /// Bytes transmitted since boot.
    pub transmitted: u64,
}

impl NetworkCounters {
    /// Creates counters from the two totals.
    pub const fn new(received: u64, transmitted: u64) -> Self {
        Self {
            received,
            transmitted,
        }
    }

    /// Minimum elapsed interval for which a rate is reported (1 ms).
    ///
    /// Shorter gaps are treated as a bad timestamp: dividing a large byte delta
    /// by a few nanoseconds would saturate the `u64` rate and draw a bogus spike.
    /// The sampler ticks once per second, so a legitimate sample is never this
    /// close to the previous one.
    const MIN_ELAPSED: Duration = Duration::from_millis(1);

    /// Computes `(rx, tx)` bytes-per-second rates since `previous`.
    ///
    /// The result is saturating and returns zeros for a zero, non-finite, or
    /// sub-millisecond elapsed time, so a bad timestamp can never produce a
    /// wild spike.
    pub fn rate_since(self, previous: Self, elapsed: Duration) -> (u64, u64) {
        if elapsed < Self::MIN_ELAPSED {
            return (0, 0);
        }
        let seconds = elapsed.as_secs_f64();
        if !seconds.is_finite() {
            return (0, 0);
        }
        let rx = self.received.saturating_sub(previous.received);
        let tx = self.transmitted.saturating_sub(previous.transmitted);
        (
            (rx as f64 / seconds).round() as u64,
            (tx as f64 / seconds).round() as u64,
        )
    }
}

/// Fixed-capacity ring buffer that keeps the newest `capacity` values.
///
/// The history is bounded on purpose: a task manager that runs for days must
/// not grow its memory without bound. Charts iterate from oldest to newest.
#[derive(Clone, Debug)]
pub struct RingBuffer<T> {
    items: VecDeque<T>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    /// Creates a buffer with the given capacity (at least one).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            items: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Pushes a value, dropping the oldest one when full.
    pub fn push(&mut self, value: T) {
        if self.items.len() == self.capacity {
            self.items.pop_front();
        }
        self.items.push_back(value);
    }

    /// Iterates from oldest to newest.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        self.items.iter()
    }

    /// Returns the newest value, if any.
    pub fn latest(&self) -> Option<&T> {
        self.items.back()
    }

    /// Returns the oldest value, if any.
    pub fn earliest(&self) -> Option<&T> {
        self.items.front()
    }

    /// Number of stored values.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` when no value has been stored yet.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Maximum number of stored values.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Removes every stored value.
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

impl<T> Default for RingBuffer<T> {
    fn default() -> Self {
        Self::new(120)
    }
}

impl<T: Clone> RingBuffer<T> {
    /// Copies the stored values, oldest first.
    pub fn to_vec(&self) -> Vec<T> {
        self.items.iter().cloned().collect()
    }
}

/// Chart history for the overview tab.
///
/// One push per sample keeps CPU, memory and network aligned, so the charts can
/// share an X axis without extra bookkeeping.
#[derive(Clone, Debug)]
pub struct MetricHistory {
    /// Machine-wide CPU utilisation in percent.
    pub cpu_percent: RingBuffer<f32>,
    /// RAM utilisation in percent.
    pub memory_percent: RingBuffer<f32>,
    /// Network receive rate in bytes per second.
    pub network_rx_rate: RingBuffer<u64>,
    /// Network transmit rate in bytes per second.
    pub network_tx_rate: RingBuffer<u64>,
}

impl MetricHistory {
    /// Creates a history with the same capacity for every series.
    pub fn new(capacity: usize) -> Self {
        Self {
            cpu_percent: RingBuffer::new(capacity),
            memory_percent: RingBuffer::new(capacity),
            network_rx_rate: RingBuffer::new(capacity),
            network_tx_rate: RingBuffer::new(capacity),
        }
    }

    /// Appends one aligned sample.
    ///
    /// Percentages are clamped to 0..=100; non-finite values (which `clamp`
    /// would pass through as `NaN`) become zero so charts never see `NaN`.
    pub fn push(&mut self, cpu_percent: f32, memory_percent: f32, rx_rate: u64, tx_rate: u64) {
        let clamp = |value: f32| {
            if value.is_finite() {
                value.clamp(0.0, 100.0)
            } else {
                0.0
            }
        };
        self.cpu_percent.push(clamp(cpu_percent));
        self.memory_percent.push(clamp(memory_percent));
        self.network_rx_rate.push(rx_rate);
        self.network_tx_rate.push(tx_rate);
    }

    /// Number of samples stored.
    pub fn len(&self) -> usize {
        self.cpu_percent.len()
    }

    /// Returns `true` when no sample has been stored yet.
    pub fn is_empty(&self) -> bool {
        self.cpu_percent.is_empty()
    }

    /// Removes all samples.
    pub fn clear(&mut self) {
        self.cpu_percent.clear();
        self.memory_percent.clear();
        self.network_rx_rate.clear();
        self.network_tx_rate.clear();
    }
}

impl Default for MetricHistory {
    fn default() -> Self {
        Self::new(120)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percent_is_computed_from_deltas() {
        let previous = CpuCounters::new(100, 1_000);
        let current = CpuCounters::new(200, 1_200);
        assert_eq!(current.percent_since(previous), 50.0);
    }

    #[test]
    fn cpu_percent_handles_stalled_counters() {
        let counters = CpuCounters::new(10, 20);
        assert_eq!(counters.percent_since(counters), 0.0);
    }

    #[test]
    fn cpu_percent_handles_backwards_counters() {
        let previous = CpuCounters::new(500, 1_000);
        let current = CpuCounters::new(100, 500);
        assert_eq!(current.percent_since(previous), 0.0);
    }

    #[test]
    fn cpu_percent_is_clamped() {
        let previous = CpuCounters::new(0, 0);
        let current = CpuCounters::new(200, 100);
        assert!(current.percent_since(previous) <= 100.0);
    }

    #[test]
    fn linux_times_busy_math() {
        // user nice system idle iowait irq softirq steal
        let counters = CpuCounters::from_linux_times(10, 0, 10, 60, 10, 5, 5, 0);
        assert_eq!(counters.total, 100);
        assert_eq!(counters.busy, 30);
    }

    #[test]
    fn network_rate_uses_elapsed_seconds() {
        let previous = NetworkCounters::new(1_000, 2_000);
        let current = NetworkCounters::new(3_000, 2_000);
        let (rx, tx) = current.rate_since(previous, Duration::from_secs(2));
        assert_eq!(rx, 1_000);
        assert_eq!(tx, 0);
    }

    #[test]
    fn network_rate_handles_backwards_counters() {
        // An interface reset or counter wrap can make totals go backwards;
        // saturating_sub must yield zero, never a huge bogus rate.
        let previous = NetworkCounters::new(5_000, 9_000);
        let current = NetworkCounters::new(1_000, 2_000);
        let (rx, tx) = current.rate_since(previous, Duration::from_secs(2));
        assert_eq!((rx, tx), (0, 0));
    }

    #[test]
    fn network_rate_handles_zero_elapsed() {
        let previous = NetworkCounters::new(1, 1);
        let current = NetworkCounters::new(2, 2);
        assert_eq!(current.rate_since(previous, Duration::ZERO), (0, 0));
    }

    #[test]
    fn network_rate_ignores_sub_millisecond_gaps() {
        // A bogus 1 ns timestamp with a large byte delta would saturate u64.
        let previous = NetworkCounters::new(0, 0);
        let current = NetworkCounters::new(u64::MAX, u64::MAX);
        assert_eq!(
            current.rate_since(previous, Duration::from_nanos(1)),
            (0, 0)
        );
        // At or above the lower bound the rate is computed normally.
        let small = NetworkCounters::new(1_000, 2_000);
        assert_eq!(
            small.rate_since(NetworkCounters::new(0, 0), Duration::from_millis(1)),
            (1_000_000, 2_000_000)
        );
    }

    #[test]
    fn ring_buffer_drops_oldest_values() {
        let mut buffer = RingBuffer::new(3);
        for value in 1..=5 {
            buffer.push(value);
        }
        assert_eq!(buffer.to_vec(), vec![3, 4, 5]);
        assert_eq!(buffer.latest(), Some(&5));
        assert_eq!(buffer.earliest(), Some(&3));
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn ring_buffer_capacity_is_at_least_one() {
        let mut buffer = RingBuffer::new(0);
        buffer.push(1);
        buffer.push(2);
        assert_eq!(buffer.to_vec(), vec![2]);
        assert_eq!(buffer.capacity(), 1);
    }

    #[test]
    fn history_clamps_percentages() {
        let mut history = MetricHistory::new(4);
        history.push(150.0, -5.0, 10, 20);
        assert_eq!(history.cpu_percent.latest(), Some(&100.0));
        assert_eq!(history.memory_percent.latest(), Some(&0.0));
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn history_rejects_non_finite_percentages() {
        let mut history = MetricHistory::new(4);
        history.push(f32::NAN, f32::INFINITY, 1, 2);
        assert_eq!(history.cpu_percent.latest(), Some(&0.0));
        assert_eq!(history.memory_percent.latest(), Some(&0.0));
        assert!(
            history.cpu_percent.iter().all(|value| value.is_finite()),
            "no NaN may enter the chart history"
        );
    }
}
