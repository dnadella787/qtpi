use serde::{Deserialize, Serialize};

use crate::protocol::{ReplaceRange, SuggestRequest};
use crate::providers::ProviderRootIndex;
use crate::spec::{ProviderId, SlotId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteContext {
    Unquoted,
    SingleQuoted,
    DoubleQuoted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedSyntax {
    Pipe,
    LogicalAnd,
    LogicalOr,
    Background,
    Semicolon,
    Redirection,
    CommandSubstitution,
    ProcessSubstitution,
    BacktickSubstitution,
    ParameterExpansion,
    Comment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseDegradedReason {
    CursorOutOfBounds,
    MultilineBuffer,
    UnterminatedQuote(QuoteContext),
    TrailingEscape,
    UnsupportedSyntax(UnsupportedSyntax),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserStatus {
    Complete,
    Degraded(ParseDegradedReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionPosition {
    RootCommand,
    Subcommand,
    Flag,
    Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedToken {
    pub text: String,
    pub raw_range: ReplaceRange,
    pub quote_context: QuoteContext,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveToken {
    pub index: Option<usize>,
    pub value: String,
    pub raw_range: ReplaceRange,
    pub quote_context: QuoteContext,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseOutput {
    pub status: ParserStatus,
    pub tokens: Vec<ParsedToken>,
    pub active_token: ActiveToken,
    pub completion_position: CompletionPosition,
    pub provider_root: Option<ProviderId>,
    pub active_slot_id: Option<SlotId>,
}

impl ParseOutput {
    pub fn degraded(reason: ParseDegradedReason, cursor_byte_offset: usize) -> Self {
        let raw_range = ReplaceRange {
            start_byte: cursor_byte_offset,
            end_byte: cursor_byte_offset,
        };

        Self {
            status: ParserStatus::Degraded(reason),
            tokens: Vec::new(),
            active_token: ActiveToken {
                index: None,
                value: String::new(),
                raw_range,
                quote_context: QuoteContext::Unquoted,
            },
            completion_position: CompletionPosition::RootCommand,
            provider_root: None,
            active_slot_id: None,
        }
    }

    pub fn degraded_reason(&self) -> Option<ParseDegradedReason> {
        match self.status {
            ParserStatus::Complete => None,
            ParserStatus::Degraded(reason) => Some(reason),
        }
    }
}

struct TokenBuilder {
    start_byte: usize,
    text: String,
    quote_context: QuoteContext,
}

impl TokenBuilder {
    fn new(start_byte: usize, quote_context: QuoteContext) -> Self {
        Self {
            start_byte,
            text: String::new(),
            quote_context,
        }
    }

    fn into_token(self, end_byte: usize) -> ParsedToken {
        ParsedToken {
            text: self.text,
            raw_range: ReplaceRange {
                start_byte: self.start_byte,
                end_byte,
            },
            quote_context: self.quote_context,
        }
    }
}

pub fn parse_request(request: &SuggestRequest, root_index: &ProviderRootIndex) -> ParseOutput {
    if request.cursor_byte_offset > request.buffer.len() {
        return ParseOutput::degraded(ParseDegradedReason::CursorOutOfBounds, request.buffer.len());
    }

    if !request.buffer.is_char_boundary(request.cursor_byte_offset) {
        return ParseOutput::degraded(
            ParseDegradedReason::CursorOutOfBounds,
            request.cursor_byte_offset,
        );
    }

    if request.buffer.contains('\n') || request.buffer.contains('\r') {
        return ParseOutput::degraded(
            ParseDegradedReason::MultilineBuffer,
            request.cursor_byte_offset,
        );
    }

    let parse_buffer = &request.buffer[..request.cursor_byte_offset];
    let mut tokens = Vec::new();
    let mut current: Option<TokenBuilder> = None;
    let mut quote_context = QuoteContext::Unquoted;
    let mut index = 0;

    while index < parse_buffer.len() {
        let current_slice = &parse_buffer[index..];
        let ch = current_slice
            .chars()
            .next()
            .expect("slice at valid index should have a char");
        let ch_len = ch.len_utf8();
        let next_char = parse_buffer[index + ch_len..].chars().next();

        if quote_context == QuoteContext::Unquoted {
            match ch {
                '`' => {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(
                            UnsupportedSyntax::BacktickSubstitution,
                        ),
                        request.cursor_byte_offset,
                    );
                }
                '$' if next_char == Some('(') => {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(
                            UnsupportedSyntax::CommandSubstitution,
                        ),
                        request.cursor_byte_offset,
                    );
                }
                '$' if next_char
                    .map(|next| next == '{' || next == '_' || next.is_ascii_alphanumeric())
                    .unwrap_or(false) =>
                {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(
                            UnsupportedSyntax::ParameterExpansion,
                        ),
                        request.cursor_byte_offset,
                    );
                }
                '<' | '>' if next_char == Some('(') => {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(
                            UnsupportedSyntax::ProcessSubstitution,
                        ),
                        request.cursor_byte_offset,
                    );
                }
                '|' if next_char == Some('|') => {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(UnsupportedSyntax::LogicalOr),
                        request.cursor_byte_offset,
                    );
                }
                '|' => {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(UnsupportedSyntax::Pipe),
                        request.cursor_byte_offset,
                    );
                }
                '&' if next_char == Some('&') => {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(UnsupportedSyntax::LogicalAnd),
                        request.cursor_byte_offset,
                    );
                }
                '&' => {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(UnsupportedSyntax::Background),
                        request.cursor_byte_offset,
                    );
                }
                ';' => {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(UnsupportedSyntax::Semicolon),
                        request.cursor_byte_offset,
                    );
                }
                '#' if current.is_none() => {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(UnsupportedSyntax::Comment),
                        request.cursor_byte_offset,
                    );
                }
                '<' | '>' => {
                    return ParseOutput::degraded(
                        ParseDegradedReason::UnsupportedSyntax(UnsupportedSyntax::Redirection),
                        request.cursor_byte_offset,
                    );
                }
                '\\' => {
                    let Some(escaped_char) = next_char else {
                        return ParseOutput::degraded(
                            ParseDegradedReason::TrailingEscape,
                            request.cursor_byte_offset,
                        );
                    };
                    let escaped_len = escaped_char.len_utf8();
                    let builder = current
                        .get_or_insert_with(|| TokenBuilder::new(index, QuoteContext::Unquoted));
                    builder.text.push(escaped_char);
                    index += ch_len + escaped_len;
                    continue;
                }
                '\'' => {
                    current.get_or_insert_with(|| {
                        TokenBuilder::new(index, QuoteContext::SingleQuoted)
                    });
                    quote_context = QuoteContext::SingleQuoted;
                    index += ch_len;
                    continue;
                }
                '"' => {
                    current.get_or_insert_with(|| {
                        TokenBuilder::new(index, QuoteContext::DoubleQuoted)
                    });
                    quote_context = QuoteContext::DoubleQuoted;
                    index += ch_len;
                    continue;
                }
                whitespace if whitespace.is_whitespace() => {
                    if let Some(builder) = current.take() {
                        tokens.push(builder.into_token(index));
                    }
                    index += ch_len;
                    continue;
                }
                _ => {
                    let builder = current
                        .get_or_insert_with(|| TokenBuilder::new(index, QuoteContext::Unquoted));
                    builder.text.push(ch);
                    index += ch_len;
                    continue;
                }
            }
        }

        match quote_context {
            QuoteContext::SingleQuoted => {
                if ch == '\'' {
                    quote_context = QuoteContext::Unquoted;
                } else if let Some(builder) = current.as_mut() {
                    builder.text.push(ch);
                }
                index += ch_len;
            }
            QuoteContext::DoubleQuoted => {
                if ch == '"' {
                    quote_context = QuoteContext::Unquoted;
                    index += ch_len;
                    continue;
                }

                if ch == '\\' {
                    let Some(escaped_char) = next_char else {
                        return ParseOutput::degraded(
                            ParseDegradedReason::TrailingEscape,
                            request.cursor_byte_offset,
                        );
                    };
                    let escaped_len = escaped_char.len_utf8();
                    if let Some(builder) = current.as_mut() {
                        builder.text.push(escaped_char);
                    }
                    index += ch_len + escaped_len;
                    continue;
                }

                if let Some(builder) = current.as_mut() {
                    builder.text.push(ch);
                }
                index += ch_len;
            }
            QuoteContext::Unquoted => unreachable!("handled above"),
        }
    }

    if quote_context != QuoteContext::Unquoted {
        return ParseOutput::degraded(
            ParseDegradedReason::UnterminatedQuote(quote_context),
            request.cursor_byte_offset,
        );
    }

    if let Some(builder) = current.take() {
        tokens.push(builder.into_token(parse_buffer.len()));
    }

    extend_active_token_to_full_word(&request.buffer, request.cursor_byte_offset, &mut tokens);
    let active_token = determine_active_token(request.cursor_byte_offset, &tokens);
    let provider_root = tokens.first().and_then(|token| {
        root_index
            .exact_match(&token.text)
            .map(|entry| entry.provider_id.clone())
    });
    let completion_position =
        determine_completion_position(request, &tokens, &active_token, provider_root.is_some());

    ParseOutput {
        status: ParserStatus::Complete,
        tokens,
        active_token,
        completion_position,
        provider_root,
        active_slot_id: None,
    }
}

