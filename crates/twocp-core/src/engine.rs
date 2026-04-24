use std::cmp::Reverse;
use std::time::Instant;

use crate::parser::{ParserStatus, parse_request};
use crate::protocol::{
    Diagnostics, DynamicLookupDiagnostics, RenderModel, ResponseStatus, SuggestRequest,
    SuggestResponse, Suggestion, SuggestionKind, TimingDiagnostics,
};
use crate::providers::{ProviderCandidate, ProviderCatalog, ProviderQuery};
use crate::spec::{CacheStatus, DynamicLookupResult, DynamicLookupStatus, QuoteStyle, ValueSpec};

const DEFAULT_MAX_SUGGESTIONS: usize = 8;

pub struct SuggestEngine<'a, C>
where
    C: ProviderCatalog + ?Sized,
{
    catalog: &'a C,
    max_suggestions: usize,
}

impl<'a, C> SuggestEngine<'a, C>
where
    C: ProviderCatalog + ?Sized,
{
    pub fn new(catalog: &'a C) -> Self {
        Self {
            catalog,
            max_suggestions: DEFAULT_MAX_SUGGESTIONS,
        }
    }

    pub fn with_max_suggestions(mut self, max_suggestions: usize) -> Self {
        self.max_suggestions = max_suggestions.max(1);
        self
    }

    pub fn suggest(&self, request: &SuggestRequest) -> SuggestResponse {
        let total_start = Instant::now();
        if request.validate().is_err() {
            return SuggestResponse {
                replace_range: None,
                suggestions: Vec::new(),
                selection_index: None,
                render_model: RenderModel::from_suggestions(&[], 0, true),
                status: ResponseStatus::Error,
                diagnostics: None,
            };
        }

        let parse_start = Instant::now();
        let parse = parse_request(request, self.catalog.root_index());
        let parse_ms = elapsed_ms(parse_start);

        if let ParserStatus::Degraded(_) = parse.status {
            return SuggestResponse {
                replace_range: None,
                suggestions: Vec::new(),
                selection_index: None,
                render_model: RenderModel::from_suggestions(&[], 0, true),
                status: ResponseStatus::Degraded,
                diagnostics: Some(Diagnostics {
                    provider_id: parse.provider_root,
                    parser_status: parse.status,
                    timings: TimingDiagnostics {
                        parse_ms,
                        provider_ms: 0,
                        dynamic_lookup_ms: 0,
                        total_ms: elapsed_ms(total_start),
                    },
                    dynamic_lookup: None,
                }),
            };
        }

        let Some(query) = ProviderQuery::from_parse(request, &parse) else {
            return SuggestResponse {
                replace_range: None,
                suggestions: Vec::new(),
                selection_index: None,
                render_model: RenderModel::from_suggestions(&[], 0, false),
                status: ResponseStatus::NoMatch,
                diagnostics: Some(Diagnostics {
                    provider_id: None,
                    parser_status: parse.status,
                    timings: TimingDiagnostics {
                        parse_ms,
                        provider_ms: 0,
                        dynamic_lookup_ms: 0,
                        total_ms: elapsed_ms(total_start),
                    },
                    dynamic_lookup: None,
                }),
            };
        };

        let provider_start = Instant::now();
        let provider = match self.catalog.load_provider(&query.provider_id) {
            Ok(provider) => provider,
            Err(_) => {
                return SuggestResponse {
                    replace_range: None,
                    suggestions: Vec::new(),
                    selection_index: None,
                    render_model: RenderModel::from_suggestions(&[], 0, false),
                    status: ResponseStatus::Error,
                    diagnostics: Some(Diagnostics {
                        provider_id: Some(query.provider_id),
                        parser_status: parse.status,
                        timings: TimingDiagnostics {
                            parse_ms,
                            provider_ms: elapsed_ms(provider_start),
                            dynamic_lookup_ms: 0,
                            total_ms: elapsed_ms(total_start),
                        },
                        dynamic_lookup: None,
                    }),
                };
            }
        };

        let scope = provider.resolve_scope(&query);
        let (candidates, dynamic_result, dynamic_request_present) = if let Some(dynamic_request) =
            provider.build_dynamic_lookup_request(&query, &scope, self.max_suggestions)
        {
            let dynamic_result = provider
                .dynamic_value_provider()
                .map(|dynamic_provider| dynamic_provider.dynamic_lookup(&dynamic_request))
                .unwrap_or_else(|| DynamicLookupResult {
                    matches: Vec::new(),
                    cache_status: CacheStatus::Unsupported,
                    status: DynamicLookupStatus::Unsupported,
                    degraded: true,
                    lookup_time_ms: 0,
                });

            (
                dynamic_result_to_candidates(&dynamic_result, scope.active_value.as_ref()),
                Some((dynamic_request, dynamic_result)),
                true,
            )
        } else {
            let value_candidates = provider.value_suggestions(&scope);
            let candidates = if scope.active_slot_id.is_some() {
                value_candidates
            } else {
                provider.static_suggestions(&scope)
            };
            (candidates, None, false)
        };
        let provider_ms = elapsed_ms(provider_start);
        let ranked = rank_candidates(&query, candidates, self.max_suggestions);
        let suggestions: Vec<Suggestion> = ranked
            .candidates
            .iter()
            .map(|candidate| Suggestion {
                insert_text: insert_text_for_candidate(candidate),
                display: candidate.display.clone(),
                annotation: candidate.annotation.clone(),
                kind: candidate.kind,
                requires_quoting: candidate.requires_quoting,
            })
            .collect();
        let (status, degraded) = response_status(
            &scope,
            &suggestions,
            dynamic_result.as_ref().map(|(_, result)| result),
            dynamic_request_present,
        );
        let dynamic_lookup_ms = dynamic_result
            .as_ref()
            .map(|(_, result)| result.lookup_time_ms)
            .unwrap_or_default();
        let dynamic_lookup =
            dynamic_result
                .as_ref()
                .map(|(request, result)| DynamicLookupDiagnostics {
                    slot_id: request.slot_id.clone(),
                    scope: request.scope.clone(),
                    cache_status: result.cache_status,
                    status: result.status,
                    match_count: result.matches.len(),
                    lookup_time_ms: result.lookup_time_ms,
                });
        let render_model =
            RenderModel::from_suggestions(&suggestions, ranked.truncated_count, degraded);
        let total_ms = elapsed_ms(total_start);

        SuggestResponse {
            replace_range: suggestions.first().map(|_| query.replace_range),
            selection_index: if suggestions.is_empty() {
                None
            } else {
                Some(0)
            },
            render_model,
            suggestions,
            status,
            diagnostics: Some(Diagnostics {
                provider_id: Some(query.provider_id),
                parser_status: parse.status,
                timings: TimingDiagnostics {
                    parse_ms,
                    provider_ms,
                    dynamic_lookup_ms,
                    total_ms,
                },
                dynamic_lookup,
            }),
        }
    }
}

