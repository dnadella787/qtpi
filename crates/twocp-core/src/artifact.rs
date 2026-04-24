use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::spec::{CommandNode, ProviderCapabilities, ProviderId};

pub const PROVIDER_ARTIFACT_MAGIC: [u8; 4] = *b"2CPA";
pub const PROVIDER_ARTIFACT_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledProviderMetadata {
    pub provider_id: ProviderId,
    pub description: Option<String>,
    pub capabilities: ProviderCapabilities,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledProviderArtifact {
    pub provider: CompiledProviderMetadata,
    pub root: CommandNode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ArtifactEnvelope {
    magic: [u8; 4],
    schema_version: u16,
    provider_id: ProviderId,
    artifact: CompiledProviderArtifact,
}

pub fn encode_artifact(artifact: &CompiledProviderArtifact) -> Result<Vec<u8>, postcard::Error> {
    let envelope = ArtifactEnvelope {
        magic: PROVIDER_ARTIFACT_MAGIC,
        schema_version: PROVIDER_ARTIFACT_SCHEMA_VERSION,
        provider_id: artifact.provider.provider_id.clone(),
        artifact: artifact.clone(),
    };

    postcard::to_allocvec(&envelope)
}

pub fn decode_artifact(bytes: &[u8]) -> Result<CompiledProviderArtifact, ArtifactDecodeError> {
    let envelope = postcard::from_bytes::<ArtifactEnvelope>(bytes)
        .map_err(ArtifactDecodeError::PostcardDecode)?;

    if envelope.magic != PROVIDER_ARTIFACT_MAGIC {
        return Err(ArtifactDecodeError::WrongMagic(envelope.magic));
    }

    if envelope.schema_version != PROVIDER_ARTIFACT_SCHEMA_VERSION {
        return Err(ArtifactDecodeError::UnsupportedSchemaVersion(
            envelope.schema_version,
        ));
    }

    if envelope.provider_id != envelope.artifact.provider.provider_id {
        return Err(ArtifactDecodeError::ProviderIdMismatch {
            envelope: envelope.provider_id,
            artifact: envelope.artifact.provider.provider_id,
        });
    }

    Ok(envelope.artifact)
}

#[derive(Debug)]
pub enum ArtifactDecodeError {
    PostcardDecode(postcard::Error),
    WrongMagic([u8; 4]),
    UnsupportedSchemaVersion(u16),
    ProviderIdMismatch {
        envelope: ProviderId,
        artifact: ProviderId,
    },
}

impl fmt::Display for ArtifactDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PostcardDecode(error) => write!(f, "failed to decode provider artifact: {error}"),
            Self::WrongMagic(magic) => write!(f, "unexpected provider artifact magic: {magic:?}"),
            Self::UnsupportedSchemaVersion(version) => {
                write!(f, "unsupported provider artifact schema version: {version}")
            }
            Self::ProviderIdMismatch { envelope, artifact } => write!(
                f,
                "artifact provider id mismatch between envelope ({envelope}) and payload ({artifact})"
            ),
        }
    }
}

impl Error for ArtifactDecodeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{CommandNodeKind, ProviderCapabilities};

    fn sample_artifact() -> CompiledProviderArtifact {
        CompiledProviderArtifact {
            provider: CompiledProviderMetadata {
                provider_id: ProviderId::from("git"),
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
                subcommands: Vec::new(),
                flags: Vec::new(),
                positional_args: Vec::new(),
            },
        }
    }

    #[test]
    fn artifact_round_trip_preserves_payload() {
        let artifact = sample_artifact();
        let bytes = encode_artifact(&artifact).expect("artifact should encode");
        let decoded = decode_artifact(&bytes).expect("artifact should decode");
        assert_eq!(decoded, artifact);
    }

    #[test]
    fn artifact_rejects_unsupported_schema_versions() {
        let artifact = sample_artifact();
        let envelope = ArtifactEnvelope {
            magic: PROVIDER_ARTIFACT_MAGIC,
            schema_version: PROVIDER_ARTIFACT_SCHEMA_VERSION + 1,
            provider_id: artifact.provider.provider_id.clone(),
            artifact,
        };
        let bytes = postcard::to_allocvec(&envelope).expect("envelope should encode");
        let error = decode_artifact(&bytes).expect_err("artifact should fail");
        assert!(matches!(
            error,
            ArtifactDecodeError::UnsupportedSchemaVersion(_)
        ));
    }

    #[test]
    fn artifact_rejects_corrupt_bytes() {
        let error = decode_artifact(&[1, 2, 3, 4]).expect_err("artifact should fail");
        assert!(matches!(error, ArtifactDecodeError::PostcardDecode(_)));
    }
}