fn extend_active_token_to_full_word(
    buffer: &str,
    cursor_byte_offset: usize,
    tokens: &mut [ParsedToken],
) {
    let Some(token) = tokens
        .iter_mut()
        .find(|token| token.raw_range.end_byte == cursor_byte_offset)
    else {
        return;
    };

    if token.quote_context != QuoteContext::Unquoted {
        return;
    }

    token.raw_range.end_byte = unquoted_token_end(buffer, cursor_byte_offset);
}

fn unquoted_token_end(buffer: &str, start_byte: usize) -> usize {
    let mut end = start_byte;
    for (relative_index, ch) in buffer[start_byte..].char_indices() {
        if ch.is_whitespace() || matches!(ch, '|' | '&' | ';' | '<' | '>') {
            return start_byte + relative_index;
        }
        end = start_byte + relative_index + ch.len_utf8();
    }
    end
}

fn determine_active_token(cursor_byte_offset: usize, tokens: &[ParsedToken]) -> ActiveToken {
    if let Some((index, token)) = tokens.iter().enumerate().find(|(_, token)| {
        cursor_byte_offset >= token.raw_range.start_byte
            && cursor_byte_offset <= token.raw_range.end_byte
    }) {
        return ActiveToken {
            index: Some(index),
            value: token.text.clone(),
            raw_range: token.raw_range,
            quote_context: token.quote_context,
        };
    }

    ActiveToken {
        index: None,
        value: String::new(),
        raw_range: ReplaceRange {
            start_byte: cursor_byte_offset,
            end_byte: cursor_byte_offset,
        },
        quote_context: QuoteContext::Unquoted,
    }
}

