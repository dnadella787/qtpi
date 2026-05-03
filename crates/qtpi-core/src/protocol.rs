use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::parser::ParserStatus;
use crate::spec::{CacheStatus, DynamicLookupScope, DynamicLookupStatus, ProviderId, SlotId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellKind {
    Zsh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestMode {
    Suggest,
    Accept,
    Dismiss,
    Doctor,
    Debug,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCapabilities {
    pub color: bool,
    pub cursor_movement: bool,
    pub terminal_width: Option<u16>,
    pub terminal_height: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestRequest {
    pub shell: ShellKind,
    pub buffer: String,
    pub cursor_byte_offset: usize,
    pub cwd: PathBuf,
    pub env_hints: BTreeMap<String, String>,
    pub terminal_capabilities: TerminalCapabilities,
    pub mode: RequestMode,
}

impl SuggestRequest {
    pub fn minimal(shell: ShellKind, buffer: impl Into<String>, cursor_byte_offset: usize) -> Self {
        Self {
            shell,
            buffer: buffer.into(),
            cursor_byte_offset,
            cwd: PathBuf::from("."),
            env_hints: BTreeMap::new(),
            terminal_capabilities: TerminalCapabilities::default(),
            mode: RequestMode::Suggest,
        }
    }

    pub fn validate(&self) -> Result<(), RequestValidationError> {
        if self.cursor_byte_offset > self.buffer.len() {
            return Err(RequestValidationError::CursorOutOfBounds {
                cursor_byte_offset: self.cursor_byte_offset,
                buffer_len: self.buffer.len(),
            });
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaceRange {
    pub start_byte: usize,
    pub end_byte: usize,
}

impl ReplaceRange {
    pub fn new(start_byte: usize, end_byte: usize) -> Result<Self, ReplaceRangeError> {
        if start_byte > end_byte {
            return Err(ReplaceRangeError {
                start_byte,
                end_byte,
            });
        }

        Ok(Self {
            start_byte,
            end_byte,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    Command,
    Flag,
    Value,
    Help,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    pub insert_text: String,
    pub display: String,
    pub annotation: Option<String>,
    pub kind: SuggestionKind,
    pub requires_quoting: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderRow {
    pub primary: String,
    pub secondary: Option<String>,
    pub kind: SuggestionKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderModel {
    pub rows: Vec<RenderRow>,
    pub truncated_count: usize,
    pub degraded: bool,
}

impl RenderModel {
    pub fn from_suggestions(
        suggestions: &[Suggestion],
        truncated_count: usize,
        degraded: bool,
    ) -> Self {
        let rows = suggestions
            .iter()
            .map(|suggestion| RenderRow {
                primary: suggestion.display.clone(),
                secondary: suggestion.annotation.clone(),
                kind: suggestion.kind,
            })
            .collect();

        Self {
            rows,
            truncated_count,
            degraded,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Ok,
    NoMatch,
    Degraded,
    Error,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingDiagnostics {
    pub parse_ms: u32,
    pub provider_ms: u32,
    pub dynamic_lookup_ms: u32,
    pub total_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicLookupDiagnostics {
    pub slot_id: SlotId,
    pub scope: DynamicLookupScope,
    pub cache_status: CacheStatus,
    pub status: DynamicLookupStatus,
    pub match_count: usize,
    pub lookup_time_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostics {
    pub provider_id: Option<ProviderId>,
    pub parser_status: ParserStatus,
    pub timings: TimingDiagnostics,
    pub dynamic_lookup: Option<DynamicLookupDiagnostics>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestResponse {
    pub replace_range: Option<ReplaceRange>,
    pub suggestions: Vec<Suggestion>,
    pub selection_index: Option<usize>,
    pub render_model: RenderModel,
    pub status: ResponseStatus,
    pub diagnostics: Option<Diagnostics>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RequestValidationError {
    CursorOutOfBounds {
        cursor_byte_offset: usize,
        buffer_len: usize,
    },
}

impl fmt::Display for RequestValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CursorOutOfBounds {
                cursor_byte_offset,
                buffer_len,
            } => write!(
                f,
                "cursor byte offset {cursor_byte_offset} is outside buffer length {buffer_len}"
            ),
        }
    }
}

impl Error for RequestValidationError {}

#[derive(Debug, PartialEq, Eq)]
pub struct ReplaceRangeError {
    start_byte: usize,
    end_byte: usize,
}

impl fmt::Display for ReplaceRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "replace range start {} is after end {}",
            self.start_byte, self.end_byte
        )
    }
}

impl Error for ReplaceRangeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_validation_rejects_cursor_past_end() {
        let request = SuggestRequest::minimal(ShellKind::Zsh, "git", 8);
        let error = request.validate().expect_err("request should fail");
        assert_eq!(
            error,
            RequestValidationError::CursorOutOfBounds {
                cursor_byte_offset: 8,
                buffer_len: 3,
            }
        );
    }

    #[test]
    fn render_model_tracks_rows_and_degraded_state() {
        let suggestions = vec![Suggestion {
            insert_text: "checkout".into(),
            display: "checkout".into(),
            annotation: Some("Switch branches".into()),
            kind: SuggestionKind::Command,
            requires_quoting: false,
        }];

        let render_model = RenderModel::from_suggestions(&suggestions, 2, true);
        assert_eq!(render_model.rows.len(), 1);
        assert_eq!(render_model.truncated_count, 2);
        assert!(render_model.degraded);
    }

    #[test]
    fn replace_range_rejects_inverted_ranges() {
        let error = ReplaceRange::new(4, 2).expect_err("range should fail");
        assert_eq!(
            error,
            ReplaceRangeError {
                start_byte: 4,
                end_byte: 2,
            }
        );
    }
}
