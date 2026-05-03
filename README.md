# 2cp

`2cp` is a Rust-first terminal autocomplete project aimed at IntelliSense-style dropdown suggestions for command-line workflows across multiple CLIs.

The immediate target is a local installable tool that helps users discover and complete commands from inside the terminal, starting with `zsh` and proving the model first on `git`.

## Current Status

Phase 4 is implemented as a multi-CLI built-in prototype with the first interactive `zsh` path and targeted bounded dynamic lookup. The workspace now includes the Phase 1 Rust-side foundation, a suggestion engine for built-in `git` and `kubectl`, a shell-facing `twocp suggest` command, a thin `zsh` bridge with a zsh-owned 5-row overlay renderer driven by shell-managed selection state, and live provider-owned lookup paths for git branches plus high-value kubectl resource names, namespaces, and contexts with explicit cache and degraded-mode behavior.

The current interactive path remains intentionally narrow:

- `zsh` only
- built-in curated `git` and `kubectl` command trees
- static command, subcommand, and common flag suggestions
- enum-backed value suggestions such as `kubectl get --output `
- dynamic git branch lookup for `git checkout <target>`, `git switch <branch>`, `git merge <branch>`, and `git rebase <upstream>`
- dynamic kubectl lookup for common resource names, `--namespace`, `--context`, and `kubectl config use-context <context>`
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
zsh shell/zsh/twocp_state_test.zsh
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

- type `git ` to auto-show the expanded built-in git command set
- keep typing after `git `, such as `git ch`, to narrow to `checkout` and `cherry-pick`
- after later spaces under a supported root, such as `git checkout ` or `git commit `, the menu reopens for the next flags or values
- type `kubectl ` or `k ` to auto-show the built-in kubectl command tree
- keep typing after `kubectl `, such as `kubectl get po`, to narrow to `pods`
- after later spaces under kubectl, such as `kubectl get ` or `kubectl get pods `, the menu reopens for the next resources, names, or flags
- type `kubectl describe pod `, `kubectl get pods `, or `kubectl config use-context `, then press `Ctrl-X 2 s` to trigger bounded live lookup for names or contexts
- the menu is painted by `zsh` as a terminal overlay below the prompt instead of using `complist`, `menu-select`, or Rust-owned terminal drawing
- the visible window is fixed at 5 rows; if more than 5 candidates exist, the window scrolls only after the highlighted row leaves the visible slice
- the first ranked row is highlighted immediately on show and after typed refreshes
- `Down` / `Up` move the highlighted row one item at a time while the menu is visible, clamp at the ends, and otherwise keep their original shell behavior
- typing or backspacing while the menu is visible refreshes suggestions, resets the highlight to the first ranked row, and repaints the overlay in place
- `Enter` accepts the highlighted suggestion when the current token is narrowed or after explicit row movement; for auto-opened empty-fragment menus it keeps normal shell Enter behavior
- `Esc` dismisses the current suggestion list while it is visible
- `Ctrl-C` dismisses the current suggestion list and then falls through to native shell interrupt behavior
- `Tab` and normal `zsh` completion remain owned by the user's existing shell setup

The candidate set remains bounded by `TWOCP_MAX_SUGGESTIONS`. The prototype
dropdown intentionally renders exactly 5 visible rows and does not expose a row
count override. If the terminal cannot safely support cursor-addressed overlay
painting, 2cp disables the dropdown for that session instead of falling back to
`complist`.

Auto-show roots default to `git`, `kubectl`, and `k`. Override them before
sourcing the bridge with `TWOCP_AUTO_ROOTS`.

The default 2cp keybindings can be overridden before sourcing the bridge with
`TWOCP_KEY_SHOW` using `bindkey` notation. `Enter` defaults to both `^M`
and `^J`, and can be overridden with `TWOCP_KEY_ENTER` and
`TWOCP_KEY_ENTER_ALT`. `Esc` defaults to `^[` and can be overridden with
`TWOCP_KEY_ESCAPE`. `Ctrl-C` defaults to `^C` and can be overridden with
`TWOCP_KEY_INTERRUPT`.

The repo-local shell regression harness lives at `shell/zsh/twocp_state_test.zsh`.
It exercises show and typed-refresh state, scroll-window movement, clamping, and
clean invalidation directly against the internal zsh helpers. It does not try to
assert terminal cursor painting.

Manual terminal verification for the current overlay contract should record:

- the terminal emulator and `TERM` value used
- whether the highlight path used standout, reverse-video, or marker fallback
- that `git `, `git c`, `git co`, and `git com` narrow in place while keeping the menu visible when matches remain
- that moving down past row 5 scrolls the frame and moving up from the bottom keeps the highlight inside the frame before the frame scrolls upward
- that `Enter`, `Esc`, `Ctrl-C`, prompt redraw, and command submission clear the overlay cleanly
