use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<&str> for ProviderId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SlotId(String);

impl SlotId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SlotId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommandPath(pub Vec<String>);

impl CommandPath {
    pub fn root() -> Self {
        Self(Vec::new())
    }

    pub fn push(&mut self, segment: impl Into<String>) {
        self.0.push(segment.into());
    }

    pub fn segments(&self) -> &[String] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandNodeKind {
    Root,
    Command,
    ActionGroup,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandNode {
    pub kind: CommandNodeKind,
    pub name: String,
    pub summary: Option<String>,
    pub aliases: Vec<String>,
    pub hidden: bool,
    pub deprecated: bool,
    pub priority: u16,
    pub subcommands: Vec<CommandNode>,
    pub flags: Vec<FlagSpec>,
    pub positional_args: Vec<ArgumentSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagSpec {
    pub long: String,
    pub short: Option<char>,
    pub aliases: Vec<String>,
    pub summary: Option<String>,
    pub hidden: bool,
    pub deprecated: bool,
    pub repeatable: bool,
    pub conflicts_with: Vec<String>,
    pub value: Option<ValueSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgumentSpec {
    pub name: String,
    pub summary: Option<String>,
    pub position: u16,
    pub required: bool,
    pub repeatable: bool,
    pub value: ValueSpec,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueSpec {
    pub slot_id: SlotId,
    pub source: ValueSource,
    pub quote_style: QuoteStyle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueSource {
    pub kind: ValueSourceKind,
    pub enum_values: Vec<String>,
    pub dynamic_source: Option<DynamicValueSource>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueSourceKind {
    FreeText,
    Enum,
    Dynamic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteStyle {
    None,
    SingleQuotes,
    DoubleQuotes,
    BackslashEscape,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicValueSource {
    pub lookup_class: DynamicValueClass,
    pub cache_policy: CachePolicy,
    pub cost: DynamicValueCost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicValueClass {
    Branch,
    Namespace,
    ResourceName,
    Profile,
    Region,
    Context,
    RepositoryPath,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachePolicy {
    pub mode: CacheMode,
    pub ttl_ms: Option<u32>,
}

impl CachePolicy {
    pub fn disabled() -> Self {
        Self {
            mode: CacheMode::None,
            ttl_ms: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    None,
    ReadThrough,
    PreferCache,
    CacheRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicValueCost {
    CheapSynchronous,
    BoundedSubprocess,
    CacheRequired,
    UnavailableInDegradedMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicLookupRequest {
    pub provider_id: ProviderId,
    pub command_path: CommandPath,
    pub slot_id: SlotId,
    pub partial_input: String,
    pub scope: DynamicLookupScope,
    pub budget: DynamicLookupBudget,
    pub allow_stale_cache: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicLookupScope {
    pub namespace: Option<String>,
    pub resource_kind: Option<String>,
    pub profile: Option<String>,
    pub region: Option<String>,
    pub cwd: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicLookupBudget {
    pub timeout_ms: u32,
    pub max_candidates: u16,
    pub allow_subprocess: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookupMatch {
    pub value: String,
    pub display: String,
    pub annotation: Option<String>,
    pub confidence: u16,
    pub requires_quoting: bool,
    pub is_stale: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheStatus {
    NotChecked,
    HitFresh,
    HitStale,
    Miss,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicLookupStatus {
    Complete,
    NoMatch,
    Unsupported,
    BudgetExceeded,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicLookupResult {
    pub matches: Vec<LookupMatch>,
    pub cache_status: CacheStatus,
    pub status: DynamicLookupStatus,
    pub degraded: bool,
    pub lookup_time_ms: u32,
}

impl DynamicLookupResult {
    pub fn unsupported() -> Self {
        Self {
            matches: Vec::new(),
            cache_status: CacheStatus::Unsupported,
            status: DynamicLookupStatus::Unsupported,
            degraded: false,
            lookup_time_ms: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub supports_static_commands: bool,
    pub supports_dynamic_values: bool,
    pub requires_subprocess: bool,
}
