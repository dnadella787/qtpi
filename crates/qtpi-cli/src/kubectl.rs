use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use qtpi_core::artifact::ArtifactDecodeError;
use qtpi_core::providers::{
    ArtifactProvider, DynamicValueProvider, Provider, ProviderCandidate, ProviderQuery,
    ProviderRootSummary, ProviderScope,
};
use qtpi_core::spec::{
    CacheStatus, DynamicLookupRequest, DynamicLookupResult, DynamicLookupStatus, LookupMatch,
    ProviderId,
};
use serde::{Deserialize, Serialize};

const DEFAULT_CACHE_TTL_MS: u32 = 5_000;
const DEFAULT_TIMEOUT_MS: u32 = 120;
const CACHE_VERSION: u8 = 3;
const ALL_NAMESPACES_SENTINEL: &str = "*";
const EXPLICIT_KUBECONFIG_PREFIX: &str = "explicit:";

pub struct KubectlProvider {
    artifact: ArtifactProvider,
    dynamic: KubectlDynamicValueProvider,
}

impl KubectlProvider {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ArtifactDecodeError> {
        Ok(Self {
            artifact: ArtifactProvider::from_bytes(bytes)?,
            dynamic: KubectlDynamicValueProvider::default(),
        })
    }
}

impl Provider for KubectlProvider {
    fn id(&self) -> &ProviderId {
        self.artifact.id()
    }

    fn metadata(&self) -> &qtpi_core::artifact::CompiledProviderMetadata {
        self.artifact.metadata()
    }

    fn root_summary(&self) -> ProviderRootSummary {
        self.artifact.root_summary()
    }