fn insert_text_for_candidate(candidate: &ProviderCandidate) -> String {
    let replacement = if candidate.requires_quoting {
        quote_replacement(&candidate.replacement, candidate.quote_style)
    } else {
        candidate.replacement.clone()
    };

    match candidate.kind {
        SuggestionKind::Command | SuggestionKind::Flag => format!("{replacement} "),
        SuggestionKind::Value | SuggestionKind::Help => replacement,
    }
}

fn dynamic_result_to_candidates(
    result: &DynamicLookupResult,
    value_spec: Option<&ValueSpec>,
) -> Vec<ProviderCandidate> {
    let quote_style = value_spec
        .map(|value| value.quote_style)
        .unwrap_or(QuoteStyle::None);

    result
        .matches
        .iter()
        .map(|candidate| ProviderCandidate {
            replacement: candidate.value.clone(),
            display: candidate.display.clone(),
            annotation: candidate.annotation.clone(),
            kind: SuggestionKind::Value,
            priority: candidate.confidence,
            requires_quoting: candidate.requires_quoting,
            quote_style,
        })
        .collect()
}

fn quote_replacement(value: &str, quote_style: QuoteStyle) -> String {
    match quote_style {
        QuoteStyle::None => value.to_string(),
        QuoteStyle::SingleQuotes => format!("'{}'", value.replace('\'', "'\\''")),
        QuoteStyle::DoubleQuotes => {
            let escaped = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$")
                .replace('`', "\\`");
            format!("\"{escaped}\"")
        }
        QuoteStyle::BackslashEscape => {
            let mut escaped = String::with_capacity(value.len());
            for ch in value.chars() {
                if ch.is_whitespace()
                    || matches!(
                        ch,
                        '\'' | '"'
                            | '\\'
                            | '$'
                            | '`'
                            | '|'
                            | '&'
                            | ';'
                            | '<'
                            | '>'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '*'
                            | '?'
                            | '!'
                    )
                {
                    escaped.push('\\');
                }
                escaped.push(ch);
            }
            escaped
        }
    }
}

