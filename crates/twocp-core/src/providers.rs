use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::artifact::{
    ArtifactDecodeError, CompiledProviderArtifact, CompiledProviderMetadata, decode_artifact,
};
use crate::parser::{CompletionPosition, ParseDegradedReason, ParseOutput};
use crate::protocol::{ReplaceRange, SuggestRequest, SuggestionKind};
use crate::spec::{CommandNode, CommandPath, DynamicLookupResult, ProviderId, SlotId};

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
            let active_token_index = parse.active_token.index;
            let trailing_unresolved_index =
                unresolved_trailing_token_index(parse, active_token_index);
            let active_index = active_token_index.or(trailing_unresolved_index);
            let command_tokens = parse
                .tokens
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(index, _)| Some(*index) != active_index)
                .map(|(_, token)| token.text.clone())
                .collect();
            let (active_fragment, replace_range) = active_index
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
    pub degraded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCandidate {
    pub replacement: String,
    pub display: String,
    pub annotation: Option<String>,
    pub kind: SuggestionKind,
    pub priority: u16,
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

        while token_index < query.command_tokens.len() {
            let token = &query.command_tokens[token_index];
            if token.starts_with('-') {
                if flag_expects_value(current, &self.root, token) {
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
            token_index += 1;
        }

        ProviderScope {
            command_path: path,
            active_slot_id: query.active_slot_id.clone(),
            degraded: query.degraded_parse.is_some(),
        }
    }

    fn static_suggestions(&self, scope: &ProviderScope) -> Vec<ProviderCandidate> {
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
            });
        let flags = node
            .flags
            .iter()
            .filter(|flag| !flag.hidden)
            .map(|flag| ProviderCandidate {
                replacement: format!("--{}", flag.long),
                display: format!("--{}", flag.long),
                annotation: flag.summary.clone(),
                kind: SuggestionKind::Flag,
                priority: 40,
            });

        subcommands.chain(flags).collect()
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

fn unresolved_trailing_token_index(
    parse: &ParseOutput,
    active_token_index: Option<usize>,
) -> Option<usize> {
    if active_token_index.is_some() {
        return None;
    }

    let trailing_index = parse.tokens.len().checked_sub(1)?;
    if trailing_index == 0 {
        return None;
    }

    let trailing = &parse.tokens[trailing_index];
    if trailing.text.starts_with('-') {
        return None;
    }

    Some(trailing_index)
}

fn flag_expects_value(current: &CommandNode, root: &CommandNode, token: &str) -> bool {
    node_flag_expects_value(current, token) || node_flag_expects_value(root, token)
}

fn node_flag_expects_value(node: &CommandNode, token: &str) -> bool {
    node.flags.iter().any(|flag| {
        let long_flag = format!("--{}", flag.long);
        let short_flag = flag.short.map(|short| format!("-{short}"));
        (token == long_flag || short_flag.as_deref() == Some(token)) && flag.value.is_some()
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
    fn provider_query_repairs_trailing_unresolved_token_after_space() {
        let request = SuggestRequest::minimal(ShellKind::Zsh, "git ch ", 7);
        let parse = parse_request(&request, &root_index());
        let query = ProviderQuery::from_parse(&request, &parse).expect("query should resolve");
        assert_eq!(query.active_fragment, "ch");
        assert_eq!(
            query.replace_range,
            ReplaceRange {
                start_byte: 4,
                end_byte: 6,
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
}
