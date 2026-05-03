use std::collections::BTreeMap;

use qtpi_core::artifact::PROVIDER_ARTIFACT_SCHEMA_VERSION;
use qtpi_core::providers::{
    ArtifactProvider, CatalogError, Provider, ProviderCatalog, ProviderRootIndex,
    ProviderRootIndexEntry, ProviderSourceKind,
};
use qtpi_core::spec::ProviderId;

use crate::git::GitProvider;
use crate::kubectl::KubectlProvider;

static GIT_MINIMAL_PROVIDER: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/git-minimal.qtpi-provider"));
static KUBECTL_MINIMAL_PROVIDER: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/kubectl-minimal.qtpi-provider"));

pub struct BuiltinCatalog {
    root_index: ProviderRootIndex,
    artifacts: BTreeMap<ProviderId, &'static [u8]>,
}

impl BuiltinCatalog {
    pub fn new() -> Result<Self, CatalogError> {
        let root_index = ProviderRootIndex::new(vec![
            ProviderRootIndexEntry {
                root_command: "git".into(),
                root_aliases: Vec::new(),
                provider_id: ProviderId::from("builtin.git"),
                source_kind: ProviderSourceKind::EmbeddedBuiltin,
                schema_version: PROVIDER_ARTIFACT_SCHEMA_VERSION,
            },
            ProviderRootIndexEntry {
                root_command: "kubectl".into(),
                root_aliases: vec!["k".into()],
                provider_id: ProviderId::from("builtin.kubectl"),
                source_kind: ProviderSourceKind::EmbeddedBuiltin,
                schema_version: PROVIDER_ARTIFACT_SCHEMA_VERSION,
            },
        ])
        .expect("built-in provider root index should be valid");

        Ok(Self {
            root_index,
            artifacts: BTreeMap::from([
                (ProviderId::from("builtin.git"), GIT_MINIMAL_PROVIDER),
                (
                    ProviderId::from("builtin.kubectl"),
                    KUBECTL_MINIMAL_PROVIDER,
                ),
            ]),
        })
    }

    pub fn list_roots(&self) -> &[ProviderRootIndexEntry] {
        self.root_index.entries()
    }
}

impl ProviderCatalog for BuiltinCatalog {
    fn root_index(&self) -> &ProviderRootIndex {
        &self.root_index
    }

    fn load_provider(&self, provider_id: &ProviderId) -> Result<Box<dyn Provider>, CatalogError> {
        let bytes = self
            .artifacts
            .get(provider_id)
            .copied()
            .ok_or_else(|| CatalogError::UnknownProvider(provider_id.clone()))?;
        if provider_id == &ProviderId::from("builtin.git") {
            let provider =
                GitProvider::from_bytes(bytes).map_err(|source| CatalogError::InvalidArtifact {
                    provider_id: provider_id.clone(),
                    source,
                })?;
            Ok(Box::new(provider))
        } else if provider_id == &ProviderId::from("builtin.kubectl") {
            let provider = KubectlProvider::from_bytes(bytes).map_err(|source| {
                CatalogError::InvalidArtifact {
                    provider_id: provider_id.clone(),
                    source,
                }
            })?;
            Ok(Box::new(provider))
        } else {
            let provider = ArtifactProvider::from_bytes(bytes).map_err(|source| {
                CatalogError::InvalidArtifact {
                    provider_id: provider_id.clone(),
                    source,
                }
            })?;
            Ok(Box::new(provider))
        }
    }
}

pub fn builtin_catalog() -> Result<BuiltinCatalog, CatalogError> {
    BuiltinCatalog::new()
}