fn response_status(
    scope: &crate::providers::ProviderScope,
    suggestions: &[Suggestion],
    dynamic_result: Option<&DynamicLookupResult>,
    dynamic_request_present: bool,
) -> (ResponseStatus, bool) {
    if let Some(dynamic_result) = dynamic_result {
        let degraded = dynamic_result.degraded
            || matches!(
                dynamic_result.status,
                DynamicLookupStatus::BudgetExceeded | DynamicLookupStatus::Error
            );
        let status = if degraded {
            ResponseStatus::Degraded
        } else if suggestions.is_empty() {
            ResponseStatus::NoMatch
        } else {
            ResponseStatus::Ok
        };
        return (status, degraded);
    }

    if dynamic_request_present {
        return (ResponseStatus::Degraded, true);
    }

    if scope.active_slot_id.is_some() && suggestions.is_empty() {
        return (ResponseStatus::NoMatch, false);
    }

    if suggestions.is_empty() {
        (ResponseStatus::NoMatch, false)
    } else {
        (ResponseStatus::Ok, false)
    }
}

fn elapsed_ms(start: Instant) -> u32 {
    start.elapsed().as_millis().min(u32::MAX as u128) as u32
}

struct RankedCandidates {
    candidates: Vec<ProviderCandidate>,
    truncated_count: usize,
}

fn rank_candidates(
    query: &ProviderQuery,
    candidates: Vec<ProviderCandidate>,
    max_suggestions: usize,
) -> RankedCandidates {
    let fragment = query.active_fragment.to_ascii_lowercase();
    let mut ranked: Vec<(u8, u8, Reverse<u16>, String, ProviderCandidate)> = candidates
        .iter()
        .filter_map(|candidate| {
            let display = candidate.display.to_ascii_lowercase();
            let quality = prefix_quality(&display, &fragment)?;

            Some((
                quality,
                position_fitness(query, candidate),
                Reverse(candidate.priority),
                candidate.display.clone(),
                candidate.clone(),
            ))
        })
        .collect();

    if ranked.is_empty() && !fragment.is_empty() {
        ranked = candidates
            .iter()
            .filter_map(|candidate| {
                let display = candidate.display.to_ascii_lowercase();
                if !is_light_fuzzy_match(&display, &fragment) {
                    return None;
                }

                Some((
                    3,
                    position_fitness(query, candidate),
                    Reverse(candidate.priority),
                    candidate.display.clone(),
                    candidate.clone(),
                ))
            })
            .collect();
    }

    ranked.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(left.1.cmp(&right.1))
            .then(left.2.cmp(&right.2))
            .then(left.3.cmp(&right.3))
    });
    let truncated_count = ranked.len().saturating_sub(max_suggestions);
    let candidates = ranked
        .into_iter()
        .take(max_suggestions)
        .map(|(_, _, _, _, candidate)| candidate)
        .collect();

    RankedCandidates {
        candidates,
        truncated_count,
    }
}