fn determine_completion_position(
    request: &SuggestRequest,
    tokens: &[ParsedToken],
    active_token: &ActiveToken,
    provider_selected: bool,
) -> CompletionPosition {
    if !provider_selected {
        return CompletionPosition::RootCommand;
    }

    if let Some(index) = active_token.index {
        if index == 0 {
            return CompletionPosition::RootCommand;
        }

        if tokens[index].text.starts_with('-') {
            return CompletionPosition::Flag;
        }

        if index == 1 {
            return CompletionPosition::Subcommand;
        }

        return CompletionPosition::Value;
    }

    if let Some(previous_token) = tokens
        .iter()
        .rev()
        .find(|token| token.raw_range.end_byte <= request.cursor_byte_offset)
    {
        if previous_token.text.starts_with('-') {
            return CompletionPosition::Value;
        }

        if previous_token.raw_range.start_byte == 0 {
            return CompletionPosition::Subcommand;
        }
    }

    CompletionPosition::Value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ShellKind;
    use crate::providers::{ProviderRootIndex, ProviderRootIndexEntry, ProviderSourceKind};

    fn roots(ids: &[(&str, &str)]) -> ProviderRootIndex {
        ProviderRootIndex::new(
            ids.iter()
                .map(|(root, provider_id)| ProviderRootIndexEntry {
                    root_command: (*root).to_string(),
                    root_aliases: Vec::new(),
                    provider_id: ProviderId::from(*provider_id),
                    source_kind: ProviderSourceKind::EmbeddedBuiltin,
                    schema_version: 1,
                })
                .collect(),
        )
        .expect("root index should build")
    }

    #[test]
    fn parser_supports_plain_tokens_and_quotes() {
        let request = SuggestRequest::minimal(ShellKind::Zsh, "git checkout 'feature/a'", 24);
        let parse = parse_request(&request, &roots(&[("git", "builtin.git")]));
        assert_eq!(parse.status, ParserStatus::Complete);
        assert_eq!(parse.tokens.len(), 3);
        assert_eq!(parse.tokens[2].text, "feature/a");
        assert_eq!(parse.tokens[2].quote_context, QuoteContext::SingleQuoted);
        assert_eq!(parse.provider_root, Some(ProviderId::from("builtin.git")));
    }

    #[test]
    fn parser_rejects_command_substitution() {
        let request = SuggestRequest::minimal(ShellKind::Zsh, "git $(status)", 13);
        let parse = parse_request(&request, &roots(&[("git", "builtin.git")]));
        assert_eq!(
            parse.degraded_reason(),
            Some(ParseDegradedReason::UnsupportedSyntax(
                UnsupportedSyntax::CommandSubstitution
            ))
        );
    }

    #[test]
    fn parser_rejects_multiline_buffers() {
        let request = SuggestRequest::minimal(ShellKind::Zsh, "git status\npwd", 7);
        let parse = parse_request(&request, &roots(&[("git", "builtin.git")]));
        assert_eq!(
            parse.degraded_reason(),
            Some(ParseDegradedReason::MultilineBuffer)
        );
    }

    #[test]
    fn parser_selects_provider_root_exactly() {
        let request = SuggestRequest::minimal(ShellKind::Zsh, "gi", 2);
        let parse = parse_request(&request, &roots(&[("git", "builtin.git")]));
        assert_eq!(parse.provider_root, None);
        assert_eq!(parse.completion_position, CompletionPosition::RootCommand);
    }

    #[test]
    fn parser_rejects_parameter_expansion_and_comments() {
        let root_index = roots(&[("git", "builtin.git")]);

        let expansion = SuggestRequest::minimal(ShellKind::Zsh, "git $FOO", 8);
        let comment = SuggestRequest::minimal(ShellKind::Zsh, "git st # note", 13);

        assert_eq!(
            parse_request(&expansion, &root_index).degraded_reason(),
            Some(ParseDegradedReason::UnsupportedSyntax(
                UnsupportedSyntax::ParameterExpansion
            ))
        );
        assert_eq!(
            parse_request(&comment, &root_index).degraded_reason(),
            Some(ParseDegradedReason::UnsupportedSyntax(
                UnsupportedSyntax::Comment
            ))
        );
    }

    #[test]
    fn parser_distinguishes_git_and_kubectl_roots_exactly() {
        let root_index = roots(&[("git", "builtin.git"), ("kubectl", "builtin.kubectl")]);

        let kub = SuggestRequest::minimal(ShellKind::Zsh, "kub", 3);
        let kubectl = SuggestRequest::minimal(ShellKind::Zsh, "kubectl get", 11);

        assert_eq!(parse_request(&kub, &root_index).provider_root, None);
        assert_eq!(
            parse_request(&kubectl, &root_index).provider_root,
            Some(ProviderId::from("builtin.kubectl"))
        );
    }

    #[test]
    fn parser_only_degrades_for_unsupported_syntax_before_cursor() {
        let request = SuggestRequest::minimal(ShellKind::Zsh, "git st $(later)", 6);
        let parse = parse_request(&request, &roots(&[("git", "builtin.git")]));

        assert_eq!(parse.status, ParserStatus::Complete);
        assert_eq!(parse.tokens.len(), 2);
        assert_eq!(parse.active_token.value, "st");
    }

    #[test]
    fn parser_replacement_range_covers_active_unquoted_suffix() {
        let request = SuggestRequest::minimal(ShellKind::Zsh, "git checkout", 7);
        let parse = parse_request(&request, &roots(&[("git", "builtin.git")]));

        assert_eq!(parse.active_token.value, "che");
        assert_eq!(
            parse.active_token.raw_range,
            ReplaceRange {
                start_byte: 4,
                end_byte: 12,
            }
        );
    }
}
