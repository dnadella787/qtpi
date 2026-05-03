mod builtins;
mod git;
mod kubectl;

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use twocp_core::engine::SuggestEngine;
use twocp_core::protocol::{
    ReplaceRange, RequestMode, ShellKind, SuggestRequest, SuggestResponse, TerminalCapabilities,
};

#[derive(Parser)]
#[command(
    name = "twocp",
    about = "Phase 4 multi-CLI shell bridge and dynamic lookup surface for 2cp"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    BuildProvider {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    ListBuiltins,
    Suggest {
        #[arg(long, default_value = "zsh")]
        shell: String,
        #[arg(long)]
        buffer: String,
        #[arg(long)]
        cursor: usize,
        #[arg(long, value_enum, default_value_t = CursorUnits::Bytes)]
        cursor_units: CursorUnits,
        #[arg(long)]
        cwd: PathBuf,
        #[arg(long, default_value_t = 0)]
        columns: u16,
        #[arg(long, default_value_t = 0)]
        rows: u16,
        #[arg(long, default_value = "json")]
        format: String,
        #[arg(long, default_value_t = 8)]
        max_suggestions: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CursorUnits {
    Bytes,
    Chars,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::ListBuiltins) {
        Command::BuildProvider { input, output } => {
            twocp_build::compile_json_file_to_file(&input, &output)?;
            println!("compiled provider artifact to {}", output.display());
        }
        Command::ListBuiltins => {
            let catalog = builtins::builtin_catalog()?;
            for entry in catalog.list_roots() {
                let (command_name, subcommand_count, flag_count) =
                    builtins::builtin_summary(&entry.provider_id)?;
                println!(
                    "{}: root={}, provider_id={}, subcommands={}, flags={}",
                    entry.root_command,
                    command_name,
                    entry.provider_id,
                    subcommand_count,
                    flag_count
                );
            }
        }
        Command::Suggest {
            shell,
            buffer,
            cursor,
            cursor_units,
            cwd,
            columns,
            rows,
            format,
            max_suggestions,
        } => {
            let shell = parse_shell_kind(&shell)?;
            let cursor_byte_offset = match cursor_units {
                CursorUnits::Bytes => cursor,
                CursorUnits::Chars => char_offset_to_byte_offset(&buffer, cursor)?,
            };
            let catalog = builtins::builtin_catalog()?;
            let engine = SuggestEngine::new(&catalog).with_max_suggestions(max_suggestions);
            let request = SuggestRequest {
                shell,
                buffer: buffer.clone(),
                cursor_byte_offset,
                cwd,
                env_hints: BTreeMap::new(),
                terminal_capabilities: TerminalCapabilities {
                    color: true,
                    cursor_movement: true,
                    terminal_width: non_zero(columns),
                    terminal_height: non_zero(rows),
                },
                mode: RequestMode::Suggest,
            };
            let response = engine.suggest(&request);

            match format.as_str() {
                "json" => {
                    println!("{}", serde_json::to_string(&response)?);
                }
                "zsh" => {
                    print!("{}", render_zsh_response(&response, &buffer, cursor)?);
                }
                other => {
                    bail!("unsupported suggest output format: {other}");
                }
            }
        }
    }

    Ok(())
}

fn parse_shell_kind(shell: &str) -> Result<ShellKind> {
    match shell {
        "zsh" => Ok(ShellKind::Zsh),
        _ => bail!("unsupported shell: {shell}"),
    }
}

fn non_zero(value: u16) -> Option<u16> {
    if value == 0 { None } else { Some(value) }
}

fn char_offset_to_byte_offset(buffer: &str, char_offset: usize) -> Result<usize> {
    if char_offset > buffer.chars().count() {
        bail!(
            "cursor char offset {} is outside buffer char length {}",
            char_offset,
            buffer.chars().count()
        );
    }

    Ok(buffer
        .char_indices()
        .nth(char_offset)
        .map(|(byte_offset, _)| byte_offset)
        .unwrap_or(buffer.len()))
}

fn byte_range_to_char_range(buffer: &str, range: ReplaceRange) -> Result<(usize, usize)> {
    let start = byte_offset_to_char_offset(buffer, range.start_byte)?;
    let end = byte_offset_to_char_offset(buffer, range.end_byte)?;
    Ok((start, end))
}

fn byte_offset_to_char_offset(buffer: &str, byte_offset: usize) -> Result<usize> {
    if byte_offset > buffer.len() {
        bail!(
            "byte offset {} is outside buffer byte length {}",
            byte_offset,
            buffer.len()
        );
    }

    if !buffer.is_char_boundary(byte_offset) {
        return Err(anyhow!(
            "byte offset {byte_offset} is not on a char boundary"
        ));
    }

    Ok(buffer[..byte_offset].chars().count())
}

fn render_zsh_response(
    response: &SuggestResponse,
    buffer: &str,
    cursor_char_offset: usize,
) -> Result<String> {
    let mut output = String::new();
    let (replace_start, replace_end) = match response.replace_range {
        Some(range) => byte_range_to_char_range(buffer, range)?,
        None => (0, 0),
    };
    let selection_index = response.selection_index.unwrap_or(0);
    let provider_id = response
        .diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.provider_id.as_ref())
        .map(|provider_id| provider_id.as_str())
        .unwrap_or("");
    let parser_status = response
        .diagnostics
        .as_ref()
        .map(|diagnostics| format!("{:?}", diagnostics.parser_status).to_ascii_lowercase())
        .unwrap_or_else(|| "unknown".to_string());
    let dynamic_slot_id = response
        .diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.dynamic_lookup.as_ref())
        .map(|lookup| lookup.slot_id.as_str())
        .unwrap_or("");
    let lookup_status = response
        .diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.dynamic_lookup.as_ref())
        .map(|lookup| format!("{:?}", lookup.status).to_ascii_lowercase())
        .unwrap_or_else(|| "not_checked".to_string());
    let cache_status = response
        .diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.dynamic_lookup.as_ref())
        .map(|lookup| format!("{:?}", lookup.cache_status).to_ascii_lowercase())
        .unwrap_or_else(|| "not_checked".to_string());
    let lookup_count = response
        .diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.dynamic_lookup.as_ref())
        .map(|lookup| lookup.match_count)
        .unwrap_or_default();
    let lookup_time_ms = response
        .diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.dynamic_lookup.as_ref())
        .map(|lookup| lookup.lookup_time_ms)
        .unwrap_or_default();

    output.push_str(&format!(
        "typeset -g __twocp_status={}\n",
        shell_quote(match response.status {
            twocp_core::protocol::ResponseStatus::Ok => "ok",
            twocp_core::protocol::ResponseStatus::NoMatch => "no_match",
            twocp_core::protocol::ResponseStatus::Degraded => "degraded",
            twocp_core::protocol::ResponseStatus::Error => "error",
        })
    ));
    output.push_str(&format!(
        "typeset -gi __twocp_replace_start={replace_start}\n\
typeset -gi __twocp_replace_end={replace_end}\n\
typeset -gi __twocp_selection_index={selection_index}\n\
typeset -gi __twocp_truncated_count={}\n\
typeset -gi __twocp_request_cursor={cursor_char_offset}\n\
typeset -gi __twocp_lookup_count={lookup_count}\n\
typeset -gi __twocp_lookup_time_ms={lookup_time_ms}\n\
typeset -g __twocp_provider_id={}\n\
typeset -g __twocp_parser_status={}\n\
typeset -g __twocp_request_buffer={}\n\
typeset -g __twocp_dynamic_slot_id={}\n\
typeset -g __twocp_lookup_status={}\n\
typeset -g __twocp_cache_status={}\n",
        response.render_model.truncated_count,
        shell_quote(provider_id),
        shell_quote(&parser_status),
        shell_quote(buffer),
        shell_quote(dynamic_slot_id),
        shell_quote(&lookup_status),
        shell_quote(&cache_status),
    ));

    output.push_str("typeset -ga __twocp_insert_texts=(");
    for suggestion in &response.suggestions {
        output.push(' ');
        output.push_str(&shell_quote(&suggestion.insert_text));
    }
    output.push_str(" )\n");

    output.push_str("typeset -ga __twocp_displays=(");
    for suggestion in &response.suggestions {
        output.push(' ');
        output.push_str(&shell_quote(&suggestion.display));
    }
    output.push_str(" )\n");

    output.push_str("typeset -ga __twocp_annotations=(");
    for suggestion in &response.suggestions {
        output.push(' ');
        output.push_str(&shell_quote(suggestion.annotation.as_deref().unwrap_or("")));
    }
    output.push_str(" )\n");

    output.push_str("typeset -ga __twocp_kinds=(");
    for suggestion in &response.suggestions {
        output.push(' ');
        output.push_str(&shell_quote(match suggestion.kind {
            twocp_core::protocol::SuggestionKind::Command => "command",
            twocp_core::protocol::SuggestionKind::Flag => "flag",
            twocp_core::protocol::SuggestionKind::Value => "value",
            twocp_core::protocol::SuggestionKind::Help => "help",
        }));
    }
    output.push_str(" )\n");

    Ok(output)
}

fn shell_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{escaped}'")
}