fn position_fitness(query: &ProviderQuery, candidate: &ProviderCandidate) -> u8 {
    match query.completion_position {
        crate::parser::CompletionPosition::RootCommand
        | crate::parser::CompletionPosition::Subcommand => match candidate.kind {
            SuggestionKind::Command => 0,
            SuggestionKind::Flag => 1,
            SuggestionKind::Value | SuggestionKind::Help => 2,
        },
        crate::parser::CompletionPosition::Flag => match candidate.kind {
            SuggestionKind::Flag => 0,
            SuggestionKind::Command => 1,
            SuggestionKind::Value | SuggestionKind::Help => 2,
        },
        crate::parser::CompletionPosition::Value => match candidate.kind {
            SuggestionKind::Value => 0,
            SuggestionKind::Command => 1,
            SuggestionKind::Flag => 2,
            SuggestionKind::Help => 3,
        },
    }
}

fn prefix_quality(display: &str, fragment: &str) -> Option<u8> {
    if fragment.is_empty() {
        return Some(2);
    }

    if display == fragment {
        return Some(0);
    }

    if display.starts_with(fragment) {
        return Some(1);
    }

    None
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use crate::artifact::{CompiledProviderArtifact, CompiledProviderMetadata};
    use crate::protocol::{RequestMode, ShellKind, TerminalCapabilities};
    use crate::providers::{
        ArtifactProvider, CatalogError, DynamicValueProvider, Provider, ProviderCatalog,
        ProviderRootIndex, ProviderRootIndexEntry, ProviderSourceKind,
    };
    use crate::spec::{
        CacheMode, CachePolicy, CommandNode, CommandNodeKind, DynamicLookupRequest,
        DynamicLookupResult, DynamicLookupStatus, DynamicValueClass, DynamicValueCost,
        DynamicValueSource, LookupMatch, ProviderCapabilities, ProviderId, SlotId, ValueSource,
        ValueSourceKind, ValueSpec,
    };

    use super::*;

    fn request(buffer: &str, cursor: usize) -> SuggestRequest {
        SuggestRequest {
            shell: ShellKind::Zsh,
            buffer: buffer.to_string(),
            cursor_byte_offset: cursor,
            cwd: PathBuf::from("."),
            env_hints: BTreeMap::new(),
            terminal_capabilities: TerminalCapabilities::default(),
            mode: RequestMode::Suggest,
        }
    }

    struct TestCatalog {
        index: ProviderRootIndex,
        artifacts: BTreeMap<ProviderId, CompiledProviderArtifact>,
        loaded: Arc<Mutex<Vec<ProviderId>>>,
    }

    impl ProviderCatalog for TestCatalog {
        fn root_index(&self) -> &ProviderRootIndex {
            &self.index
        }

        fn load_provider(
            &self,
            provider_id: &ProviderId,
        ) -> Result<Box<dyn Provider>, CatalogError> {
            self.loaded
                .lock()
                .expect("loaded log should be available")
                .push(provider_id.clone());
            let artifact = self
                .artifacts
                .get(provider_id)
                .cloned()
                .ok_or_else(|| CatalogError::UnknownProvider(provider_id.clone()))?;
            Ok(Box::new(ArtifactProvider::from_compiled(artifact)))
        }
    }

    fn catalog() -> (TestCatalog, Arc<Mutex<Vec<ProviderId>>>) {
        let loaded = Arc::new(Mutex::new(Vec::new()));
        let artifacts = BTreeMap::from([
            (
                ProviderId::from("builtin.git"),
                CompiledProviderArtifact {
                    provider: CompiledProviderMetadata {
                        provider_id: ProviderId::from("builtin.git"),
                        description: Some("git fixture".into()),
                        capabilities: ProviderCapabilities {
                            supports_static_commands: true,
                            supports_dynamic_values: false,
                            requires_subprocess: false,
                        },
                    },
                    root: CommandNode {
                        kind: CommandNodeKind::Root,
                        name: "git".into(),
                        summary: None,
                        aliases: Vec::new(),
                        hidden: false,
                        deprecated: false,
                        priority: 100,
                        subcommands: vec![
                            CommandNode {
                                kind: CommandNodeKind::Command,
                                name: "checkout".into(),
                                summary: Some("Switch branches".into()),
                                aliases: vec!["co".into()],
                                hidden: false,
                                deprecated: false,
                                priority: 90,
                                subcommands: Vec::new(),
                                flags: Vec::new(),
                                positional_args: Vec::new(),
                            },
                            CommandNode {
                                kind: CommandNodeKind::Command,
                                name: "cherry-pick".into(),
                                summary: Some("Apply commits".into()),
                                aliases: Vec::new(),
                                hidden: false,
                                deprecated: false,
                                priority: 70,
                                subcommands: Vec::new(),
                                flags: Vec::new(),
                                positional_args: Vec::new(),
                            },
                        ],
                        flags: Vec::new(),
                        positional_args: Vec::new(),
                    },
                },
            ),
            (
                ProviderId::from("builtin.kubectl"),
                CompiledProviderArtifact {
                    provider: CompiledProviderMetadata {
                        provider_id: ProviderId::from("builtin.kubectl"),
                        description: Some("kubectl fixture".into()),
                        capabilities: ProviderCapabilities {
                            supports_static_commands: true,
                            supports_dynamic_values: false,
                            requires_subprocess: false,
                        },
                    },
                    root: CommandNode {
                        kind: CommandNodeKind::Root,
                        name: "kubectl".into(),
                        summary: None,
                        aliases: Vec::new(),
                        hidden: false,
                        deprecated: false,
                        priority: 100,
                        subcommands: vec![
                            CommandNode {
                                kind: CommandNodeKind::Command,
                                name: "config".into(),
                                summary: Some("Modify kubeconfig".into()),
                                aliases: Vec::new(),
                                hidden: false,
                                deprecated: false,
                                priority: 90,
                                subcommands: Vec::new(),
                                flags: Vec::new(),
                                positional_args: Vec::new(),
                            },
                            CommandNode {
                                kind: CommandNodeKind::Command,
                                name: "get".into(),
                                summary: Some("Display resources".into()),
                                aliases: Vec::new(),
                                hidden: false,
                                deprecated: false,
                                priority: 95,
                                subcommands: Vec::new(),
                                flags: Vec::new(),
                                positional_args: Vec::new(),
                            },
                        ],
                        flags: vec![
                            crate::spec::FlagSpec {
                                long: "namespace".into(),
                                short: Some('n'),
                                aliases: Vec::new(),
                                summary: Some("Scope to namespace".into()),
                                hidden: false,
                                deprecated: false,
                                repeatable: false,
                                terminal: false,
                                conflicts_with: Vec::new(),
                                value: None,
                            },
                            crate::spec::FlagSpec {
                                long: "context".into(),
                                short: None,
                                aliases: Vec::new(),
                                summary: Some("Select context".into()),
                                hidden: false,
                                deprecated: false,
                                repeatable: false,
                                terminal: false,
                                conflicts_with: Vec::new(),
                                value: None,
                            },
                        ],
                        positional_args: Vec::new(),
                    },
                },
            ),
        ]);
        let index = ProviderRootIndex::new(vec![
            ProviderRootIndexEntry {
                root_command: "git".into(),
                root_aliases: Vec::new(),
                provider_id: ProviderId::from("builtin.git"),
                source_kind: ProviderSourceKind::EmbeddedBuiltin,
                schema_version: 1,
            },
            ProviderRootIndexEntry {
                root_command: "kubectl".into(),
                root_aliases: vec!["k".into()],
                provider_id: ProviderId::from("builtin.kubectl"),
                source_kind: ProviderSourceKind::EmbeddedBuiltin,
                schema_version: 1,
            },
        ])
        .expect("index should build");

        (
            TestCatalog {
                index,
                artifacts,
                loaded: loaded.clone(),
            },
            loaded,
        )
    }

    #[test]
    fn suggest_returns_ranked_prefix_matches_for_git_subcommands() {
        let (catalog, _) = catalog();
        let engine = SuggestEngine::new(&catalog);
        let response = engine.suggest(&request("git ch", 6));
        assert_eq!(response.status, ResponseStatus::Ok);
        assert_eq!(response.suggestions.len(), 2);
        assert_eq!(response.suggestions[0].display, "checkout");
        assert_eq!(response.suggestions[1].display, "cherry-pick");
        assert_eq!(response.selection_index, Some(0));
    }

    #[test]
    fn suggest_degrades_on_unsupported_syntax() {
        let (catalog, _) = catalog();
        let engine = SuggestEngine::new(&catalog);
        let response = engine.suggest(&request("git $(status)", 13));
        assert_eq!(response.status, ResponseStatus::Degraded);
        assert!(response.suggestions.is_empty());
    }

    #[test]
    fn suggest_only_loads_selected_provider() {
        let (catalog, loaded) = catalog();
        let engine = SuggestEngine::new(&catalog);
        let response = engine.suggest(&request("git ch", 6));
        assert_eq!(response.status, ResponseStatus::Ok);

        let loaded_set: HashSet<_> = loaded
            .lock()
            .expect("loaded log should be available")
            .iter()
            .cloned()
            .collect();
        assert!(loaded_set.contains(&ProviderId::from("builtin.git")));
        assert!(!loaded_set.contains(&ProviderId::from("builtin.kubectl")));
    }

    #[test]
    fn suggest_uses_dynamic_lookup_for_resolved_value_slots() {
        struct DynamicCatalog {
            index: ProviderRootIndex,
        }

        struct DynamicProvider {
            artifact: ArtifactProvider,
        }

        impl Provider for DynamicProvider {
            fn id(&self) -> &ProviderId {
                self.artifact.id()
            }

            fn metadata(&self) -> &CompiledProviderMetadata {
                self.artifact.metadata()
            }

            fn root_summary(&self) -> crate::providers::ProviderRootSummary {
                self.artifact.root_summary()
            }

            fn resolve_scope(&self, query: &ProviderQuery) -> crate::providers::ProviderScope {
                self.artifact.resolve_scope(query)
            }

            fn static_suggestions(
                &self,
                scope: &crate::providers::ProviderScope,
            ) -> Vec<ProviderCandidate> {
                self.artifact.static_suggestions(scope)
            }

            fn value_suggestions(
                &self,
                scope: &crate::providers::ProviderScope,
            ) -> Vec<ProviderCandidate> {
                self.artifact.value_suggestions(scope)
            }

            fn build_dynamic_lookup_request(
                &self,
                query: &ProviderQuery,
                scope: &crate::providers::ProviderScope,
                max_candidates: usize,
            ) -> Option<DynamicLookupRequest> {
                self.artifact
                    .build_dynamic_lookup_request(query, scope, max_candidates)
            }

            fn dynamic_value_provider(&self) -> Option<&dyn DynamicValueProvider> {
                Some(self)
            }
        }

        impl DynamicValueProvider for DynamicProvider {
            fn dynamic_lookup(&self, _request: &DynamicLookupRequest) -> DynamicLookupResult {
                DynamicLookupResult {
                    matches: vec![
                        LookupMatch {
                            value: "pod alpha".into(),
                            display: "pod alpha".into(),
                            annotation: Some("kube-system".into()),
                            confidence: 95,
                            requires_quoting: true,
                            is_stale: false,
                        },
                        LookupMatch {
                            value: "pod-beta".into(),
                            display: "pod-beta".into(),
                            annotation: Some("kube-system".into()),
                            confidence: 90,
                            requires_quoting: false,
                            is_stale: false,
                        },
                    ],
                    cache_status: crate::spec::CacheStatus::Miss,
                    status: DynamicLookupStatus::Complete,
                    degraded: false,
                    lookup_time_ms: 5,
                }
            }
        }

        impl ProviderCatalog for DynamicCatalog {
            fn root_index(&self) -> &ProviderRootIndex {
                &self.index
            }

            fn load_provider(
                &self,
                _provider_id: &ProviderId,
            ) -> Result<Box<dyn Provider>, CatalogError> {
                Ok(Box::new(DynamicProvider {
                    artifact: ArtifactProvider::from_compiled(CompiledProviderArtifact {
                        provider: CompiledProviderMetadata {
                            provider_id: ProviderId::from("builtin.kubectl"),
                            description: Some("dynamic kubectl fixture".into()),
                            capabilities: ProviderCapabilities {
                                supports_static_commands: true,
                                supports_dynamic_values: true,
                                requires_subprocess: true,
                            },
                        },
                        root: CommandNode {
                            kind: CommandNodeKind::Root,
                            name: "kubectl".into(),
                            summary: None,
                            aliases: Vec::new(),
                            hidden: false,
                            deprecated: false,
                            priority: 100,
                            subcommands: vec![CommandNode {
                                kind: CommandNodeKind::Command,
                                name: "describe".into(),
                                summary: Some("Describe resources".into()),
                                aliases: Vec::new(),
                                hidden: false,
                                deprecated: false,
                                priority: 90,
                                subcommands: vec![CommandNode {
                                    kind: CommandNodeKind::Command,
                                    name: "pods".into(),
                                    summary: Some("Describe a pod".into()),
                                    aliases: vec!["pod".into()],
                                    hidden: false,
                                    deprecated: false,
                                    priority: 80,
                                    subcommands: Vec::new(),
                                    flags: Vec::new(),
                                    positional_args: vec![crate::spec::ArgumentSpec {
                                        name: "name".into(),
                                        summary: Some("Pod name".into()),
                                        position: 0,
                                        required: false,
                                        repeatable: false,
                                        value: ValueSpec {
                                            slot_id: SlotId::from("kubectl.describe.pod.name"),
                                            source: ValueSource {
                                                kind: ValueSourceKind::Dynamic,
                                                enum_values: Vec::new(),
                                                dynamic_source: Some(DynamicValueSource {
                                                    lookup_class: DynamicValueClass::ResourceName,
                                                    cache_policy: CachePolicy {
                                                        mode: CacheMode::PreferCache,
                                                        ttl_ms: Some(5_000),
                                                    },
                                                    cost: DynamicValueCost::BoundedSubprocess,
                                                }),
                                            },
                                            quote_style: crate::spec::QuoteStyle::BackslashEscape,
                                        },
                                    }],
                                }],
                                flags: Vec::new(),
                                positional_args: Vec::new(),
                            }],
                            flags: Vec::new(),
                            positional_args: Vec::new(),
                        },
                    }),
                }))
            }
        }

        let catalog = DynamicCatalog {
            index: ProviderRootIndex::new(vec![ProviderRootIndexEntry {
                root_command: "kubectl".into(),
                root_aliases: Vec::new(),
                provider_id: ProviderId::from("builtin.kubectl"),
                source_kind: ProviderSourceKind::EmbeddedBuiltin,
                schema_version: 1,
            }])
            .expect("root index should build"),
        };
        let engine = SuggestEngine::new(&catalog).with_max_suggestions(1);
        let response = engine.suggest(&request("kubectl describe pod po", 23));

        assert_eq!(response.status, ResponseStatus::Ok);
        assert_eq!(response.suggestions.len(), 1);
        assert_eq!(response.suggestions[0].display, "pod alpha");
        assert_eq!(response.suggestions[0].insert_text, "pod\\ alpha");
        assert!(response.suggestions[0].requires_quoting);
        assert_eq!(
            response
                .diagnostics
                .as_ref()
                .and_then(|diagnostics| diagnostics.dynamic_lookup.as_ref())
                .map(|lookup| lookup.slot_id.clone()),
            Some(SlotId::from("kubectl.describe.pod.name"))
        );
    }
}
