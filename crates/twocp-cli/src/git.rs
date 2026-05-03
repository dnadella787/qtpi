use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use twocp_core::artifact::ArtifactDecodeError;
use twocp_core::providers::{
    ArtifactProvider, DynamicValueProvider, Provider, ProviderCandidate, ProviderQuery,
    ProviderRootSummary, ProviderScope,
};
use twocp_core::spec::{
    CacheStatus, DynamicLookupRequest, DynamicLookupResult, DynamicLookupStatus, LookupMatch,
    ProviderId,
};

const DEFAULT_CACHE_TTL_MS: u32 = 5_000;
const DEFAULT_TIMEOUT_MS: u32 = 80;
const CACHE_VERSION: u8 = 1;

pub struct GitProvider {
    artifact: ArtifactProvider,
    dynamic: GitDynamicValueProvider,
}

impl GitProvider {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ArtifactDecodeError> {
        Ok(Self {
            artifact: ArtifactProvider::from_bytes(bytes)?,
            dynamic: GitDynamicValueProvider::default(),
        })
    }
}

impl Provider for GitProvider {
    fn id(&self) -> &ProviderId {
        self.artifact.id()
    }

    fn metadata(&self) -> &twocp_core::artifact::CompiledProviderMetadata {
        self.artifact.metadata()
    }

    fn root_summary(&self) -> ProviderRootSummary {
        self.artifact.root_summary()
    }

    fn resolve_scope(&self, query: &ProviderQuery) -> ProviderScope {
        self.artifact.resolve_scope(query)
    }

    fn static_suggestions(&self, scope: &ProviderScope) -> Vec<ProviderCandidate> {
        self.artifact.static_suggestions(scope)
    }

    fn value_suggestions(&self, scope: &ProviderScope) -> Vec<ProviderCandidate> {
        self.artifact.value_suggestions(scope)
    }

    fn build_dynamic_lookup_request(
        &self,
        query: &ProviderQuery,
        scope: &ProviderScope,
        max_candidates: usize,
    ) -> Option<DynamicLookupRequest> {
        let mut request =
            self.artifact
                .build_dynamic_lookup_request(query, scope, max_candidates)?;

        request.budget.timeout_ms = DEFAULT_TIMEOUT_MS;
        request.budget.max_candidates = max_candidates.min(8).min(u16::MAX as usize) as u16;
        request.allow_stale_cache = true;
        Some(request)
    }

    fn dynamic_value_provider(&self) -> Option<&dyn DynamicValueProvider> {
        Some(&self.dynamic)
    }
}

#[derive(Default)]
struct GitDynamicValueProvider {
    cache_dir_override: Option<PathBuf>,
    git_bin_override: Option<PathBuf>,
}