    fn resolve_scope(&self, query: &ProviderQuery) -> ProviderScope {
        let normalized_query = normalize_kubectl_query(query);
        let scope_query = normalized_query.as_ref().unwrap_or(query);
        let mut scope = self.artifact.resolve_scope(scope_query);
        scope.lookup_scope.namespace = kubectl_namespace_scope(scope_query);
        scope.lookup_scope.resource_kind = kubectl_resource_kind(&scope);
        scope.lookup_scope.profile = kubectl_context_scope(scope_query);
        scope.lookup_scope.region = kubeconfig_scope_identity(scope_query);
        scope
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
struct KubectlDynamicValueProvider {
    cache_dir_override: Option<PathBuf>,
    kubectl_bin_override: Option<PathBuf>,
}

impl DynamicValueProvider for KubectlDynamicValueProvider {
    fn dynamic_lookup(&self, request: &DynamicLookupRequest) -> DynamicLookupResult {
        let Some(lookup_kind) = lookup_kind(request) else {
            return DynamicLookupResult::unsupported();
        };

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

        let fetch_result = match lookup_kind {
            KubectlLookupKind::ResourceName => {
                fetch_resource_snapshot(request, self.kubectl_bin_override.as_ref())
            }
            KubectlLookupKind::Namespace => {
                fetch_namespace_snapshot(request, self.kubectl_bin_override.as_ref())
            }
            KubectlLookupKind::Context => {
                fetch_context_snapshot(request, self.kubectl_bin_override.as_ref())
            }
        };

        match fetch_result {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KubectlLookupKind {
    ResourceName,
    Namespace,
    Context,
}

fn lookup_kind(request: &DynamicLookupRequest) -> Option<KubectlLookupKind> {
    match request.slot_id.as_str() {
        "kubectl.root.namespace" => Some(KubectlLookupKind::Namespace),
        "kubectl.root.context" | "kubectl.config.use_context.name" => {
            Some(KubectlLookupKind::Context)
        }
        slot_id if slot_id.starts_with("kubectl.") && slot_id.ends_with(".name") => {
            Some(KubectlLookupKind::ResourceName)
        }
        _ => None,
    }
}

fn kubectl_namespace_scope(query: &ProviderQuery) -> Option<String> {
    if query
        .command_tokens
        .iter()
        .any(|token| token == "--all-namespaces" || token == "-A")
    {
        Some(ALL_NAMESPACES_SENTINEL.to_string())
    } else {
        option_value(query, &["--namespace", "-n"])
    }
}

fn kubectl_context_scope(query: &ProviderQuery) -> Option<String> {
    option_value(query, &["--context"])
}

fn kubeconfig_scope_identity(query: &ProviderQuery) -> Option<String> {
    if let Some(kubeconfig) = option_value(query, &["--kubeconfig"]) {
        return Some(format!(
            "{EXPLICIT_KUBECONFIG_PREFIX}{}",
            kubeconfig_path_identity(&query.cwd, &kubeconfig)
        ));
    }

    if let Some(kubeconfig) = env::var_os("KUBECONFIG") {
        return Some(format!("env:{}", kubeconfig.to_string_lossy()));
    }

    env::var_os("HOME").map(|home| {
        let path = PathBuf::from(home).join(".kube").join("config");
        format!("default:{}", path.display())
    })
}

fn kubectl_resource_kind(scope: &ProviderScope) -> Option<String> {
    match scope
        .active_slot_id
        .as_ref()
        .map(|slot_id| slot_id.as_str())
    {
        Some("kubectl.root.namespace")
        | Some("kubectl.root.context")
        | Some("kubectl.config.use_context.name") => None,
        Some("kubectl.logs.pod.name") | Some("kubectl.exec.pod.name") => Some("pods".into()),
        Some(_) => scope
            .command_path
            .segments()
            .last()
            .and_then(|segment| canonical_resource_kind(segment)),
        None => None,
    }
}

fn canonical_resource_kind(segment: &str) -> Option<String> {
    match segment {
        "pods"
        | "deployments"
        | "replicasets"
        | "statefulsets"
        | "daemonsets"
        | "jobs"
        | "cronjobs"
        | "services"
        | "ingresses"
        | "configmaps"
        | "secrets"
        | "namespaces"
        | "nodes"
        | "events"
        | "persistentvolumeclaims" => Some(segment.to_string()),
        _ => None,
    }
}

fn cache_key(request: &DynamicLookupRequest) -> String {
    let mut hasher = DefaultHasher::new();
    request.provider_id.hash(&mut hasher);
    request.slot_id.hash(&mut hasher);
    request.scope.namespace.hash(&mut hasher);
    request.scope.resource_kind.hash(&mut hasher);
    request.scope.profile.hash(&mut hasher);
    request.scope.region.hash(&mut hasher);
    request.scope.cwd.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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
        env::var_os("QTPI_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| env::temp_dir().join("qtpi-cache").join("kubectl"))
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

fn fetch_resource_snapshot(
    request: &DynamicLookupRequest,
    kubectl_bin_override: Option<&PathBuf>,
) -> Result<Vec<LookupMatch>, FetchError> {
    let Some(resource_kind) = request.scope.resource_kind.as_deref() else {
        return Err(FetchError::Process);
    };

    let output = run_kubectl(
        request,
        kubectl_bin_override,
        &[
            "get",
            resource_kind,
            "--no-headers",
            "-o",
            "custom-columns=NAME:.metadata.name,NAMESPACE:.metadata.namespace",
        ],
        should_include_namespace_scope(resource_kind),
    )?;

    let mut rows = Vec::new();
    for line in output.lines() {
        let mut columns = line.split_whitespace();
        let Some(name) = columns.next() else {
            continue;
        };
        let namespace = columns.next().unwrap_or_default();
        rows.push((name.to_string(), namespace.to_string()));
    }

    rows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(resource_rows_to_matches(rows, request))
}

fn fetch_namespace_snapshot(
    request: &DynamicLookupRequest,
    kubectl_bin_override: Option<&PathBuf>,
) -> Result<Vec<LookupMatch>, FetchError> {
    let output = run_kubectl(
        request,
        kubectl_bin_override,
        &[
            "get",
            "namespaces",
            "--no-headers",
            "-o",
            "custom-columns=NAME:.metadata.name",
        ],
        false,
    )?;

    let mut names: Vec<String> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    names.sort();

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

fn fetch_context_snapshot(
    request: &DynamicLookupRequest,
    kubectl_bin_override: Option<&PathBuf>,
) -> Result<Vec<LookupMatch>, FetchError> {
    let output = run_kubectl(
        request,
        kubectl_bin_override,
        &["config", "get-contexts", "-o", "name"],
        false,
    )?;

    let mut names: Vec<String> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    names.sort();

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

fn resource_rows_to_matches(
    rows: Vec<(String, String)>,
    request: &DynamicLookupRequest,
) -> Vec<LookupMatch> {
    let annotate_namespace = matches!(
        request.scope.namespace.as_deref(),
        Some(ALL_NAMESPACES_SENTINEL)
    );

    rows.into_iter()
        .map(|(name, namespace)| {
            let annotation = if annotate_namespace && !namespace.is_empty() {
                Some(namespace)
            } else {
                None
            };
            LookupMatch {
                value: name.clone(),
                display: name,
                annotation,
                confidence: 90,
                requires_quoting: false,
                is_stale: false,
            }
        })
        .collect()
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

fn run_kubectl(
    request: &DynamicLookupRequest,
    kubectl_bin_override: Option<&PathBuf>,
    args: &[&str],
    include_namespace_scope: bool,
) -> Result<String, FetchError> {
    let kubectl_bin = kubectl_bin_override.cloned().unwrap_or_else(|| {
        env::var_os("QTPI_KUBECTL_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("kubectl"))
    });
    let mut command = Command::new(kubectl_bin);
    command
        .args(args)
        .current_dir(&request.scope.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    if let Some(context) = request.scope.profile.as_deref() {
        command.arg("--context").arg(context);
    }

    if let Some(kubeconfig) = explicit_kubeconfig_arg(request) {
        command.arg("--kubeconfig").arg(kubeconfig);
    }

    if include_namespace_scope {
        match request.scope.namespace.as_deref() {
            Some(ALL_NAMESPACES_SENTINEL) => {
                command.arg("-A");
            }
            Some(namespace) => {
                command.arg("-n").arg(namespace);
            }
            None => {}
        }
    }

    run_command(command, request.budget.timeout_ms)
}

fn should_include_namespace_scope(resource_kind: &str) -> bool {
    !matches!(resource_kind, "namespaces" | "nodes")
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn elapsed_ms(start: Instant) -> u32 {
    start.elapsed().as_millis().min(u32::MAX as u128) as u32
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

fn normalize_kubectl_query(query: &ProviderQuery) -> Option<ProviderQuery> {
    let mut changed = false;
    let mut command_tokens = Vec::with_capacity(query.command_tokens.len());

    for token in &query.command_tokens {
        if let Some((flag, value)) = split_inline_option(token) {
            command_tokens.push(flag);
            command_tokens.push(value);
            changed = true;
        } else {
            command_tokens.push(token.clone());
        }
    }

    changed.then(|| ProviderQuery {
        command_tokens,
        ..query.clone()
    })
}

fn split_inline_option(token: &str) -> Option<(String, String)> {
    for flag in ["--namespace", "--context", "--kubeconfig"] {
        let prefix = format!("{flag}=");
        if let Some(value) = token.strip_prefix(&prefix) {
            return Some((flag.to_string(), value.to_string()));
        }
    }

    None
}

fn option_value(query: &ProviderQuery, names: &[&str]) -> Option<String> {
    let mut token_index = 0usize;
    let mut value = None;

    while token_index < query.command_tokens.len() {
        if names
            .iter()
            .any(|name| query.command_tokens[token_index] == *name)
        {
            value = query.command_tokens.get(token_index + 1).cloned();
            token_index += 2;
        } else {
            token_index += 1;
        }
    }

    value
}

fn kubeconfig_path_identity(cwd: &std::path::Path, kubeconfig: &str) -> String {
    let path = PathBuf::from(kubeconfig);
    if path.is_absolute() {
        path.display().to_string()
    } else {
        cwd.join(path).display().to_string()
    }
}

fn explicit_kubeconfig_arg(request: &DynamicLookupRequest) -> Option<&str> {
    request
        .scope
        .region
        .as_deref()
        .and_then(|identity| identity.strip_prefix(EXPLICIT_KUBECONFIG_PREFIX))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use qtpi_core::providers::Provider;
    use qtpi_core::spec::{CommandPath, DynamicLookupBudget, DynamicLookupScope, SlotId};
    use tempfile::tempdir;

    use super::*;

    fn kubectl_provider() -> KubectlProvider {
        KubectlProvider::from_bytes(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/kubectl-minimal.qtpi-provider"
        )))
        .expect("kubectl provider should load")
    }

    #[test]
    fn resolve_scope_captures_namespace_context_kubeconfig_and_dynamic_resource_slot() {
        let provider = kubectl_provider();
        let scope = provider.resolve_scope(&ProviderQuery {
            provider_id: ProviderId::from("builtin.kubectl"),
            command_tokens: vec![
                "--context=team-a".into(),
                "--kubeconfig".into(),
                "/tmp/qtpi-kubeconfig".into(),
                "--namespace".into(),
                "kube-system".into(),
                "describe".into(),
                "pod".into(),
            ],
            completion_position: qtpi_core::parser::CompletionPosition::Value,
            active_fragment: String::new(),
            replace_range: qtpi_core::protocol::ReplaceRange {
                start_byte: 32,
                end_byte: 32,
            },
            active_slot_id: None,
            degraded_parse: None,
            cwd: PathBuf::from("."),
        });

        assert_eq!(
            scope.command_path,
            CommandPath(vec!["describe".into(), "pods".into()])
        );
        assert_eq!(
            scope.active_slot_id,
            Some(SlotId::from("kubectl.describe.pods.name"))
        );
        assert_eq!(scope.lookup_scope.namespace.as_deref(), Some("kube-system"));
        assert_eq!(scope.lookup_scope.resource_kind.as_deref(), Some("pods"));
        assert_eq!(scope.lookup_scope.profile.as_deref(), Some("team-a"));
        assert_eq!(
            scope.lookup_scope.region.as_deref(),
            Some("explicit:/tmp/qtpi-kubeconfig")
        );
    }

    #[test]
    fn resolve_scope_supports_get_pods_and_use_context_dynamic_slots() {
        let provider = kubectl_provider();

        let get_scope = provider.resolve_scope(&ProviderQuery {
            provider_id: ProviderId::from("builtin.kubectl"),
            command_tokens: vec!["get".into(), "pods".into()],
            completion_position: qtpi_core::parser::CompletionPosition::Value,
            active_fragment: String::new(),
            replace_range: qtpi_core::protocol::ReplaceRange {
                start_byte: 16,
                end_byte: 16,
            },
            active_slot_id: None,
            degraded_parse: None,
            cwd: PathBuf::from("."),
        });
        assert_eq!(
            get_scope.active_slot_id,
            Some(SlotId::from("kubectl.get.pods.name"))
        );
        assert_eq!(
            get_scope.lookup_scope.resource_kind.as_deref(),
            Some("pods")
        );

        let context_scope = provider.resolve_scope(&ProviderQuery {
            provider_id: ProviderId::from("builtin.kubectl"),
            command_tokens: vec!["config".into(), "use-context".into()],
            completion_position: qtpi_core::parser::CompletionPosition::Value,
            active_fragment: String::new(),
            replace_range: qtpi_core::protocol::ReplaceRange {
                start_byte: 27,
                end_byte: 27,
            },
            active_slot_id: None,
            degraded_parse: None,
            cwd: PathBuf::from("."),
        });
        assert_eq!(
            context_scope.active_slot_id,
            Some(SlotId::from("kubectl.config.use_context.name"))
        );
    }

    #[test]
    fn dynamic_lookup_uses_cache_after_timeout() {
        let tempdir = tempdir().expect("tempdir should exist");
        let cache_dir = tempdir.path().join("cache");
        let script_path = tempdir.path().join("kubectl");
        fs::write(
            &script_path,
            "#!/bin/sh\nsleep 1\nprintf 'pod-a kube-system\\n'",
        )
        .expect("script should write");
        let mut permissions = fs::metadata(&script_path)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("script should be executable");

        let request = resource_lookup_request(
            tempdir.path(),
            "kubectl.describe.pods.name",
            CommandPath(vec!["describe".into(), "pods".into()]),
            "pods",
            "pod",
            Some("kube-system"),
            None,
        );

        let cache_key = cache_key(&request);
        let provider = KubectlDynamicValueProvider {
            cache_dir_override: Some(cache_dir.clone()),
            kubectl_bin_override: Some(script_path.clone()),
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
                    value: "pod-cached".into(),
                    display: "pod-cached".into(),
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
        assert_eq!(result.matches[0].display, "pod-cached");
    }

    #[test]
    fn fresh_cache_filters_for_current_prefix_without_prefix_poisoning() {
        let tempdir = tempdir().expect("tempdir should exist");
        let cache_dir = tempdir.path().join("cache");
        let script_path = tempdir.path().join("kubectl");
        write_executable_script(
            &script_path,
            "#!/bin/sh\nprintf 'api-one default\\napi-two default\\nworker default\\n'",
        );

        let provider = KubectlDynamicValueProvider {
            cache_dir_override: Some(cache_dir),
            kubectl_bin_override: Some(script_path),
        };
        let narrow = resource_lookup_request(
            tempdir.path(),
            "kubectl.describe.pods.name",
            CommandPath(vec!["describe".into(), "pods".into()]),
            "pods",
            "api-t",
            Some("default"),
            None,
        );
        let broad = resource_lookup_request(
            tempdir.path(),
            "kubectl.describe.pods.name",
            CommandPath(vec!["describe".into(), "pods".into()]),
            "pods",
            "api",
            Some("default"),
            None,
        );
        let worker = resource_lookup_request(
            tempdir.path(),
            "kubectl.describe.pods.name",
            CommandPath(vec!["describe".into(), "pods".into()]),
            "pods",
            "work",
            Some("default"),
            None,
        );

        let first = provider.dynamic_lookup(&narrow);
        assert_eq!(first.cache_status, CacheStatus::Miss);
        assert_eq!(
            first
                .matches
                .iter()
                .map(|candidate| candidate.display.as_str())
                .collect::<Vec<_>>(),
            vec!["api-two"]
        );

        let second = provider.dynamic_lookup(&broad);
        assert_eq!(second.cache_status, CacheStatus::HitFresh);
        assert_eq!(
            second
                .matches
                .iter()
                .map(|candidate| candidate.display.as_str())
                .collect::<Vec<_>>(),
            vec!["api-one", "api-two"]
        );

        let third = provider.dynamic_lookup(&worker);
        assert_eq!(third.cache_status, CacheStatus::HitFresh);
        assert_eq!(
            third
                .matches
                .iter()
                .map(|candidate| candidate.display.as_str())
                .collect::<Vec<_>>(),
            vec!["worker"]
        );
    }

    #[test]
    fn dynamic_lookup_cache_is_context_scoped_and_passes_context_to_kubectl() {
        let tempdir = tempdir().expect("tempdir should exist");
        let cache_dir = tempdir.path().join("cache");
        let script_path = tempdir.path().join("kubectl");
        write_executable_script(
            &script_path,
            "#!/bin/sh\ncontext=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = '--context' ]; then\n    shift\n    context=\"$1\"\n  fi\n  shift\ndone\ncase \"$context\" in\n  ctx-a) printf 'pod-a default\\n' ;;\n  ctx-b) printf 'pod-b default\\n' ;;\n  *) exit 7 ;;\nesac\n",
        );

        let provider = KubectlDynamicValueProvider {
            cache_dir_override: Some(cache_dir),
            kubectl_bin_override: Some(script_path),
        };

        let context_a = resource_lookup_request(
            tempdir.path(),
            "kubectl.describe.pods.name",
            CommandPath(vec!["describe".into(), "pods".into()]),
            "pods",
            "pod",
            Some("default"),
            Some("ctx-a"),
        );
        let result_a = provider.dynamic_lookup(&context_a);
        assert_eq!(result_a.cache_status, CacheStatus::Miss);
        assert_eq!(result_a.status, DynamicLookupStatus::Complete);
        assert_eq!(result_a.matches[0].display, "pod-a");

        let context_b = resource_lookup_request(
            tempdir.path(),
            "kubectl.describe.pods.name",
            CommandPath(vec!["describe".into(), "pods".into()]),
            "pods",
            "pod",
            Some("default"),
            Some("ctx-b"),
        );
        let result_b = provider.dynamic_lookup(&context_b);
        assert_eq!(result_b.cache_status, CacheStatus::Miss);
        assert_eq!(result_b.status, DynamicLookupStatus::Complete);
        assert_eq!(result_b.matches[0].display, "pod-b");

        let result_a_cached = provider.dynamic_lookup(&context_a);
        assert_eq!(result_a_cached.cache_status, CacheStatus::HitFresh);
        assert_eq!(result_a_cached.matches[0].display, "pod-a");
    }

    #[test]
    fn namespace_and_context_slots_use_dedicated_lookup_paths() {
        let tempdir = tempdir().expect("tempdir should exist");
        let cache_dir = tempdir.path().join("cache");
        let script_path = tempdir.path().join("kubectl");
        write_executable_script(
            &script_path,
            "#!/bin/sh\nif [ \"$1\" = 'get' ] && [ \"$2\" = 'namespaces' ]; then\n  printf 'default\\nkube-system\\n'\n  exit 0\nfi\nif [ \"$1\" = 'config' ] && [ \"$2\" = 'get-contexts' ]; then\n  printf 'ctx-a\\nctx-b\\n'\n  exit 0\nfi\nexit 7\n",
        );

        let provider = KubectlDynamicValueProvider {
            cache_dir_override: Some(cache_dir),
            kubectl_bin_override: Some(script_path),
        };

        let namespaces = provider.dynamic_lookup(&DynamicLookupRequest {
            provider_id: ProviderId::from("builtin.kubectl"),
            command_path: CommandPath::root(),
            slot_id: SlotId::from("kubectl.root.namespace"),
            partial_input: "ku".into(),
            scope: DynamicLookupScope {
                namespace: None,
                resource_kind: None,
                profile: Some("ctx-a".into()),
                region: None,
                cwd: tempdir.path().to_path_buf(),
            },
            budget: DynamicLookupBudget {
                timeout_ms: 1_000,
                max_candidates: 8,
                allow_subprocess: true,
            },
            allow_stale_cache: true,
        });
        assert_eq!(namespaces.status, DynamicLookupStatus::Complete);
        assert_eq!(namespaces.matches[0].display, "kube-system");

        let contexts = provider.dynamic_lookup(&DynamicLookupRequest {
            provider_id: ProviderId::from("builtin.kubectl"),
            command_path: CommandPath(vec!["config".into(), "use-context".into()]),
            slot_id: SlotId::from("kubectl.config.use_context.name"),
            partial_input: "ctx".into(),
            scope: DynamicLookupScope {
                namespace: None,
                resource_kind: None,
                profile: None,
                region: None,
                cwd: tempdir.path().to_path_buf(),
            },
            budget: DynamicLookupBudget {
                timeout_ms: 1_000,
                max_candidates: 8,
                allow_subprocess: true,
            },
            allow_stale_cache: true,
        });
        assert_eq!(contexts.status, DynamicLookupStatus::Complete);
        assert_eq!(
            contexts
                .matches
                .iter()
                .map(|candidate| candidate.display.as_str())
                .collect::<Vec<_>>(),
            vec!["ctx-a", "ctx-b"]
        );
    }

    fn write_executable_script(path: &std::path::Path, contents: &str) {
        fs::write(path, contents).expect("script should write");
        let mut permissions = fs::metadata(path).expect("script metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("script should be executable");
    }

    fn resource_lookup_request(
        cwd: &std::path::Path,
        slot_id: &str,
        command_path: CommandPath,
        resource_kind: &str,
        partial_input: &str,
        namespace: Option<&str>,
        context: Option<&str>,
    ) -> DynamicLookupRequest {
        DynamicLookupRequest {
            provider_id: ProviderId::from("builtin.kubectl"),
            command_path,
            slot_id: SlotId::from(slot_id),
            partial_input: partial_input.into(),
            scope: DynamicLookupScope {
                namespace: namespace.map(String::from),
                resource_kind: Some(resource_kind.into()),
                profile: context.map(String::from),
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
