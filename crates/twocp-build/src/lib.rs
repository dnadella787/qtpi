use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use twocp_core::artifact::{CompiledProviderArtifact, CompiledProviderMetadata, encode_artifact};
use twocp_core::spec::{
    ArgumentSpec, CommandNode, CommandNodeKind, FlagSpec, ProviderCapabilities, ProviderId,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSourceFile {
    pub provider_id: ProviderId,
    pub description: Option<String>,
    pub capabilities: ProviderCapabilities,
    pub root: CommandNode,
}

pub fn compile_source_file(source: ProviderSourceFile) -> Result<CompiledProviderArtifact> {
    validate_provider_source(&source)?;

    let mut root = source.root;
    normalize_command_node(&mut root);

    Ok(CompiledProviderArtifact {
        provider: CompiledProviderMetadata {
            provider_id: source.provider_id,
            description: source.description,
            capabilities: source.capabilities,
        },
        root,
    })
}

pub fn compile_json_bytes(source_bytes: &[u8]) -> Result<Vec<u8>> {
    let source: ProviderSourceFile =
        serde_json::from_slice(source_bytes).context("failed to parse provider source json")?;
    let artifact = compile_source_file(source)?;
    encode_artifact(&artifact).context("failed to encode provider artifact")
}

pub fn compile_json_file_to_file(input: &Path, output: &Path) -> Result<()> {
    let source_bytes = fs::read(input)
        .with_context(|| format!("failed to read provider source {}", input.display()))?;
    let artifact_bytes = compile_json_bytes(&source_bytes)?;

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create artifact output directory {}",
                parent.display()
            )
        })?;
    }

    fs::write(output, artifact_bytes)
        .with_context(|| format!("failed to write provider artifact {}", output.display()))?;
    Ok(())
}

fn validate_provider_source(source: &ProviderSourceFile) -> Result<()> {
    if source.provider_id.is_empty() {
        bail!("provider_id must not be empty");
    }

    if source.root.kind != CommandNodeKind::Root {
        bail!("provider root node must use kind=root");
    }

    if source.root.name.is_empty() {
        bail!("provider root node name must not be empty");
    }

    validate_command_node(&source.root, source.provider_id.as_str())
}

fn validate_command_node(node: &CommandNode, path: &str) -> Result<()> {
    let mut seen_subcommands = BTreeSet::new();
    for child in &node.subcommands {
        for name in std::iter::once(&child.name).chain(child.aliases.iter()) {
            if !seen_subcommands.insert(name.clone()) {
                bail!("duplicate subcommand or alias {name} under {path}");
            }
        }
        let child_path = format!("{path} {}", child.name);
        validate_command_node(child, &child_path)?;
    }

    let mut seen_long_flags = BTreeSet::new();
    let mut seen_short_flags = BTreeSet::new();
    for flag in &node.flags {
        validate_flag(flag, path, &mut seen_long_flags, &mut seen_short_flags)?;
    }

    let mut seen_positions = BTreeSet::new();
    for argument in &node.positional_args {
        validate_argument(argument, path, &mut seen_positions)?;
    }

    Ok(())
}

fn validate_flag(
    flag: &FlagSpec,
    path: &str,
    seen_long_flags: &mut BTreeSet<String>,
    seen_short_flags: &mut BTreeSet<char>,
) -> Result<()> {
    if !seen_long_flags.insert(flag.long.clone()) {
        bail!("duplicate flag --{} under {path}", flag.long);
    }

    for alias in &flag.aliases {
        if !seen_long_flags.insert(alias.clone()) {
            bail!("duplicate flag alias {alias} under {path}");
        }
    }

    if let Some(short) = flag.short {
        if !seen_short_flags.insert(short) {
            bail!("duplicate short flag -{short} under {path}");
        }
    }

    Ok(())
}

fn validate_argument(
    argument: &ArgumentSpec,
    path: &str,
    seen_positions: &mut BTreeSet<u16>,
) -> Result<()> {
    if !seen_positions.insert(argument.position) {
        bail!(
            "duplicate positional argument position {} under {path}",
            argument.position
        );
    }

    Ok(())
}

fn normalize_command_node(node: &mut CommandNode) {
    node.aliases.sort();
    node.aliases.dedup();

    for child in &mut node.subcommands {
        normalize_command_node(child);
    }

    for flag in &mut node.flags {
        flag.aliases.sort();
        flag.aliases.dedup();
        flag.conflicts_with.sort();
        flag.conflicts_with.dedup();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;
    use twocp_core::artifact::decode_artifact;

    use super::*;

    fn fixture_json() -> &'static [u8] {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../providers-src/git-minimal.json"
        ))
    }

    #[test]
    fn provider_artifacts_compile_deterministically_from_json() {
        let first = compile_json_bytes(fixture_json()).expect("first compile should pass");
        let second = compile_json_bytes(fixture_json()).expect("second compile should pass");
        assert_eq!(first, second);

        let artifact = decode_artifact(&first).expect("artifact should decode");
        assert_eq!(
            artifact.provider.provider_id,
            ProviderId::from("builtin.git")
        );
        assert_eq!(artifact.root.name, "git");
    }

    #[test]
    fn provider_artifacts_write_to_output_files() {
        let tempdir = tempdir().expect("tempdir should exist");
        let input = tempdir.path().join("git.json");
        let output = tempdir
            .path()
            .join(PathBuf::from("nested/git.twocp-provider"));
        fs::write(&input, fixture_json()).expect("fixture should write");

        compile_json_file_to_file(&input, &output).expect("compile should pass");
        let bytes = fs::read(output).expect("artifact should exist");
        let artifact = decode_artifact(&bytes).expect("artifact should decode");
        assert_eq!(
            artifact.provider.provider_id,
            ProviderId::from("builtin.git")
        );
    }

    #[test]
    fn provider_compiler_rejects_duplicate_flag_aliases() {
        let mut source: ProviderSourceFile =
            serde_json::from_slice(fixture_json()).expect("fixture should parse");
        source.root.flags.push(FlagSpec {
            long: "help".into(),
            short: None,
            aliases: Vec::new(),
            summary: None,
            hidden: false,
            deprecated: false,
            repeatable: false,
            conflicts_with: Vec::new(),
            value: None,
        });

        let error = compile_source_file(source).expect_err("duplicate flag should fail");
        assert!(error.to_string().contains("duplicate flag"));
    }
}
