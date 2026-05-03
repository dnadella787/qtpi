use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::artifact::{
    ArtifactDecodeError, CompiledProviderArtifact, CompiledProviderMetadata, decode_artifact,
};
use crate::parser::{CompletionPosition, ParseDegradedReason, ParseOutput};
use crate::protocol::{ReplaceRange, SuggestRequest, SuggestionKind};
use crate::spec::{
    CacheMode, CommandNode, CommandPath, DynamicLookupBudget, DynamicLookupRequest,
    DynamicLookupResult, DynamicLookupScope, FlagSpec, ProviderId, QuoteStyle, SlotId,
    ValueSourceKind, ValueSpec,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderQuery {
    pub provider_id: ProviderId,
    pub command_tokens: Vec<String>,
    pub completion_position: CompletionPosition,
    pub active_fragment: String,
    pub replace_range: ReplaceRange,
    pub active_slot_id: Option<SlotId>,
    pub degraded_parse: Option<ParseDegradedReason>,
    pub cwd: PathBuf,
}

impl ProviderQuery {
    pub fn from_parse(request: &SuggestRequest, parse: &ParseOutput) -> Option<Self> {
        parse.provider_root.as_ref().map(|provider_id| {
            let command_tokens = parse
                .tokens
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(index, _)| Some(*index) != parse.active_token.index)
                .map(|(_, token)| token.text.clone())
                .collect();
            let (active_fragment, replace_range) = parse
                .active_token
                .index
                .and_then(|index| parse.tokens.get(index))
                .map(|token| (token.text.clone(), token.raw_range))
                .unwrap_or((
                    parse.active_token.value.clone(),
                    parse.active_token.raw_range,
                ));

            Self {
                provider_id: provider_id.clone(),
                command_tokens,
                completion_position: parse.completion_position,
                active_fragment,
                replace_range,
                active_slot_id: parse.active_slot_id.clone(),
                degraded_parse: parse.degraded_reason(),
                cwd: request.cwd.clone(),
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderScope {
    pub command_path: CommandPath,
    pub active_slot_id: Option<SlotId>,
    pub active_value: Option<ValueSpec>,
    pub lookup_scope: DynamicLookupScope,
    pub degraded: bool,
    pub used_flags: BTreeSet<String>,
    pub terminal_flag_seen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCandidate {
    pub replacement: String,
    pub display: String,
    pub annotation: Option<String>,
    pub kind: SuggestionKind,
    pub priority: u16,
    pub requires_quoting: bool,
    pub quote_style: QuoteStyle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRootSummary {
    pub command_name: String,
    pub subcommand_count: usize,
    pub flag_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSourceKind {
    EmbeddedBuiltin,
    ExternalArtifact,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRootIndexEntry {
    pub root_command: String,
    pub root_aliases: Vec<String>,
    pub provider_id: ProviderId,
    pub source_kind: ProviderSourceKind,
    pub schema_version: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRootIndex {
    entries: Vec<ProviderRootIndexEntry>,
}

impl ProviderRootIndex {
    pub fn new(entries: Vec<ProviderRootIndexEntry>) -> Result<Self, RootIndexError> {
        let mut seen = BTreeMap::new();
        for entry in &entries {
            for token in std::iter::once(&entry.root_command).chain(entry.root_aliases.iter()) {
                let lowered = token.to_ascii_lowercase();
                if let Some(existing) = seen.insert(lowered.clone(), entry.provider_id.clone()) {
                    return Err(RootIndexError::DuplicateRootToken {
                        token: lowered,
                        existing_provider: existing,
                        new_provider: entry.provider_id.clone(),
                    });
                }
            }
        }

        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[ProviderRootIndexEntry] {
        &self.entries
    }

    pub fn exact_match(&self, token: &str) -> Option<&ProviderRootIndexEntry> {
        self.entries.iter().find(|entry| {
            entry.root_command == token || entry.root_aliases.iter().any(|alias| alias == token)
        })
    }
}

pub trait Provider: Send + Sync {
    fn id(&self) -> &ProviderId;
    fn metadata(&self) -> &CompiledProviderMetadata;
    fn root_summary(&self) -> ProviderRootSummary;
    fn resolve_scope(&self, query: &ProviderQuery) -> ProviderScope;
    fn static_suggestions(&self, scope: &ProviderScope) -> Vec<ProviderCandidate>;
    fn value_suggestions(&self, _scope: &ProviderScope) -> Vec<ProviderCandidate> {
        Vec::new()
    }
    fn build_dynamic_lookup_request(
        &self,
        _query: &ProviderQuery,
        _scope: &ProviderScope,
        _max_candidates: usize,
    ) -> Option<DynamicLookupRequest> {
        None
    }
    fn dynamic_value_provider(&self) -> Option<&dyn DynamicValueProvider> {
        None
    }
}

pub trait DynamicValueProvider: Send + Sync {
    fn dynamic_lookup(&self, request: &crate::spec::DynamicLookupRequest) -> DynamicLookupResult;
}

pub trait ProviderCatalog {
    fn root_index(&self) -> &ProviderRootIndex;
    fn load_provider(&self, provider_id: &ProviderId) -> Result<Box<dyn Provider>, CatalogError>;
}

pub struct ArtifactProvider {
    metadata: CompiledProviderMetadata,
    root: CommandNode,
}

impl ArtifactProvider {
    pub fn from_compiled(artifact: CompiledProviderArtifact) -> Self {
        Self {
            metadata: artifact.provider,
            root: artifact.root,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ArtifactDecodeError> {
        let artifact = decode_artifact(bytes)?;
        Ok(Self::from_compiled(artifact))
    }

    fn root_node(&self) -> &CommandNode {
        &self.root
    }

    fn node_for_path(&self, path: &CommandPath) -> Option<&CommandNode> {
        let mut current = &self.root;
        for segment in path.segments() {
            current = current.subcommands.iter().find(|child| {
                child.name == *segment || child.aliases.iter().any(|alias| alias == segment)
            })?;
        }
        Some(current)
    }
}

impl Provider for ArtifactProvider {
    fn id(&self) -> &ProviderId {
        &self.metadata.provider_id
    }

    fn metadata(&self) -> &CompiledProviderMetadata {
        &self.metadata
    }

    fn root_summary(&self) -> ProviderRootSummary {
        ProviderRootSummary {
            command_name: self.root.name.clone(),
            subcommand_count: self.root.subcommands.len(),
            flag_count: self.root.flags.len(),
        }
    }

    fn resolve_scope(&self, query: &ProviderQuery) -> ProviderScope {
        let mut path = CommandPath::root();
        let mut current = self.root_node();
        let mut token_index = 0;
        let mut positional_index = 0usize;
        let mut pending_flag_value: Option<ValueSpec> = None;
        let mut used_flags = BTreeSet::new();
        let mut terminal_flag_seen = false;

        while token_index < query.command_tokens.len() {
            let token = &query.command_tokens[token_index];
            if let Some(flag) = flag_for_token(current, &self.root, token) {
                used_flags.insert(flag.long.clone());
                terminal_flag_seen |= flag.terminal;
                if let Some(value) = &flag.value {
                    if token_index + 1 >= query.command_tokens.len() {
                        pending_flag_value = Some(value.clone());
                        token_index += 1;
                        break;
                    }

                    token_index += 2;
                } else {
                    token_index += 1;
                }
                continue;
            }

            let Some(next_node) = current.subcommands.iter().find(|child| {
                child.name == *token || child.aliases.iter().any(|alias| alias == token)
            }) else {
                break;
            };

            path.push(next_node.name.clone());
            current = next_node;
            positional_index = 0;
            pending_flag_value = None;
            token_index += 1;
            continue;
        }

        while token_index < query.command_tokens.len() {
            let token = &query.command_tokens[token_index];
            if let Some(flag) = flag_for_token(current, &self.root, token) {
                used_flags.insert(flag.long.clone());
                terminal_flag_seen |= flag.terminal;
                if flag.value.is_some() {
                    token_index = (token_index + 2).min(query.command_tokens.len());
                } else {
                    token_index += 1;
                }
                continue;
            }

            if positional_argument_for_index(current, positional_index).is_none() {
                break;
            }

            positional_index += 1;
            token_index += 1;
        }

        let active_value = if query.completion_position == CompletionPosition::Value {
            pending_flag_value.or_else(|| {
                positional_argument_for_index(current, positional_index)
                    .map(|argument| argument.value.clone())
            })
        } else {
            None
        };

        ProviderScope {
            command_path: path,
            active_slot_id: active_value.as_ref().map(|value| value.slot_id.clone()),
            active_value,
            lookup_scope: DynamicLookupScope {
                namespace: None,
                resource_kind: None,
                profile: None,
                region: None,
                cwd: query.cwd.clone(),
            },
            degraded: query.degraded_parse.is_some(),
            used_flags,
            terminal_flag_seen,
        }
    }

    fn static_suggestions(&self, scope: &ProviderScope) -> Vec<ProviderCandidate> {
        if scope.terminal_flag_seen {
            return Vec::new();
        }

        let Some(node) = self.node_for_path(&scope.command_path) else {
            return Vec::new();
        };

        let subcommands = node
            .subcommands
            .iter()
            .filter(|child| !child.hidden)
            .map(|child| ProviderCandidate {
                replacement: child.name.clone(),
                display: child.name.clone(),
                annotation: child.summary.clone(),
                kind: SuggestionKind::Command,
                priority: child.priority,
                requires_quoting: false,
                quote_style: QuoteStyle::None,
            });
        let flags = node
            .flags
            .iter()
            .filter(|flag| !flag.hidden)
            .filter(|flag| flag.repeatable || !scope.used_flags.contains(&flag.long))
            .filter(|flag| {
                !flag
                    .conflicts_with
                    .iter()
                    .any(|conflict| scope.used_flags.contains(conflict))
            })
            .map(|flag| ProviderCandidate {
                replacement: format!("--{}", flag.long),
                display: format!("--{}", flag.long),
                annotation: flag.summary.clone(),
                kind: SuggestionKind::Flag,
                priority: 40,
                requires_quoting: false,
                quote_style: QuoteStyle::None,
            });

        subcommands.chain(flags).collect()
    }

    fn value_suggestions(&self, scope: &ProviderScope) -> Vec<ProviderCandidate> {
        if scope.terminal_flag_seen {
            return Vec::new();
        }

        let Some(value) = scope.active_value.as_ref() else {
            return Vec::new();
        };

        if value.source.kind != ValueSourceKind::Enum {
            return Vec::new();
        }

        value
            .source
            .enum_values
            .iter()
            .map(|entry| ProviderCandidate {
                replacement: entry.clone(),
                display: entry.clone(),
                annotation: None,
                kind: SuggestionKind::Value,
                priority: 50,
                requires_quoting: value_needs_quoting(entry),
                quote_style: value.quote_style,
            })
            .collect()
    }

    fn build_dynamic_lookup_request(
        &self,
        query: &ProviderQuery,
        scope: &ProviderScope,
        max_candidates: usize,
    ) -> Option<DynamicLookupRequest> {
        if scope.terminal_flag_seen {
            return None;
        }

        let value = scope.active_value.as_ref()?;
        if value.source.kind != ValueSourceKind::Dynamic {
            return None;
        }

        let dynamic_source = value.source.dynamic_source.as_ref()?;
        let allow_stale_cache = dynamic_source.cache_policy.mode != CacheMode::ReadThrough
            && dynamic_source.cache_policy.mode != CacheMode::None;

        Some(DynamicLookupRequest {
            provider_id: self.id().clone(),
            command_path: scope.command_path.clone(),
            slot_id: value.slot_id.clone(),
            partial_input: query.active_fragment.clone(),
            scope: scope.lookup_scope.clone(),
            budget: DynamicLookupBudget {
                timeout_ms: 120,
                max_candidates: max_candidates.min(u16::MAX as usize) as u16,
                allow_subprocess: true,
            },
            allow_stale_cache,
        })
    }
}

#[derive(Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<ProviderId, Box<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<P>(&mut self, provider: P) -> Result<(), RegistryError>
    where
        P: Provider + 'static,
    {
        let provider_id = provider.id().clone();
        if self.providers.contains_key(&provider_id) {
            return Err(RegistryError::DuplicateProvider(provider_id));
        }

        self.providers.insert(provider_id, Box::new(provider));
        Ok(())
    }

    pub fn get(&self, provider_id: &ProviderId) -> Option<&dyn Provider> {
        self.providers.get(provider_id).map(Box::as_ref)
    }

    pub fn provider_ids(&self) -> Vec<ProviderId> {
        self.providers.keys().cloned().collect()
    }

    pub fn load_embedded_artifact(
        &mut self,
        label: &str,
        bytes: &[u8],
    ) -> Result<ProviderId, RegistryError> {
        let provider = ArtifactProvider::from_bytes(bytes).map_err(|source| {
            RegistryError::InvalidArtifact {
                label: label.to_string(),
                source,
            }
        })?;
        let provider_id = provider.id().clone();
        self.insert(provider)?;
        Ok(provider_id)
    }
}

#[derive(Debug)]
pub enum RegistryError {
    DuplicateProvider(ProviderId),
    InvalidArtifact {
        label: String,
        source: ArtifactDecodeError,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum RootIndexError {
    DuplicateRootToken {
        token: String,
        existing_provider: ProviderId,
        new_provider: ProviderId,
    },
}

impl fmt::Display for RootIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateRootToken {
                token,
                existing_provider,
                new_provider,
            } => write!(
                f,
                "duplicate root token {token} for providers {existing_provider} and {new_provider}"
            ),
        }
    }
}

impl Error for RootIndexError {}

#[derive(Debug)]
pub enum CatalogError {
    UnknownProvider(ProviderId),
    InvalidArtifact {
        provider_id: ProviderId,
        source: ArtifactDecodeError,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProvider(provider_id) => write!(f, "unknown provider {provider_id}"),
            Self::InvalidArtifact {
                provider_id,
                source,
            } => write!(f, "failed to load provider {provider_id}: {source}"),
        }
    }
}

impl Error for CatalogError {}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateProvider(provider_id) => {
                write!(f, "provider {provider_id} was already registered")
            }
            Self::InvalidArtifact { label, source } => {
                write!(f, "failed to load embedded artifact {label}: {source}")
            }
        }
    }
}

impl Error for RegistryError {}

fn positional_argument_for_index(
    node: &CommandNode,
    positional_index: usize,
) -> Option<&crate::spec::ArgumentSpec> {
    node.positional_args.get(positional_index).or_else(|| {
        node.positional_args
            .last()
            .filter(|argument| argument.repeatable)
    })
}

fn flag_for_token<'a>(
    current: &'a CommandNode,
    root: &'a CommandNode,
    token: &str,
) -> Option<&'a FlagSpec> {
    node_flag_for_token(current, token).or_else(|| node_flag_for_token(root, token))
}

fn node_flag_for_token<'a>(node: &'a CommandNode, token: &str) -> Option<&'a FlagSpec> {
    node.flags.iter().find(|flag| {
        let long_flag = format!("--{}", flag.long);
        let short_flag = flag.short.map(|short| format!("-{short}"));
        token == long_flag
            || short_flag.as_deref() == Some(token)
            || flag.aliases.iter().any(|alias| token == alias)
    })
}

fn value_needs_quoting(value: &str) -> bool {
    value.chars().any(|ch| {
        ch.is_whitespace()
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{CompiledProviderMetadata, encode_artifact};
    use crate::parser::{ParserStatus, parse_request};
    use crate::protocol::{ReplaceRange, ShellKind, SuggestRequest};
    use crate::providers::{ProviderRootIndex, ProviderRootIndexEntry, ProviderSourceKind};
    use crate::spec::{CommandNodeKind, FlagSpec, ProviderCapabilities};

    fn root_index() -> ProviderRootIndex {
        ProviderRootIndex::new(vec![ProviderRootIndexEntry {
            root_command: "git".into(),
            root_aliases: Vec::new(),
            provider_id: ProviderId::from("builtin.git"),
            source_kind: ProviderSourceKind::EmbeddedBuiltin,
            schema_version: 1,
        }])
        .expect("root index should build")
    }

    fn sample_provider() -> ArtifactProvider {
        ArtifactProvider::from_compiled(CompiledProviderArtifact {
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
                subcommands: vec![CommandNode {
                    kind: CommandNodeKind::Command,
                    name: "status".into(),
                    summary: Some("Show status".into()),
                    aliases: Vec::new(),
                    hidden: false,
                    deprecated: false,
                    priority: 90,
                    subcommands: Vec::new(),
                    flags: Vec::new(),
                    positional_args: Vec::new(),
                }],
                flags: vec![FlagSpec {
                    long: "help".into(),
                    short: Some('h'),
                    aliases: Vec::new(),
                    summary: Some("Print help".into()),
                    hidden: false,
                    deprecated: false,
                    repeatable: false,
                    terminal: true,
                    conflicts_with: Vec::new(),
                    value: None,
                }],
                positional_args: Vec::new(),
            },
        })
    }

    #[test]
    fn provider_query_uses_normalized_parse_output() {
        let request = SuggestRequest::minimal(ShellKind::Zsh, "git st", 6);
        let parse = parse_request(&request, &root_index());
        assert_eq!(parse.status, ParserStatus::Complete);
        let query = ProviderQuery::from_parse(&request, &parse).expect("query should resolve");
        assert_eq!(query.provider_id, ProviderId::from("builtin.git"));
        assert!(query.command_tokens.is_empty());
        assert_eq!(query.active_fragment, "st");
    }

    #[test]
    fn provider_query_opens_new_slot_after_trailing_space() {
        let request = SuggestRequest::minimal(ShellKind::Zsh, "git status ", 11);
        let parse = parse_request(&request, &root_index());
        let query = ProviderQuery::from_parse(&request, &parse).expect("query should resolve");
        assert_eq!(query.command_tokens, vec!["status".to_string()]);
        assert_eq!(query.active_fragment, "");
        assert_eq!(
            query.replace_range,
            ReplaceRange {
                start_byte: 11,
                end_byte: 11,
            }
        );
    }

    #[test]
    fn registry_loads_embedded_artifacts() {
        let provider = sample_provider();
        let bytes = encode_artifact(&CompiledProviderArtifact {
            provider: provider.metadata.clone(),
            root: provider.root.clone(),
        })
        .expect("artifact should encode");

        let mut registry = ProviderRegistry::new();
        let provider_id = registry
            .load_embedded_artifact("git", &bytes)
            .expect("provider should load");
        assert_eq!(provider_id, ProviderId::from("builtin.git"));
        let summary = registry
            .get(&ProviderId::from("builtin.git"))
            .expect("provider should exist")
            .root_summary();
        assert_eq!(summary.command_name, "git");
        assert_eq!(summary.subcommand_count, 1);
    }

    #[test]
    fn root_index_matches_exact_root_tokens() {
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

        assert_eq!(
            index
                .exact_match("kubectl")
                .map(|entry| entry.provider_id.clone()),
            Some(ProviderId::from("builtin.kubectl"))
        );
        assert_eq!(
            index
                .exact_match("k")
                .map(|entry| entry.provider_id.clone()),
            Some(ProviderId::from("builtin.kubectl"))
        );
        assert!(index.exact_match("kub").is_none());
    }

    #[test]
    fn scope_resolution_skips_root_flags_and_their_values() {
        let provider = ArtifactProvider::from_compiled(CompiledProviderArtifact {
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
                subcommands: vec![CommandNode {
                    kind: CommandNodeKind::Command,
                    name: "get".into(),
                    summary: Some("Display resources".into()),
                    aliases: Vec::new(),
                    hidden: false,
                    deprecated: false,
                    priority: 90,
                    subcommands: vec![CommandNode {
                        kind: CommandNodeKind::Command,
                        name: "pods".into(),
                        summary: Some("List pods".into()),
                        aliases: vec!["po".into()],
                        hidden: false,
                        deprecated: false,
                        priority: 80,
                        subcommands: Vec::new(),
                        flags: Vec::new(),
                        positional_args: Vec::new(),
                    }],
                    flags: Vec::new(),
                    positional_args: Vec::new(),
                }],
                flags: vec![FlagSpec {
                    long: "namespace".into(),
                    short: Some('n'),
                    aliases: Vec::new(),
                    summary: Some("Namespace scope".into()),
                    hidden: false,
                    deprecated: false,
                    repeatable: false,
                    terminal: false,
                    conflicts_with: Vec::new(),
                    value: Some(crate::spec::ValueSpec {
                        slot_id: SlotId::from("root.namespace"),
                        source: crate::spec::ValueSource {
                            kind: crate::spec::ValueSourceKind::FreeText,
                            enum_values: Vec::new(),
                            dynamic_source: None,
                        },
                        quote_style: crate::spec::QuoteStyle::BackslashEscape,
                    }),
                }],
                positional_args: Vec::new(),
            },
        });

        let query = ProviderQuery {
            provider_id: ProviderId::from("builtin.kubectl"),
            command_tokens: vec![
                "--namespace".into(),
                "kube-system".into(),
                "get".into(),
                "po".into(),
            ],
            completion_position: CompletionPosition::Value,
            active_fragment: "po".into(),
            replace_range: ReplaceRange {
                start_byte: 30,
                end_byte: 32,
            },
            active_slot_id: None,
            degraded_parse: None,
            cwd: PathBuf::from("."),
        };

        let scope = provider.resolve_scope(&query);
        assert_eq!(
            scope.command_path,
            CommandPath(vec!["get".to_string(), "pods".to_string()])
        );
    }

    #[test]
    fn scope_resolution_honors_flag_aliases() {
        let mut provider = sample_provider();
        provider.root.flags.push(FlagSpec {
            long: "porcelain".into(),
            short: None,
            aliases: vec!["--machine-readable".into()],
            summary: Some("Machine readable output".into()),
            hidden: false,
            deprecated: false,
            repeatable: false,
            terminal: false,
            conflicts_with: Vec::new(),
            value: None,
        });

        let query = ProviderQuery {
            provider_id: ProviderId::from("builtin.git"),
            command_tokens: vec!["--machine-readable".into(), "status".into()],
            completion_position: CompletionPosition::Value,
            active_fragment: String::new(),
            replace_range: ReplaceRange {
                start_byte: 30,
                end_byte: 30,
            },
            active_slot_id: None,
            degraded_parse: None,
            cwd: PathBuf::from("."),
        };

        let scope = provider.resolve_scope(&query);
        assert_eq!(scope.command_path, CommandPath(vec!["status".to_string()]));
    }

    #[test]
    fn static_suggestions_skip_non_repeatable_flags_that_are_already_used() {
        let provider = ArtifactProvider::from_compiled(CompiledProviderArtifact {
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
                subcommands: vec![CommandNode {
                    kind: CommandNodeKind::Command,
                    name: "status".into(),
                    summary: Some("Show status".into()),
                    aliases: Vec::new(),
                    hidden: false,
                    deprecated: false,
                    priority: 90,
                    subcommands: Vec::new(),
                    flags: vec![FlagSpec {
                        long: "short".into(),
                        short: Some('s'),
                        aliases: Vec::new(),
                        summary: Some("Short format".into()),
                        hidden: false,
                        deprecated: false,
                        repeatable: false,
                        terminal: false,
                        conflicts_with: Vec::new(),
                        value: None,
                    }],
                    positional_args: Vec::new(),
                }],
                flags: Vec::new(),
                positional_args: Vec::new(),
            },
        });

        let query = ProviderQuery {
            provider_id: ProviderId::from("builtin.git"),
            command_tokens: vec!["status".into(), "--short".into()],
            completion_position: CompletionPosition::Value,
            active_fragment: String::new(),
            replace_range: ReplaceRange {
                start_byte: 18,
                end_byte: 18,
            },
            active_slot_id: None,
            degraded_parse: None,
            cwd: PathBuf::from("."),
        };

        let scope = provider.resolve_scope(&query);
        let suggestions = provider.static_suggestions(&scope);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn terminal_flags_stop_further_static_suggestions() {
        let provider = sample_provider();
        let query = ProviderQuery {
            provider_id: ProviderId::from("builtin.git"),
            command_tokens: vec!["--help".into(), "status".into()],
            completion_position: CompletionPosition::Value,
            active_fragment: String::new(),
            replace_range: ReplaceRange {
                start_byte: 18,
                end_byte: 18,
            },
            active_slot_id: None,
            degraded_parse: None,
            cwd: PathBuf::from("."),
        };

        let scope = provider.resolve_scope(&query);
        assert!(scope.terminal_flag_seen);
        assert!(provider.static_suggestions(&scope).is_empty());
    }
}
