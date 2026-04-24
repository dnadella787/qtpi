# 2cp

`2cp` is a Rust-first terminal autocomplete project aimed at IntelliSense-style dropdown suggestions for command-line workflows across multiple CLIs.

The immediate target is a local installable tool that helps users discover and complete commands from inside the terminal, starting with `zsh` and proving the model first on `git`.

## Current Status

Phase 4 is implemented as a multi-CLI built-in prototype with the first interactive `zsh` path and the first bounded dynamic lookup. The workspace now includes the Phase 1 Rust-side foundation, a suggestion engine for built-in `git` and `kubectl`, a shell-facing `twocp suggest` command, a thin `zsh` bridge that renders suggestions below the prompt with shell-owned state, and a live `kubectl` pod-name lookup path with explicit cache and degraded-mode behavior.

The current interactive path remains intentionally narrow:

- `zsh` only
- built-in `git` and `kubectl` fixtures
- static command and subcommand suggestions
- enum-backed value suggestions such as `kubectl get --output `
- dynamic pod-name lookup for `kubectl describe pod <name>` and `kubectl logs <pod>`
- explicit 2cp keybindings that leave native completion unchanged

External plugin runtime loading and additional CLIs remain future work. Dynamic lookup is intentionally narrow in the current build and only covers the first `kubectl` pod-name slot.

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

Build Phase 5: harden packaging, doctor/debug flows, shell install and uninstall paths, and broader failure recovery around the existing `zsh` bridge and provider runtime.

## Zsh Prototype

Build the binary, then source the repo-local bridge:

```bash
cargo build
TWOCP_BIN="$PWD/target/debug/twocp"
source "$PWD/shell/zsh/twocp.zsh"
```

Current interaction:

- type `git ` to auto-show built-in static suggestions
- keep typing after `git `, such as `git ch`, to narrow to `checkout` and `cherry-pick`
- after later spaces under a supported root, such as `git commit `, the menu reopens for the next flags or values
- type `kubectl ` or `k ` to auto-show the built-in kubectl command tree
- keep typing after `kubectl `, such as `kubectl get po`, to narrow to `pods`
- after later spaces under kubectl, such as `kubectl get `, the menu reopens for the next resources or flags
- type `kubectl describe pod `, then press `Ctrl-X 2 s` to trigger bounded live pod-name lookup
- the selected row is rendered with an inverted highlight
- `Down` / `Up` move the selection while the 2cp menu is visible, and otherwise keep their original shell behavior
- `Ctrl-X 2 j` / `Ctrl-X 2 k` also move the selection while the 2cp menu is visible
- `Enter` accepts the highlighted 2cp suggestion while the menu is visible, and otherwise keeps normal shell Enter behavior
- `Ctrl-X 2 a` accepts the highlighted 2cp suggestion
- `Ctrl-X 2 d` dismisses the 2cp menu
- `Tab` and normal `zsh` completion remain owned by the user's existing shell setup

The dropdown shows at most 5 rows by default while keeping a larger candidate
set loaded for scrolling. Override visible rows with `TWOCP_MAX_ROWS` and the
candidate cap with `TWOCP_MAX_SUGGESTIONS`.

Auto-show roots default to `git`, `kubectl`, and `k`. Override them before
sourcing the bridge with `TWOCP_AUTO_ROOTS`.

The default 2cp keybindings can be overridden before sourcing the bridge with
`TWOCP_KEY_SHOW`, `TWOCP_KEY_ACCEPT`, `TWOCP_KEY_DISMISS`, `TWOCP_KEY_NEXT`, and
`TWOCP_KEY_PREVIOUS` using `bindkey` notation. `Enter` defaults to both `^M`
and `^J`, and can be overridden with `TWOCP_KEY_ENTER` and
`TWOCP_KEY_ENTER_ALT`.
