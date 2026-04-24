# 2cp

`2cp` is a Rust-first terminal autocomplete project aimed at IntelliSense-style dropdown suggestions for command-line workflows across multiple CLIs.

The immediate target is a local installable tool that helps users discover and complete commands from inside the terminal, starting with `zsh` and proving the model first on `git`.

## Current Status

Phase 3 is implemented as a multi-CLI built-in prototype with the first interactive `zsh` path. The workspace now includes the Phase 1 Rust-side foundation, a static suggestion engine for built-in `git` and `kubectl`, a shell-facing `twocp suggest` command, and a thin `zsh` bridge that renders suggestions below the prompt with shell-owned state.

The current interactive path remains intentionally narrow:

- `zsh` only
- built-in `git` and `kubectl` fixtures
- static command and subcommand suggestions only
- explicit fallback to native completion via `Ctrl-X Tab`

External plugin runtime loading, dynamic value lookup, and additional CLIs remain future work.

## Technical Direction

- Rust for the core executable and suggestion engine
- `zsh` as the first shell integration target
- a thin shell bridge around a low-latency local binary
- a command-spec model that can support tools such as `git`, `aws`, `kubectl`, `argo`, and `oci`
- dropdown-style suggestion rendering without replacing the user's terminal

## Workspace Layout

- `crates/twocp-cli/`: CLI entrypoint plus developer-facing `build-provider` and built-in fixture smoke surface
- `crates/twocp-core/`: core contracts, parser-boundary types, provider interfaces, registry, and artifact loading
- `crates/twocp-build/`: provider source-data compiler for deterministic runtime artifacts
- `providers-src/`: minimal provider source-data fixtures compiled into embedded artifacts at build time
- `shell/zsh/`: repo-local `zsh` integration script for the first interactive path
- `docs/`: product, architecture, and implementation planning notes
- `docs/prompts/`: reusable Codex task prompts for repo-local agent work
- `skills/rust-terminal-autocomplete/`: repo-local skill for future AI agents
- `AGENTS.md`: repo-local operating guidance for agents

## Common Commands

```bash
cargo fmt --all
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Next Development Step

Build Phase 4: add dynamic value lookup for high-value slots such as kubectl object names and other local context, with explicit cache, degradation, and timing behavior.

## Zsh Prototype

Build the binary, then source the repo-local bridge:

```bash
cargo build
TWOCP_BIN="$PWD/target/debug/twocp"
source "$PWD/shell/zsh/twocp.zsh"
```

Current interaction:

- type `git ` to see built-in static suggestions
- type `git ch` to narrow to `checkout` and `cherry-pick`
- type `kubectl ` to see a broader static built-in command tree
- type `kubectl get po` to narrow to `pods`
- `Down` / `Up` move the selection while the menu is visible
- `Tab` accepts the highlighted suggestion
- `Esc` dismisses the menu
- `Ctrl-X Tab` calls native `zsh` completion explicitly
