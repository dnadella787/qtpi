use std::cmp::Reverse;

use crate::parser::{ParserStatus, parse_request};
use crate::protocol::{
    Diagnostics, RenderModel, ResponseStatus, SuggestRequest, SuggestResponse, Suggestion,
    SuggestionKind, TimingDiagnostics,
};
use crate::providers::{ProviderCandidate, ProviderCatalog, ProviderQuery};

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

        let parse = parse_request(request, self.catalog.root_index());

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
                    timings: TimingDiagnostics::default(),
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
                    timings: TimingDiagnostics::default(),
                }),
            };
        };

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
                        timings: TimingDiagnostics::default(),
                    }),
                };
            }
        };

        let scope = provider.resolve_scope(&query);
        let ranked = rank_candidates(
            &query,
            provider.static_suggestions(&scope),
            self.max_suggestions,
        );
        let suggestions: Vec<Suggestion> = ranked
            .candidates
            .iter()
            .map(|candidate| Suggestion {
                insert_text: insert_text_for_candidate(candidate),
                display: candidate.display.clone(),
                annotation: candidate.annotation.clone(),
                kind: candidate.kind,
                requires_quoting: false,
            })
            .collect();
        let status = if suggestions.is_empty() {
            ResponseStatus::NoMatch
        } else {
            ResponseStatus::Ok
        };

        SuggestResponse {
            replace_range: suggestions.first().map(|_| query.replace_range),
            selection_index: if suggestions.is_empty() {
                None
            } else {
                Some(0)
            },
            render_model: RenderModel::from_suggestions(
                &suggestions,
                ranked.truncated_count,
                false,
            ),
            suggestions,
            status,
            diagnostics: Some(Diagnostics {
                provider_id: Some(query.provider_id),
                parser_status: parse.status,
                timings: TimingDiagnostics::default(),
            }),
        }
    }
}

fn insert_text_for_candidate(candidate: &ProviderCandidate) -> String {
    match candidate.kind {
        SuggestionKind::Command | SuggestionKind::Flag => format!("{} ", candidate.replacement),
        SuggestionKind::Value | SuggestionKind::Help => candidate.replacement.clone(),
    }
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
        ArtifactProvider, CatalogError, Provider, ProviderCatalog, ProviderRootIndex,
        ProviderRootIndexEntry, ProviderSourceKind,
    };
    use crate::spec::{CommandNode, CommandNodeKind, ProviderCapabilities, ProviderId};

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
}