impl DynamicValueProvider for GitDynamicValueProvider {
    fn dynamic_lookup(&self, request: &DynamicLookupRequest) -> DynamicLookupResult {
        if !is_branch_slot(request.slot_id.as_str()) {
            return DynamicLookupResult::unsupported();
        }

        let started = Instant::now();
        let cache_key = cache_key(request);
        let cached = read_cache(
            cache_path(self.cache_dir_override.as_ref(), &cache_key),
            DEFAULT_CACHE_TTL_MS,
        );

        if let Some(fresh) = cached.as_ref().and_then(CacheRead::fresh_matches) {
            let matches = truncate_matches(
                &filter_matches_for_request(fresh, request),
                request.budget.max_candidates,
            );
            return DynamicLookupResult {
                status: status_for_matches(&matches),
                matches,
                cache_status: CacheStatus::HitFresh,
                degraded: false,
                lookup_time_ms: elapsed_ms(started),
            };
        }

        if !request.budget.allow_subprocess {
            return stale_or_degraded(
                cached,
                request,
                CacheStatus::Unsupported,
                DynamicLookupStatus::BudgetExceeded,
                started,
            );
        }

        match fetch_branch_snapshot(request, self.git_bin_override.as_ref()) {
            Ok(matches) => {
                let _ = write_cache(
                    cache_path(self.cache_dir_override.as_ref(), &cache_key),
                    &matches,
                );
                let matches = truncate_matches(
                    &filter_matches_for_request(&matches, request),
                    request.budget.max_candidates,
                );
                DynamicLookupResult {
                    status: status_for_matches(&matches),
                    matches,
                    cache_status: CacheStatus::Miss,
                    degraded: false,
                    lookup_time_ms: elapsed_ms(started),
                }
            }
            Err(FetchError::Timeout) => stale_or_degraded(
                cached,
                request,
                CacheStatus::HitStale,
                DynamicLookupStatus::BudgetExceeded,
                started,
            ),
            Err(FetchError::Process) => stale_or_degraded(
                cached,
                request,
                CacheStatus::HitStale,
                DynamicLookupStatus::Error,
                started,
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CacheEntry {
    version: u8,
    saved_at_ms: u64,
    matches: Vec<LookupMatch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CacheRead {
    Fresh(Vec<LookupMatch>),
    Stale(Vec<LookupMatch>),
}

impl CacheRead {
    fn fresh_matches(&self) -> Option<&[LookupMatch]> {
        match self {
            Self::Fresh(matches) => Some(matches),
            Self::Stale(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetchError {
    Timeout,
    Process,
}

fn is_branch_slot(slot_id: &str) -> bool {
    matches!(
        slot_id,
        "git.checkout.target"
            | "git.switch.branch"
            | "git.branch.name"
            | "git.merge.branch"
            | "git.rebase.upstream"
    )
}

fn cache_key(request: &DynamicLookupRequest) -> String {
    let mut hasher = DefaultHasher::new();
    request.provider_id.hash(&mut hasher);
    request.slot_id.hash(&mut hasher);
    request.scope.cwd.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn fetch_branch_snapshot(
    request: &DynamicLookupRequest,
    git_bin_override: Option<&PathBuf>,
) -> Result<Vec<LookupMatch>, FetchError> {
    let git_bin = git_bin_override.cloned().unwrap_or_else(|| {
        env::var_os("TWOCP_GIT_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("git"))
    });

    let mut command = Command::new(git_bin);
    command
        .arg("for-each-ref")
        .arg("--format=%(refname:short)")
        .arg("refs/heads")
        .arg("refs/remotes")
        .current_dir(&request.scope.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let output = run_command(command, request.budget.timeout_ms)?;
    let mut names: Vec<String> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.ends_with("/HEAD"))
        .map(ToOwned::to_owned)
        .collect();
    names.sort();
    names.dedup();

    Ok(names
        .into_iter()
        .map(|name| LookupMatch {
            value: name.clone(),
            display: name,
            annotation: None,
            confidence: 90,
            requires_quoting: false,
            is_stale: false,
        })
        .collect())
}

fn read_cache(path: PathBuf, ttl_ms: u32) -> Option<CacheRead> {
    let bytes = fs::read(path).ok()?;
    let entry: CacheEntry = serde_json::from_slice(&bytes).ok()?;
    if entry.version != CACHE_VERSION {
        return None;
    }

    let age_ms = now_ms().saturating_sub(entry.saved_at_ms);
    if age_ms <= u64::from(ttl_ms) {
        Some(CacheRead::Fresh(entry.matches))
    } else {
        Some(CacheRead::Stale(entry.matches))
    }
}

fn write_cache(path: PathBuf, matches: &[LookupMatch]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let entry = CacheEntry {
        version: CACHE_VERSION,
        saved_at_ms: now_ms(),
        matches: matches.to_vec(),
    };
    let bytes = serde_json::to_vec(&entry)?;
    fs::write(path, bytes)
}

fn cache_path(cache_dir_override: Option<&PathBuf>, cache_key: &str) -> PathBuf {
    cache_dir(cache_dir_override).join(format!("{cache_key}.json"))
}

fn cache_dir(cache_dir_override: Option<&PathBuf>) -> PathBuf {
    cache_dir_override.cloned().unwrap_or_else(|| {
        env::var_os("TWOCP_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| env::temp_dir().join("twocp-cache").join("git"))
    })
}

fn stale_or_degraded(
    cached: Option<CacheRead>,
    request: &DynamicLookupRequest,
    cache_status: CacheStatus,
    status: DynamicLookupStatus,
    started: Instant,
) -> DynamicLookupResult {
    if request.allow_stale_cache {
        if let Some(CacheRead::Stale(matches)) = cached {
            let matches = truncate_matches(
                &filter_matches_for_request(&matches, request),
                request.budget.max_candidates,
            );
            return DynamicLookupResult {
                matches,
                cache_status,
                status,
                degraded: true,
                lookup_time_ms: elapsed_ms(started),
            };
        }
    }

    DynamicLookupResult {
        matches: Vec::new(),
        cache_status: CacheStatus::Miss,
        status,
        degraded: true,
        lookup_time_ms: elapsed_ms(started),
    }
}

fn filter_matches_for_request(
    matches: &[LookupMatch],
    request: &DynamicLookupRequest,
) -> Vec<LookupMatch> {
    let fragment = request.partial_input.to_ascii_lowercase();
    let mut prefix = Vec::new();
    let mut fuzzy = Vec::new();

    for item in matches {
        let lowered = item.value.to_ascii_lowercase();
        if fragment.is_empty() || lowered.starts_with(&fragment) {
            prefix.push(item.clone());
        } else if is_light_fuzzy_match(&lowered, &fragment) {
            fuzzy.push(item.clone());
        }
    }

    prefix.extend(fuzzy);
    prefix
}

fn truncate_matches(matches: &[LookupMatch], max_candidates: u16) -> Vec<LookupMatch> {
    matches
        .iter()
        .take(max_candidates as usize)
        .cloned()
        .collect()
}

fn status_for_matches(matches: &[LookupMatch]) -> DynamicLookupStatus {
    if matches.is_empty() {
        DynamicLookupStatus::NoMatch
    } else {
        DynamicLookupStatus::Complete
    }
}

fn run_command(mut command: Command, timeout_ms: u32) -> Result<String, FetchError> {
    let mut child = command.spawn().map_err(|_| FetchError::Process)?;
    let timeout = Duration::from_millis(u64::from(timeout_ms));
    let started = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output().map_err(|_| FetchError::Process)?;
                if output.status.success() {
                    return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
                }
                return Err(FetchError::Process);
            }
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(FetchError::Timeout);
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(_) => return Err(FetchError::Process),
        }
    }
}

fn is_light_fuzzy_match(display: &str, fragment: &str) -> bool {
    let mut remaining = fragment.chars();
    let mut next = remaining.next();

    for ch in display.chars() {
        if Some(ch) == next {
            next = remaining.next();
            if next.is_none() {
                return true;
            }
        }
    }

    false
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn elapsed_ms(start: Instant) -> u32 {
    start.elapsed().as_millis().min(u32::MAX as u128) as u32
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::tempdir;
    use twocp_core::providers::Provider;
    use twocp_core::spec::{CommandPath, DynamicLookupBudget, DynamicLookupScope, SlotId};

    use super::*;

    fn git_provider() -> GitProvider {
        GitProvider::from_bytes(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/git-minimal.twocp-provider"
        )))
        .expect("git provider should load")
    }

    #[test]
    fn git_checkout_scope_resolves_dynamic_branch_slot() {
        let provider = git_provider();
        let scope = provider.resolve_scope(&ProviderQuery {
            provider_id: ProviderId::from("builtin.git"),
            command_tokens: vec!["checkout".into()],
            completion_position: twocp_core::parser::CompletionPosition::Value,
            active_fragment: String::new(),
            replace_range: twocp_core::protocol::ReplaceRange {
                start_byte: 13,
                end_byte: 13,
            },
            active_slot_id: None,
            degraded_parse: None,
            cwd: PathBuf::from("."),
        });

        assert_eq!(scope.command_path, CommandPath(vec!["checkout".into()]));
        assert_eq!(
            scope.active_slot_id,
            Some(SlotId::from("git.checkout.target"))
        );
    }

    #[test]
    fn dynamic_lookup_uses_cache_after_timeout() {
        let tempdir = tempdir().expect("tempdir should exist");
        let cache_dir = tempdir.path().join("cache");
        let script_path = tempdir.path().join("git");
        fs::write(
            &script_path,
            "#!/bin/sh\nsleep 1\nprintf 'main\\norigin/main\\n'",
        )
        .expect("script should write");
        let mut permissions = fs::metadata(&script_path)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("script should be executable");

        let mut request = branch_lookup_request(tempdir.path(), "ma");
        request.budget.timeout_ms = 25;
        let cache_key = cache_key(&request);
        let provider = GitDynamicValueProvider {
            cache_dir_override: Some(cache_dir.clone()),
            git_bin_override: Some(script_path),
        };
        let cache_path = cache_path(Some(&cache_dir), &cache_key);
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).expect("cache dir should exist");
        }
        fs::write(
            &cache_path,
            serde_json::to_vec(&CacheEntry {
                version: CACHE_VERSION,
                saved_at_ms: 0,
                matches: vec![LookupMatch {
                    value: "main-cached".into(),
                    display: "main-cached".into(),
                    annotation: None,
                    confidence: 90,
                    requires_quoting: false,
                    is_stale: false,
                }],
            })
            .expect("cache entry should serialize"),
        )
        .expect("cache entry should write");

        let result = provider.dynamic_lookup(&request);
        assert_eq!(result.cache_status, CacheStatus::HitStale);
        assert_eq!(result.status, DynamicLookupStatus::BudgetExceeded);
        assert!(result.degraded);
        assert_eq!(result.matches[0].display, "main-cached");
    }

    #[test]
    fn dynamic_lookup_cache_is_scoped_by_cwd() {
        let tempdir = tempdir().expect("tempdir should exist");
        let cache_dir = tempdir.path().join("cache");
        let script_path = tempdir.path().join("git");
        write_executable_script(
            &script_path,
            "#!/bin/sh\npwd=$(pwd)\ncase \"$pwd\" in\n  */repo-a) printf 'main\\nfeature/a\\n' ;;\n  */repo-b) printf 'main\\nfeature/b\\n' ;;\n  *) exit 7 ;;\nesac\n",
        );

        let repo_a = tempdir.path().join("repo-a");
        let repo_b = tempdir.path().join("repo-b");
        fs::create_dir_all(&repo_a).expect("repo a should exist");
        fs::create_dir_all(&repo_b).expect("repo b should exist");

        let provider = GitDynamicValueProvider {
            cache_dir_override: Some(cache_dir),
            git_bin_override: Some(script_path),
        };

        let result_a = provider.dynamic_lookup(&branch_lookup_request(&repo_a, "fea"));
        assert_eq!(result_a.cache_status, CacheStatus::Miss);
        assert_eq!(result_a.status, DynamicLookupStatus::Complete);
        assert_eq!(result_a.matches[0].display, "feature/a");

        let result_b = provider.dynamic_lookup(&branch_lookup_request(&repo_b, "fea"));
        assert_eq!(result_b.cache_status, CacheStatus::Miss);
        assert_eq!(result_b.status, DynamicLookupStatus::Complete);
        assert_eq!(result_b.matches[0].display, "feature/b");

        let result_a_cached = provider.dynamic_lookup(&branch_lookup_request(&repo_a, "fea"));
        assert_eq!(result_a_cached.cache_status, CacheStatus::HitFresh);
        assert_eq!(result_a_cached.matches[0].display, "feature/a");
    }

    fn write_executable_script(path: &std::path::Path, contents: &str) {
        fs::write(path, contents).expect("script should write");
        let mut permissions = fs::metadata(path).expect("script metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("script should be executable");
    }

    fn branch_lookup_request(cwd: &std::path::Path, partial_input: &str) -> DynamicLookupRequest {
        DynamicLookupRequest {
            provider_id: ProviderId::from("builtin.git"),
            command_path: CommandPath(vec!["checkout".into()]),
            slot_id: SlotId::from("git.checkout.target"),
            partial_input: partial_input.into(),
            scope: DynamicLookupScope {
                namespace: None,
                resource_kind: None,
                profile: None,
                region: None,
                cwd: cwd.to_path_buf(),
            },
            budget: DynamicLookupBudget {
                timeout_ms: 1_000,
                max_candidates: 8,
                allow_subprocess: true,
            },
            allow_stale_cache: true,
        }
    }
}