pub fn builtin_summary(provider_id: &ProviderId) -> Result<(String, usize, usize), CatalogError> {
    let catalog = builtin_catalog()?;
    let provider = catalog.load_provider(provider_id)?;
    let summary = provider.root_summary();
    Ok((
        summary.command_name,
        summary.subcommand_count,
        summary.flag_count,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qtpi_core::artifact::decode_artifact;
    use qtpi_core::engine::SuggestEngine;
    use qtpi_core::protocol::{ShellKind, SuggestRequest};

    #[test]
    fn built_in_catalog_indexes_git_and_kubectl() {
        let catalog = builtin_catalog().expect("catalog should load");
        assert_eq!(catalog.list_roots().len(), 2);
        assert_eq!(
            catalog
                .root_index()
                .exact_match("kubectl")
                .map(|entry| entry.provider_id.clone()),
            Some(ProviderId::from("builtin.kubectl"))
        );
        assert_eq!(
            catalog
                .root_index()
                .exact_match("k")
                .map(|entry| entry.provider_id.clone()),
            Some(ProviderId::from("builtin.kubectl"))
        );
    }

    #[test]
    fn built_in_fixture_registers_on_demand() {
        let catalog = builtin_catalog().expect("catalog should load");
        let provider = catalog
            .load_provider(&ProviderId::from("builtin.git"))
            .expect("git provider should load");
        let summary = provider.root_summary();
        assert_eq!(summary.command_name, "git");
        assert!(summary.subcommand_count >= 20);
    }

    #[test]
    fn unused_provider_bytes_do_not_break_selected_load() {
        let catalog = BuiltinCatalog {
            root_index: ProviderRootIndex::new(vec![
                ProviderRootIndexEntry {
                    root_command: "git".into(),
                    root_aliases: Vec::new(),
                    provider_id: ProviderId::from("builtin.git"),
                    source_kind: ProviderSourceKind::EmbeddedBuiltin,
                    schema_version: PROVIDER_ARTIFACT_SCHEMA_VERSION,
                },
                ProviderRootIndexEntry {
                    root_command: "kubectl".into(),
                    root_aliases: Vec::new(),
                    provider_id: ProviderId::from("builtin.kubectl"),
                    source_kind: ProviderSourceKind::EmbeddedBuiltin,
                    schema_version: PROVIDER_ARTIFACT_SCHEMA_VERSION,
                },
            ])
            .expect("root index should build"),
            artifacts: BTreeMap::from([
                (ProviderId::from("builtin.git"), GIT_MINIMAL_PROVIDER),
                (ProviderId::from("builtin.kubectl"), &[0_u8, 1, 2, 3][..]),
            ]),
        };

        let provider = catalog
            .load_provider(&ProviderId::from("builtin.git"))
            .expect("git provider should still load");
        assert_eq!(provider.root_summary().command_name, "git");
        assert!(decode_artifact(&[0_u8, 1, 2, 3]).is_err());
    }

    #[test]
    fn builtin_git_root_and_prefix_flows_rank_expected_commands() {
        let catalog = builtin_catalog().expect("catalog should load");
        let engine = SuggestEngine::new(&catalog).with_max_suggestions(8);

        let root = engine.suggest(&SuggestRequest::minimal(ShellKind::Zsh, "git ", 4));
        assert!(
            root.suggestions
                .iter()
                .any(|item| item.display == "checkout")
        );
        assert!(root.suggestions.iter().any(|item| item.display == "commit"));

        let narrowed = engine.suggest(&SuggestRequest::minimal(ShellKind::Zsh, "git ch", 6));
        assert_eq!(narrowed.suggestions[0].display, "checkout");
        assert_eq!(narrowed.suggestions[1].display, "cherry-pick");
    }

    #[test]
    fn builtin_kubectl_root_flow_includes_common_families() {
        let catalog = builtin_catalog().expect("catalog should load");
        let engine = SuggestEngine::new(&catalog).with_max_suggestions(8);
        let response = engine.suggest(&SuggestRequest::minimal(ShellKind::Zsh, "kubectl ", 8));

        assert!(
            response
                .suggestions
                .iter()
                .any(|item| item.display == "get")
        );
        assert!(
            response
                .suggestions
                .iter()
                .any(|item| item.display == "describe")
        );
        assert!(
            response
                .suggestions
                .iter()
                .any(|item| item.display == "config")
        );
    }
}
