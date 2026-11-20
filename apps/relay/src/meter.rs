//! What a request cost, counted by the thing that spent it.
//!
//! Realizes the measurement in /implementation/relay-service.md. `deployment.md` deferred
//! the lobby projection until something measured asked for it; this is the something.
//!
//! Two numbers decide: the p99 of `fold_millis`, and `events_read` for a single lobby
//! view. The rest are here so that when one of them fires, the answer to *why* is a
//! column rather than an investigation.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A shared tally. Cloning shares the counters, which is how the handler reads what the
/// storage spent after handing it to a blocking thread and getting it back.
#[derive(Clone, Default)]
pub struct Meter {
    counted: Arc<Counted>,
}

#[derive(Default)]
struct Counted {
    reads: AtomicU64,
    events: AtomicU64,
    scans: AtomicU64,
    scan_nanos: AtomicU64,
    /// Thousandths of a capacity unit. DynamoDB reports capacity as a float and there is
    /// no atomic float; a thousandth of a unit is four bytes of a read, which is well
    /// below anything worth knowing.
    capacity_thousandths: AtomicU64,
}

impl Meter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(&self, events: usize) {
        self.counted.reads.fetch_add(1, Ordering::Relaxed);
        self.counted
            .events
            .fetch_add(events as u64, Ordering::Relaxed);
    }

    pub fn scan(&self, elapsed: std::time::Duration) {
        self.counted.scans.fetch_add(1, Ordering::Relaxed);
        self.counted
            .scan_nanos
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    }

    pub fn capacity(&self, units: f64) {
        self.counted
            .capacity_thousandths
            .fetch_add((units.max(0.0) * 1_000.0).round() as u64, Ordering::Relaxed);
    }

    /// How many times the log was read.
    ///
    /// On a lobby view that is the number of matches folded, one read each. On a message
    /// it is the number of appends, because the relay reads to find the end of the log
    /// before each one — which is why the field is not called `matches_folded`. The first
    /// message the deployed relay ever served reported three, for one match.
    pub fn reads(&self) -> u64 {
        self.counted.reads.load(Ordering::Relaxed)
    }

    /// The work. Independent of how fast the hardware was that day, which is why it is
    /// the number that triggers the projection rather than a duration alone.
    pub fn events(&self) -> u64 {
        self.counted.events.load(Ordering::Relaxed)
    }

    /// Finding out which matches exist. Grows with every event ever written, so it is
    /// reported apart from the fold: no projection of open matches would shorten it.
    pub fn scan_millis(&self) -> u64 {
        self.counted.scan_nanos.load(Ordering::Relaxed) / 1_000_000
    }

    pub fn scans(&self) -> u64 {
        self.counted.scans.load(Ordering::Relaxed)
    }

    pub fn capacity_units(&self) -> f64 {
        self.counted.capacity_thousandths.load(Ordering::Relaxed) as f64 / 1_000.0
    }

    /// One CloudWatch Embedded Metric Format line.
    ///
    /// Structured JSON on stdout costs a log line and no `PutMetricData` on the hot path,
    /// and CloudWatch turns it into a metric with percentiles. So the thresholds in
    /// /implementation/relay-service.md can be alarms rather than a habit of looking.
    ///
    /// `total` is the whole call, measured by the caller; `fold_millis` is what is left
    /// of it after the scan, which is the part a projection would remove.
    pub fn emf(
        &self,
        namespace: &str,
        route: &str,
        total: std::time::Duration,
        now: protocol::Moment,
    ) -> String {
        let total_millis = total.as_millis() as u64;
        let scan_millis = self.scan_millis();
        let fold_millis = total_millis.saturating_sub(scan_millis);
        let metrics = [
            "total_millis",
            "scan_millis",
            "fold_millis",
            "log_reads",
            "events_read",
            "read_capacity",
        ]
        .map(|name| format!("{{\"Name\":\"{name}\",\"Unit\":\"None\"}}"))
        .join(",");
        format!(
            concat!(
                "{{\"_aws\":{{\"Timestamp\":{timestamp},\"CloudWatchMetrics\":[{{",
                "\"Namespace\":\"{namespace}\",\"Dimensions\":[[\"route\"]],\"Metrics\":[{metrics}]",
                "}}]}},\"route\":\"{route}\",\"total_millis\":{total},\"scan_millis\":{scan},",
                "\"fold_millis\":{fold},\"log_reads\":{reads},\"events_read\":{events},",
                "\"read_capacity\":{capacity}}}"
            ),
            timestamp = now.to_unix_nanos() / 1_000_000,
            namespace = namespace,
            metrics = metrics,
            route = route,
            total = total_millis,
            scan = scan_millis,
            fold = fold_millis,
            reads = self.reads(),
            events = self.events(),
            capacity = self.capacity_units(),
        )
    }
}
