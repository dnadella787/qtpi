mod admin;
mod builtins;
mod git;
mod kubectl;

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use twocp_core::engine::SuggestEngine;
use twocp_core::protocol::{
    ReplaceRange, RequestMode, ResponseStatus, ShellKind, SuggestRequest, SuggestResponse,
    SuggestionKind, TerminalCapabilities,
};
use twocp_core::providers::{ProviderCatalog, ProviderSourceKind};
use twocp_core::spec::{ProviderCapabilities, ProviderId};

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
    PrintShellHook {
        #[arg(long, default_value = "zsh")]
        shell: String,
        #[arg(long)]
        bin_path: Option<PathBuf>,
        #[arg(long)]
        hook_path: Option<PathBuf>,
        #[arg(long)]
        rc_file: Option<PathBuf>,
    },
    Install {
        #[arg(long, default_value = "zsh")]
        shell: String,
        #[arg(long)]
        bin_path: Option<PathBuf>,
        #[arg(long)]
        hook_path: Option<PathBuf>,
        #[arg(long)]
        rc_file: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    Uninstall {
        #[arg(long, default_value = "zsh")]
        shell: String,
        #[arg(long)]
        bin_path: Option<PathBuf>,
        #[arg(long)]
        hook_path: Option<PathBuf>,
        #[arg(long)]
        rc_file: Option<PathBuf>,
    },
    Doctor {
        #[arg(long, default_value = "zsh")]
        shell: String,
        #[arg(long)]
        bin_path: Option<PathBuf>,
        #[arg(long)]
        hook_path: Option<PathBuf>,
        #[arg(long)]
        rc_file: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = DoctorFormat::Text)]
        format: DoctorFormat,
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
    Debug {
        #[command(subcommand)]
        command: DebugCommand,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CursorUnits {
    Bytes,
    Chars,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum DoctorFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum DebugCommand {
    Request(DebugRequestArgs),
    Render(DebugRequestArgs),
    Provider {
        provider_id: String,
        #[arg(long, value_enum, default_value_t = DebugFormat::Text)]
        format: DebugFormat,
    },
}

#[derive(Parser)]
struct DebugRequestArgs {
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
    #[arg(long, value_enum, default_value_t = DebugFormat::Text)]
    format: DebugFormat,
    #[arg(long, default_value_t = 8)]
    max_suggestions: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum DebugFormat {
    Text,
    Json,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::ListBuiltins) {
        Command::BuildProvider { input, output } => {
            twocp_build::compile_json_file_to_file(&input, &output)?;
            println!("compiled provider artifact to {}", output.display());
        }
        Command::PrintShellHook {
            shell,
            bin_path,
            hook_path,
            rc_file,
        } => {
            ensure_zsh_shell(&shell)?;
            let paths = admin::resolve_install_paths(&admin::PathOverrides {
                bin_path,
                hook_path,
                rc_file,
            })?;
            print!("{}", admin::print_shell_hook(&paths));
        }
        Command::Install {
            shell,
            bin_path,
            hook_path,
            rc_file,
            force,
        } => {
            ensure_zsh_shell(&shell)?;
            let paths = admin::resolve_install_paths(&admin::PathOverrides {
                bin_path,
                hook_path,
                rc_file,
            })?;
            admin::install(&paths, force)?;
            println!(
                "installed 2cp zsh hook at {} and updated {}",
                paths.hook_path.display(),
                paths.rc_file.display()
            );
        }
        Command::Uninstall {
            shell,
            bin_path,
            hook_path,
            rc_file,
        } => {
            ensure_zsh_shell(&shell)?;
            let paths = admin::resolve_install_paths(&admin::PathOverrides {
                bin_path,
                hook_path,
                rc_file,
            })?;
            let _ = bin_path;
            admin::uninstall(&paths)?;
            println!(
                "removed 2cp zsh hook at {} and cleaned {}",
                paths.hook_path.display(),
                paths.rc_file.display()
            );
        }
        Command::Doctor {
            shell,
            bin_path,
            hook_path,
            rc_file,
            format,
        } => {
            ensure_zsh_shell(&shell)?;
            let paths = admin::resolve_install_paths(&admin::PathOverrides {
                bin_path,
                hook_path,
                rc_file,
            })?;
            let report = admin::run_doctor(&paths);
            match format {
                DoctorFormat::Text => print!("{}", admin::render_doctor_text(&report)),
                DoctorFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
            }
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
            let debug_args = DebugRequestArgs {
                shell,
                buffer,
                cursor,
                cursor_units,
                cwd,
                columns,
                rows,
                format: DebugFormat::Json,
                max_suggestions,
            };
            let execution = execute_request(debug_args, RequestMode::Suggest)?;

            match format.as_str() {
                "json" => {
                    println!("{}", serde_json::to_string(&execution.response)?);
                }
                "zsh" => {
                    print!(
                        "{}",
                        render_zsh_response(
                            &execution.response,
                            &execution.request.buffer,
                            execution.request_cursor_char_offset,
                        )?
                    );
                }
                other => {
                    bail!("unsupported suggest output format: {other}");
                }
            }
        }
        Command::Debug { command } => match command {
            DebugCommand::Request(args) => {
                let execution = execute_request(args, RequestMode::Debug)?;
                let report = build_debug_request_report(&execution);
                match execution.format {
                    DebugFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
                    DebugFormat::Text => print!("{}", render_debug_request_text(&report)),
                }
            }
            DebugCommand::Render(args) => {
                let execution = execute_request(args, RequestMode::Debug)?;
                let report = build_debug_render_report(&execution.response);
                match execution.format {
                    DebugFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
                    DebugFormat::Text => print!("{}", render_debug_render_text(&report)),
                }
            }
            DebugCommand::Provider {
                provider_id,
                format,
            } => {
                let report = inspect_provider(&provider_id)?;
                match format {
                    DebugFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
                    DebugFormat::Text => print!("{}", render_debug_provider_text(&report)),
                }
            }
        },
    }

    Ok(())
}

struct RequestExecution {
    request: SuggestRequest,
    response: SuggestResponse,
    request_cursor_char_offset: usize,
    format: DebugFormat,
    max_suggestions: usize,
}

fn execute_request(args: DebugRequestArgs, mode: RequestMode) -> Result<RequestExecution> {
    let shell = parse_shell_kind(&args.shell)?;
    let cursor_byte_offset = match args.cursor_units {
        CursorUnits::Bytes => args.cursor,
        CursorUnits::Chars => char_offset_to_byte_offset(&args.buffer, args.cursor)?,
    };
    let request_cursor_char_offset = byte_offset_to_char_offset(&args.buffer, cursor_byte_offset)?;
    let request = SuggestRequest {
        shell,
        buffer: args.buffer,
        cursor_byte_offset,
        cwd: args.cwd,
        env_hints: BTreeMap::new(),
        terminal_capabilities: TerminalCapabilities {
            color: true,
            cursor_movement: true,
            terminal_width: non_zero(args.columns),
            terminal_height: non_zero(args.rows),
        },
        mode,
    };
    let catalog = builtins::builtin_catalog()?;
    let engine = SuggestEngine::new(&catalog).with_max_suggestions(args.max_suggestions);
    let response = engine.suggest(&request);

    Ok(RequestExecution {
        request,
        response,
        request_cursor_char_offset,
        format: args.format,
        max_suggestions: args.max_suggestions,
    })
}

fn inspect_provider(provider_selector: &str) -> Result<DebugProviderReport> {
    let catalog = builtins::builtin_catalog()?;
    let entry = catalog
        .list_roots()
        .iter()
        .find(|entry| {
            entry.provider_id.as_str() == provider_selector
                || entry.root_command == provider_selector
                || entry
                    .root_aliases
                    .iter()
                    .any(|alias| alias == provider_selector)
        })
        .cloned()
        .ok_or_else(|| anyhow!("unknown provider: {provider_selector}"))?;
    let provider_id = entry.provider_id.clone();
    let provider = catalog.load_provider(&provider_id)?;
    let metadata = provider.metadata();
    let summary = provider.root_summary();

    Ok(DebugProviderReport {
        provider_id,
        exists: true,
        load_status: "loaded".into(),
        source_kind: entry.source_kind,
        root_command: entry.root_command,
        root_aliases: entry.root_aliases,
        capabilities: metadata.capabilities,
        root_summary: DebugProviderRootSummary {
            command_name: summary.command_name,
            description: metadata.description.clone(),
            subcommand_count: summary.subcommand_count,
            flag_count: summary.flag_count,
        },
    })
}

fn build_debug_request_report(execution: &RequestExecution) -> DebugRequestReport {
    let diagnostics = execution.response.diagnostics.as_ref();
    DebugRequestReport {
        request: DebugRequestSummary {
            shell: execution.request.shell,
            mode: execution.request.mode,
            buffer: execution.request.buffer.clone(),
            cursor_byte_offset: execution.request.cursor_byte_offset,
            cursor_char_offset: execution.request_cursor_char_offset,
            cwd: execution.request.cwd.clone(),
            terminal_capabilities: execution.request.terminal_capabilities.clone(),
            max_suggestions: execution.max_suggestions,
        },
        response: DebugResponseSummary {
            status: execution.response.status,
            selected_provider_id: diagnostics.and_then(|item| item.provider_id.clone()),
            parser_status: diagnostics.map(|item| item.parser_status),
            timings: diagnostics.map(|item| item.timings).unwrap_or_default(),
            dynamic_lookup: diagnostics.and_then(|item| item.dynamic_lookup.clone()),
            replacement_range: execution.response.replace_range,
            suggestions: execution
                .response
                .suggestions
                .iter()
                .map(|suggestion| DebugSuggestion {
                    display: suggestion.display.clone(),
                    insert_text: suggestion.insert_text.clone(),
                    kind: suggestion.kind,
                })
                .collect(),
        },
    }
}

fn build_debug_render_report(response: &SuggestResponse) -> DebugRenderReport {
    DebugRenderReport {
        status: response.status,
        row_count: response.render_model.rows.len(),
        truncated_count: response.render_model.truncated_count,
        degraded: response.render_model.degraded,
        rows: response
            .render_model
            .rows
            .iter()
            .map(|row| DebugRenderRow {
                primary: row.primary.clone(),
                secondary: row.secondary.clone(),
                kind: row.kind,
            })
            .collect(),
    }
}

#[derive(Debug, Serialize)]
struct DebugRequestReport {
    request: DebugRequestSummary,
    response: DebugResponseSummary,
}

#[derive(Debug, Serialize)]
struct DebugRequestSummary {
    shell: ShellKind,
    mode: RequestMode,
    buffer: String,
    cursor_byte_offset: usize,
    cursor_char_offset: usize,
    cwd: PathBuf,
    terminal_capabilities: TerminalCapabilities,
    max_suggestions: usize,
}

#[derive(Debug, Serialize)]
struct DebugResponseSummary {
    status: ResponseStatus,
    selected_provider_id: Option<ProviderId>,
    parser_status: Option<twocp_core::parser::ParserStatus>,
    timings: twocp_core::protocol::TimingDiagnostics,
    dynamic_lookup: Option<twocp_core::protocol::DynamicLookupDiagnostics>,
    replacement_range: Option<ReplaceRange>,
    suggestions: Vec<DebugSuggestion>,
}

#[derive(Debug, Serialize)]
struct DebugSuggestion {
    display: String,
    insert_text: String,
    kind: SuggestionKind,
}

#[derive(Debug, Serialize)]
struct DebugRenderReport {
    status: ResponseStatus,
    row_count: usize,
    truncated_count: usize,
    degraded: bool,
    rows: Vec<DebugRenderRow>,
}

#[derive(Debug, Serialize)]
struct DebugRenderRow {
    primary: String,
    secondary: Option<String>,
    kind: SuggestionKind,
}

#[derive(Debug, Serialize)]
struct DebugProviderReport {
    provider_id: ProviderId,
    exists: bool,
    load_status: String,
    source_kind: ProviderSourceKind,
    root_command: String,
    root_aliases: Vec<String>,
    capabilities: ProviderCapabilities,
    root_summary: DebugProviderRootSummary,
}

#[derive(Debug, Serialize)]
struct DebugProviderRootSummary {
    command_name: String,
    description: Option<String>,
    subcommand_count: usize,
    flag_count: usize,
}

fn render_debug_request_text(report: &DebugRequestReport) -> String {
    let mut output = String::new();
    output.push_str("request\n");
    output.push_str(&format!("  shell: {:?}\n", report.request.shell));
    output.push_str(&format!("  mode: {:?}\n", report.request.mode));
    output.push_str(&format!("  buffer: {}\n", report.request.buffer));
    output.push_str(&format!(
        "  cursor: bytes={} chars={}\n",
        report.request.cursor_byte_offset, report.request.cursor_char_offset
    ));
    output.push_str(&format!("  cwd: {}\n", report.request.cwd.display()));
    output.push_str(&format!(
        "  terminal: color={} cursor_movement={} columns={:?} rows={:?}\n",
        report.request.terminal_capabilities.color,
        report.request.terminal_capabilities.cursor_movement,
        report.request.terminal_capabilities.terminal_width,
        report.request.terminal_capabilities.terminal_height
    ));
    output.push_str(&format!(
        "  max_suggestions: {}\n",
        report.request.max_suggestions
    ));
    output.push_str("response\n");
    output.push_str(&format!("  status: {:?}\n", report.response.status));
    output.push_str(&format!(
        "  provider: {}\n",
        report
            .response
            .selected_provider_id
            .as_ref()
            .map(ProviderId::as_str)
            .unwrap_or("none")
    ));
    output.push_str(&format!(
        "  parser_status: {}\n",
        report
            .response
            .parser_status
            .map(|status| format!("{status:?}"))
            .unwrap_or_else(|| "unknown".into())
    ));
    output.push_str(&format!(
        "  timings_ms: parse={} provider={} dynamic_lookup={} total={}\n",
        report.response.timings.parse_ms,
        report.response.timings.provider_ms,
        report.response.timings.dynamic_lookup_ms,
        report.response.timings.total_ms
    ));
    if let Some(dynamic_lookup) = &report.response.dynamic_lookup {
        output.push_str(&format!(
            "  dynamic_lookup: slot={} status={:?} cache={:?} matches={} time_ms={}\n",
            dynamic_lookup.slot_id.as_str(),
            dynamic_lookup.status,
            dynamic_lookup.cache_status,
            dynamic_lookup.match_count,
            dynamic_lookup.lookup_time_ms
        ));
    }
    output.push_str(&format!(
        "  replacement_range: {}\n",
        report
            .response
            .replacement_range
            .map(|range| format!("{}..{}", range.start_byte, range.end_byte))
            .unwrap_or_else(|| "none".into())
    ));
    output.push_str("suggestions\n");
    for suggestion in &report.response.suggestions {
        output.push_str(&format!(
            "  - display={} insert={} kind={:?}\n",
            suggestion.display, suggestion.insert_text, suggestion.kind
        ));
    }
    output
}

fn render_debug_render_text(report: &DebugRenderReport) -> String {
    let mut output = String::new();
    output.push_str(&format!("status: {:?}\n", report.status));
    output.push_str(&format!("row_count: {}\n", report.row_count));
    output.push_str(&format!("truncated_count: {}\n", report.truncated_count));
    output.push_str(&format!("degraded: {}\n", report.degraded));
    output.push_str("rows\n");
    for row in &report.rows {
        output.push_str(&format!(
            "  - primary={} secondary={} kind={:?}\n",
            row.primary,
            row.secondary.as_deref().unwrap_or(""),
            row.kind
        ));
    }
    output
}

fn render_debug_provider_text(report: &DebugProviderReport) -> String {
    let mut output = String::new();
    output.push_str(&format!("provider_id: {}\n", report.provider_id.as_str()));
    output.push_str(&format!("exists: {}\n", report.exists));
    output.push_str(&format!("load_status: {}\n", report.load_status));
    output.push_str(&format!("source_kind: {:?}\n", report.source_kind));
    output.push_str(&format!("root_command: {}\n", report.root_command));
    output.push_str(&format!(
        "root_aliases: {}\n",
        report.root_aliases.join(", ")
    ));
    output.push_str(&format!(
        "capabilities: static_commands={} dynamic_values={} requires_subprocess={}\n",
        report.capabilities.supports_static_commands,
        report.capabilities.supports_dynamic_values,
        report.capabilities.requires_subprocess
    ));
    output.push_str(&format!(
        "root_summary: command={} subcommands={} flags={}\n",
        report.root_summary.command_name,
        report.root_summary.subcommand_count,
        report.root_summary.flag_count
    ));
    if let Some(description) = &report.root_summary.description {
        output.push_str(&format!("description: {}\n", description));
    }
    output
}

fn parse_shell_kind(shell: &str) -> Result<ShellKind> {
    match shell {
        "zsh" => Ok(ShellKind::Zsh),
        _ => bail!("unsupported shell: {shell}"),
    }
}

fn ensure_zsh_shell(shell: &str) -> Result<()> {
    match shell {
        "zsh" => Ok(()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use twocp_core::parser::{ParseDegradedReason, UnsupportedSyntax};

    fn debug_args(buffer: &str, cursor: usize) -> DebugRequestArgs {
        DebugRequestArgs {
            shell: "zsh".into(),
            buffer: buffer.into(),
            cursor,
            cursor_units: CursorUnits::Bytes,
            cwd: PathBuf::from("."),
            columns: 0,
            rows: 0,
            format: DebugFormat::Json,
            max_suggestions: 8,
        }
    }

    #[test]
    fn debug_request_json_reports_provider_parser_timings_and_suggestions() {
        let execution = execute_request(debug_args("git ", 4), RequestMode::Debug)
            .expect("debug request should execute");
        let report = build_debug_request_report(&execution);

        assert_eq!(
            report.response.selected_provider_id,
            Some(ProviderId::from("builtin.git"))
        );
        assert_eq!(
            report.response.parser_status,
            Some(twocp_core::parser::ParserStatus::Complete)
        );
        assert_eq!(report.response.status, ResponseStatus::Ok);
        assert!(!report.response.suggestions.is_empty());
        assert!(report.response.timings.total_ms >= report.response.timings.parse_ms);
    }

    #[test]
    fn debug_request_reports_degraded_unsupported_syntax() {
        let execution = execute_request(debug_args("git $(status)", 13), RequestMode::Debug)
            .expect("debug request should execute");
        let report = build_debug_request_report(&execution);

        assert_eq!(report.response.status, ResponseStatus::Degraded);
        assert!(report.response.suggestions.is_empty());
        assert_eq!(
            report.response.parser_status,
            Some(twocp_core::parser::ParserStatus::Degraded(
                ParseDegradedReason::UnsupportedSyntax(UnsupportedSyntax::CommandSubstitution),
            ))
        );
    }

    #[test]
    fn debug_render_reports_truncation_and_degraded_state() {
        let mut args = debug_args("git ", 4);
        args.max_suggestions = 2;
        let execution =
            execute_request(args, RequestMode::Debug).expect("debug render request should execute");
        let report = build_debug_render_report(&execution.response);

        assert_eq!(report.status, ResponseStatus::Ok);
        assert_eq!(report.row_count, 2);
        assert!(report.truncated_count > 0);
        assert!(!report.degraded);
        assert_eq!(
            report.rows[0].primary,
            execution.response.render_model.rows[0].primary
        );
    }

    #[test]
    fn debug_provider_reports_expected_builtin_metadata() {
        let git = inspect_provider("git").expect("git provider should load");
        assert_eq!(git.provider_id, ProviderId::from("builtin.git"));
        assert_eq!(git.root_command, "git");
        assert!(git.root_summary.subcommand_count > 0);

        let kubectl = inspect_provider("kubectl").expect("kubectl provider should load");
        assert_eq!(kubectl.provider_id, ProviderId::from("builtin.kubectl"));
        assert_eq!(kubectl.root_command, "kubectl");
        assert!(kubectl.root_aliases.iter().any(|alias| alias == "k"));
    }

    #[test]
    fn debug_provider_rejects_unknown_provider() {
        let error = inspect_provider("unknown").expect_err("unknown provider should fail");
        assert_eq!(error.to_string(), "unknown provider: unknown");
    }
}
