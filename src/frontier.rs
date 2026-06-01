use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::urlcanon::canonicalize_url;

pub const DEFAULT_MAX_FAILED_FRONTIER_RECORDS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UrlState {
    Queued,
    Fetching,
    Fetched,
    Failed,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlRecord {
    pub url: String,
    pub host: String,
    pub state: UrlState,
    pub depth: u32,
    pub discovered_at: u64,
    pub updated_at: u64,
    pub next_fetch_at: u64,
    pub attempts: u32,
    pub last_status: Option<u16>,
    pub canonical_url: Option<String>,
    pub content_hash: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontierClaim {
    pub url: String,
    pub depth: u32,
    pub host: String,
    pub attempts: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontierStats {
    pub queued: usize,
    pub fetching: usize,
    pub fetched: usize,
    pub failed: usize,
    pub deferred: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostStats {
    pub host: String,
    pub queued: usize,
    pub fetching: usize,
    pub fetched: usize,
    pub failed: usize,
    pub deferred: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrontierFailure {
    pub url: String,
    pub host: String,
    pub reason: Option<String>,
    pub status_code: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecrawlPlanEntry {
    pub url: String,
    pub priority: i64,
    pub recrawl_after: u64,
    pub last_fetched_at: u64,
    pub age_secs: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FrontierSnapshot {
    records: BTreeMap<String, UrlRecord>,
}

#[derive(Debug)]
pub struct FrontierStore {
    path: PathBuf,
    snapshot: FrontierSnapshot,
}

impl FrontierStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let snapshot = if path.exists() {
            bincode::deserialize(
                &fs::read(&path).with_context(|| format!("read frontier {}", path.display()))?,
            )
            .with_context(|| format!("decode frontier {}", path.display()))?
        } else {
            FrontierSnapshot::default()
        };

        Ok(Self { path, snapshot })
    }

    pub fn len(&self) -> usize {
        self.snapshot.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshot.records.is_empty()
    }

    pub fn get(&self, url: &str) -> Option<&UrlRecord> {
        self.snapshot.records.get(url)
    }

    pub fn discover(&mut self, url: Url, depth: u32, now: u64) -> bool {
        let url = canonicalize_url(url);
        if !matches!(url.scheme(), "http" | "https") {
            return false;
        }

        let Some(host) = url.host_str().map(str::to_owned) else {
            return false;
        };

        let key = url.to_string();
        if let Some(existing) = self.snapshot.records.get_mut(&key) {
            existing.depth = existing.depth.min(depth);
            existing.updated_at = now;
            return false;
        }

        self.snapshot.records.insert(
            key.clone(),
            UrlRecord {
                url: key,
                host,
                state: UrlState::Queued,
                depth,
                discovered_at: now,
                updated_at: now,
                next_fetch_at: now,
                attempts: 0,
                last_status: None,
                canonical_url: None,
                content_hash: None,
                last_error: None,
            },
        );
        true
    }

    pub fn schedule_recrawl(&mut self, url: Url, depth: u32, now: u64) -> bool {
        let url = canonicalize_url(url);
        if !matches!(url.scheme(), "http" | "https") {
            return false;
        }

        let Some(host) = url.host_str().map(str::to_owned) else {
            return false;
        };

        let key = url.to_string();
        if let Some(existing) = self.snapshot.records.get_mut(&key) {
            existing.state = UrlState::Queued;
            existing.depth = depth;
            existing.updated_at = now;
            existing.next_fetch_at = now;
            existing.attempts = 0;
            existing.last_error = None;
            return true;
        }

        self.snapshot.records.insert(
            key.clone(),
            UrlRecord {
                url: key,
                host,
                state: UrlState::Queued,
                depth,
                discovered_at: now,
                updated_at: now,
                next_fetch_at: now,
                attempts: 0,
                last_status: None,
                canonical_url: None,
                content_hash: None,
                last_error: None,
            },
        );
        true
    }

    pub fn claim_next(&mut self, now: u64, max_fetching_per_host: usize) -> Option<FrontierClaim> {
        let active_by_host = self.active_by_host();
        let candidate = self
            .snapshot
            .records
            .iter()
            .filter(|(_, record)| {
                matches!(record.state, UrlState::Queued | UrlState::Deferred)
                    && record.next_fetch_at <= now
                    && active_by_host
                        .get(record.host.as_str())
                        .copied()
                        .unwrap_or(0)
                        < max_fetching_per_host
            })
            .min_by(|(_, left), (_, right)| {
                left.next_fetch_at
                    .cmp(&right.next_fetch_at)
                    .then_with(|| left.depth.cmp(&right.depth))
                    .then_with(|| left.discovered_at.cmp(&right.discovered_at))
                    .then_with(|| left.url.cmp(&right.url))
            })
            .map(|(url, _)| url.clone())?;

        let record = self.snapshot.records.get_mut(&candidate)?;
        record.state = UrlState::Fetching;
        record.updated_at = now;
        record.attempts += 1;

        Some(FrontierClaim {
            url: record.url.clone(),
            depth: record.depth,
            host: record.host.clone(),
            attempts: record.attempts,
        })
    }

    pub fn reset_fetching(&mut self, now: u64) -> usize {
        let mut reset = 0;
        for record in self.snapshot.records.values_mut() {
            if record.state == UrlState::Fetching {
                record.state = UrlState::Queued;
                record.updated_at = now;
                reset += 1;
            }
        }
        reset
    }

    pub fn record_fetched(
        &mut self,
        url: &str,
        status: u16,
        canonical_url: Option<String>,
        content_hash: Option<String>,
        now: u64,
    ) -> bool {
        let Some(record) = self.snapshot.records.get_mut(url) else {
            return false;
        };

        record.state = UrlState::Fetched;
        record.updated_at = now;
        record.last_status = Some(status);
        record.canonical_url = canonical_url;
        record.content_hash = content_hash;
        record.last_error = None;
        true
    }

    pub fn record_failed(
        &mut self,
        url: &str,
        error: String,
        retry_after_secs: u64,
        now: u64,
    ) -> bool {
        let Some(record) = self.snapshot.records.get_mut(url) else {
            return false;
        };

        record.state = if retry_after_secs == 0 {
            UrlState::Failed
        } else {
            UrlState::Deferred
        };
        record.updated_at = now;
        record.next_fetch_at = now.saturating_add(retry_after_secs);
        record.last_error = Some(error);
        true
    }

    pub fn release_claim(&mut self, url: &str, now: u64) -> bool {
        let Some(record) = self.snapshot.records.get_mut(url) else {
            return false;
        };
        if record.state != UrlState::Fetching {
            return false;
        }
        record.state = UrlState::Queued;
        record.updated_at = now;
        true
    }

    pub fn stats(&self) -> FrontierStats {
        let mut stats = FrontierStats::default();
        for record in self.snapshot.records.values() {
            match record.state {
                UrlState::Queued => stats.queued += 1,
                UrlState::Fetching => stats.fetching += 1,
                UrlState::Fetched => stats.fetched += 1,
                UrlState::Failed => stats.failed += 1,
                UrlState::Deferred => stats.deferred += 1,
            }
        }
        stats.total = self.snapshot.records.len();
        stats
    }

    pub fn host_stats(&self) -> Vec<HostStats> {
        let mut hosts = BTreeMap::<String, HostStats>::new();
        for record in self.snapshot.records.values() {
            let stats = hosts
                .entry(record.host.clone())
                .or_insert_with(|| HostStats {
                    host: record.host.clone(),
                    ..HostStats::default()
                });
            match record.state {
                UrlState::Queued => stats.queued += 1,
                UrlState::Fetching => stats.fetching += 1,
                UrlState::Fetched => stats.fetched += 1,
                UrlState::Failed => stats.failed += 1,
                UrlState::Deferred => stats.deferred += 1,
            }
            stats.total += 1;
        }

        hosts.into_values().collect()
    }

    pub fn failure_samples(&self, limit: usize) -> Vec<FrontierFailure> {
        let mut failures = self
            .snapshot
            .records
            .values()
            .filter(|record| record.state == UrlState::Failed)
            .map(|record| FrontierFailure {
                url: record.url.clone(),
                host: record.host.clone(),
                reason: record.last_error.clone(),
                status_code: record.last_status,
            })
            .collect::<Vec<_>>();
        failures.sort_by(|left, right| {
            left.host
                .cmp(&right.host)
                .then_with(|| left.url.cmp(&right.url))
        });
        if limit > 0 {
            failures.truncate(limit);
        }
        failures
    }

    pub fn compact_failed_records(&mut self, max_failed: usize) -> usize {
        let failed_count = self
            .snapshot
            .records
            .values()
            .filter(|record| record.state == UrlState::Failed)
            .count();
        if failed_count <= max_failed {
            return 0;
        }

        let remove_count = failed_count - max_failed;
        let mut failed = self
            .snapshot
            .records
            .values()
            .filter(|record| record.state == UrlState::Failed)
            .map(|record| (record.updated_at, record.url.clone()))
            .collect::<Vec<_>>();
        failed.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

        for (_, url) in failed.into_iter().take(remove_count) {
            self.snapshot.records.remove(&url);
        }
        remove_count
    }

    pub fn recrawl_plan(
        &self,
        now: u64,
        interval_secs: u64,
        limit: usize,
    ) -> Vec<RecrawlPlanEntry> {
        let mut entries = self
            .snapshot
            .records
            .values()
            .filter(|record| record.state == UrlState::Fetched)
            .filter_map(|record| {
                let recrawl_after = record.updated_at.saturating_add(interval_secs);
                if recrawl_after > now {
                    return None;
                }

                let age_secs = now.saturating_sub(record.updated_at);
                Some(RecrawlPlanEntry {
                    url: record.url.clone(),
                    priority: i64::try_from(age_secs).unwrap_or(i64::MAX),
                    recrawl_after,
                    last_fetched_at: record.updated_at,
                    age_secs,
                })
            })
            .collect::<Vec<_>>();

        entries.sort_by(|left, right| {
            right
                .age_secs
                .cmp(&left.age_secs)
                .then_with(|| left.url.cmp(&right.url))
        });
        if limit > 0 {
            entries.truncate(limit);
        }
        entries
    }

    pub fn save(&mut self) -> Result<()> {
        self.compact_failed_records(DEFAULT_MAX_FAILED_FRONTIER_RECORDS);

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create frontier parent {}", parent.display()))?;
        }

        let tmp_path = self.path.with_extension("tmp");
        fs::write(&tmp_path, bincode::serialize(&self.snapshot)?)
            .with_context(|| format!("write frontier temp {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &self.path).with_context(|| {
            format!(
                "replace frontier {} with {}",
                self.path.display(),
                tmp_path.display()
            )
        })?;
        Ok(())
    }

    fn active_by_host(&self) -> HashMap<&str, usize> {
        let mut active = HashMap::new();
        for record in self.snapshot.records.values() {
            if record.state == UrlState::Fetching {
                *active.entry(record.host.as_str()).or_insert(0) += 1;
            }
        }
        active
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_and_resumes_frontier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frontier.bin");
        let mut frontier = FrontierStore::open(&path).unwrap();

        assert!(frontier.discover(Url::parse("https://example.com/a#frag").unwrap(), 0, 10));
        assert!(frontier.discover(Url::parse("https://example.com/b").unwrap(), 1, 11));
        assert!(!frontier.discover(Url::parse("https://example.com/a").unwrap(), 3, 12));
        frontier.save().unwrap();

        let frontier = FrontierStore::open(&path).unwrap();
        assert_eq!(frontier.len(), 2);
        assert!(frontier.get("https://example.com/a").is_some());
        assert_eq!(frontier.stats().queued, 2);
    }

    #[test]
    fn claims_respect_host_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frontier.bin");
        let mut frontier = FrontierStore::open(path).unwrap();

        frontier.discover(Url::parse("https://example.com/a").unwrap(), 0, 10);
        frontier.discover(Url::parse("https://example.com/b").unwrap(), 0, 10);
        frontier.discover(Url::parse("https://other.test/a").unwrap(), 0, 10);

        let first = frontier.claim_next(20, 1).unwrap();
        assert_eq!(first.host, "example.com");

        let second = frontier.claim_next(20, 1).unwrap();
        assert_eq!(second.host, "other.test");

        assert!(frontier.claim_next(20, 1).is_none());
        assert_eq!(frontier.stats().fetching, 2);
    }

    #[test]
    fn records_fetch_and_retry_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frontier.bin");
        let mut frontier = FrontierStore::open(path).unwrap();
        frontier.discover(Url::parse("https://example.com/a").unwrap(), 0, 10);

        let claim = frontier.claim_next(20, 1).unwrap();
        assert_eq!(claim.attempts, 1);
        assert!(frontier.record_failed(&claim.url, "timeout".to_owned(), 30, 21));
        assert!(frontier.claim_next(40, 1).is_none());

        let retry = frontier.claim_next(60, 1).unwrap();
        assert_eq!(retry.attempts, 2);
        assert!(frontier.record_fetched(
            &retry.url,
            200,
            Some("https://example.com/a".to_owned()),
            Some("hash".to_owned()),
            61,
        ));

        let record = frontier.get("https://example.com/a").unwrap();
        assert_eq!(record.state, UrlState::Fetched);
        assert_eq!(record.last_status, Some(200));
        assert_eq!(record.content_hash.as_deref(), Some("hash"));
    }

    #[test]
    fn schedule_recrawl_requeues_fetched_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frontier.bin");
        let mut frontier = FrontierStore::open(path).unwrap();
        frontier.discover(Url::parse("https://example.com/a").unwrap(), 2, 10);
        let claim = frontier.claim_next(11, 1).unwrap();
        frontier.record_fetched(&claim.url, 200, None, Some("old-hash".to_owned()), 12);

        assert!(frontier.schedule_recrawl(Url::parse("https://example.com/a").unwrap(), 0, 20));
        let record = frontier.get("https://example.com/a").unwrap();
        assert_eq!(record.state, UrlState::Queued);
        assert_eq!(record.depth, 0);
        assert_eq!(record.attempts, 0);
        assert_eq!(record.content_hash.as_deref(), Some("old-hash"));

        let claim = frontier.claim_next(21, 1).unwrap();
        assert_eq!(claim.url, "https://example.com/a");
        assert_eq!(claim.attempts, 1);
    }

    #[test]
    fn resets_in_flight_claims_on_resume() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frontier.bin");
        let mut frontier = FrontierStore::open(path).unwrap();
        frontier.discover(Url::parse("https://example.com/a").unwrap(), 0, 10);

        let claim = frontier.claim_next(20, 1).unwrap();
        assert_eq!(claim.url, "https://example.com/a");
        assert!(frontier.claim_next(20, 1).is_none());

        assert_eq!(frontier.reset_fetching(30), 1);
        let claim = frontier.claim_next(31, 1).unwrap();
        assert_eq!(claim.url, "https://example.com/a");
        assert_eq!(claim.attempts, 2);
    }

    #[test]
    fn reports_host_stats() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frontier.bin");
        let mut frontier = FrontierStore::open(path).unwrap();
        frontier.discover(Url::parse("https://example.com/a").unwrap(), 0, 10);
        frontier.discover(Url::parse("https://example.com/b").unwrap(), 0, 10);
        frontier.discover(Url::parse("https://other.test/a").unwrap(), 0, 10);
        let claim = frontier.claim_next(20, 2).unwrap();
        frontier.record_fetched(&claim.url, 200, None, None, 21);

        let hosts = frontier.host_stats();
        let example = hosts
            .iter()
            .find(|stats| stats.host == "example.com")
            .unwrap();
        assert_eq!(example.total, 2);
        assert_eq!(example.fetched, 1);
        assert_eq!(example.queued, 1);
    }

    #[test]
    fn recrawl_plan_returns_old_fetched_urls_by_age() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frontier.bin");
        let mut frontier = FrontierStore::open(path).unwrap();

        frontier.discover(Url::parse("https://example.com/a-old").unwrap(), 0, 1);
        frontier.discover(Url::parse("https://example.com/b-new").unwrap(), 0, 1);
        frontier.discover(Url::parse("https://example.com/queued").unwrap(), 0, 1);

        let old = frontier.claim_next(2, 10).unwrap();
        frontier.record_fetched(&old.url, 200, None, None, 10);
        let new = frontier.claim_next(2, 10).unwrap();
        frontier.record_fetched(&new.url, 200, None, None, 95);

        let plan = frontier.recrawl_plan(110, 20, 10);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].url, "https://example.com/a-old");
        assert_eq!(plan[0].recrawl_after, 30);
        assert_eq!(plan[0].age_secs, 100);

        let plan = frontier.recrawl_plan(110, 0, 1);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].url, "https://example.com/a-old");
    }

    #[test]
    fn compact_failed_records_prunes_oldest_terminal_failures() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frontier.bin");
        let mut frontier = FrontierStore::open(path).unwrap();

        for (url, updated_at) in [
            ("https://example.com/oldest", 10),
            ("https://example.com/newest", 30),
            ("https://example.com/middle", 20),
        ] {
            frontier.discover(Url::parse(url).unwrap(), 0, 1);
            let claim = frontier.claim_next(2, 10).unwrap();
            frontier.record_failed(&claim.url, format!("failed at {updated_at}"), 0, updated_at);
        }

        frontier.discover(Url::parse("https://example.com/queued").unwrap(), 0, 40);
        frontier.discover(Url::parse("https://example.com/fetched").unwrap(), 0, 40);
        let fetched = frontier.claim_next(41, 10).unwrap();
        frontier.record_fetched(&fetched.url, 200, None, None, 42);
        frontier.discover(Url::parse("https://example.com/deferred").unwrap(), 0, 40);
        let deferred = frontier.claim_next(43, 10).unwrap();
        frontier.record_failed(&deferred.url, "retry later".to_owned(), 60, 44);

        assert_eq!(frontier.compact_failed_records(2), 1);
        assert!(frontier.get("https://example.com/oldest").is_none());
        assert_eq!(
            frontier.get("https://example.com/middle").unwrap().state,
            UrlState::Failed
        );
        assert_eq!(
            frontier.get("https://example.com/newest").unwrap().state,
            UrlState::Failed
        );
        assert_eq!(
            frontier.get("https://example.com/queued").unwrap().state,
            UrlState::Queued
        );
        assert_eq!(
            frontier.get("https://example.com/fetched").unwrap().state,
            UrlState::Fetched
        );
        assert_eq!(
            frontier.get("https://example.com/deferred").unwrap().state,
            UrlState::Deferred
        );
    }
}
