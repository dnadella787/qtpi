# Architecture Notes

## Recommended Shape

Keep the early architecture simple:

1. a local Rust binary
2. a thin `zsh` bridge that forwards input context
3. a suggestion engine that ranks continuations
4. CLI-specific command providers that expose metadata in a shared format
5. a shell-owned dropdown paint path driven by a Rust `render_model`

## Why This Shape

- it keeps installation understandable
- it limits shell-specific complexity
- it preserves room to split modules later without starting over

## Early Module Boundaries

These boundaries are useful even before separate crates exist:

- `cli`: startup, argument handling, developer commands
- `shell`: shell-bridge contracts and integration assets
- `suggest`: command metadata, matching, ranking
- `providers`: per-CLI metadata loaders and adapters
- `render`: normalized render-model types and row shaping, not shell-specific painting
- `context`: optional dynamic sources such as branches, kube contexts, profiles, or repository state

## Current Phase 1 Layout

The repo now uses a small three-crate split that matches the Phase 1 delivery plan without introducing shell or render crates early:

- `twocp-cli`: composition layer, developer commands, and embedded built-in fixture registration
- `twocp-core`: canonical request and response types, parser-boundary modeling, provider traits, registry, and compiled-artifact loading
- `twocp-build`: provider source-data parsing, validation, and runtime artifact compilation

This keeps authoring-format parsing out of the runtime crate while preserving a single artifact contract for both embedded built-ins and later external providers.

Phase 2 adds a repo-local `shell/zsh/twocp.zsh` bridge instead of a new shell crate. That keeps the early integration explicit and easy to remove while the widget contract is still settling.

Phase 3 keeps the same crate layout while adding a built-in provider-root index and a second validation CLI. Exact root discovery is now a small startup contract separate from provider instantiation, so the suggest path can select `git` or `kubectl` without eagerly deserializing unrelated providers.

Phase 4 keeps the same crate layout while adding provider-scoped value-slot resolution, enum-backed value suggestions, and the first bounded dynamic lookup path for `kubectl` pod names. The current live lookup path stays single-process and stateless per request, so cache policy is explicit and persisted outside process memory instead of relying on a resident helper.

## Rendering Bias

For the first interactive path, prefer shell-owned painting over Rust-owned terminal control.

Reason:

- `zle` already owns the editable buffer and prompt lifecycle
- a shell-owned paint path keeps redraw semantics explicit while Rust stays focused on parsing, ranking, and replacement ranges
- full-screen abstractions can fight the shell instead of augmenting it

Phase 1 stops short of painting entirely. The current Rust surface only shapes a normalized `render_model`; terminal control remains deferred to the future shell bridge.

The current Phase 2 bridge still follows that rule. Rust returns structured suggestions and replacement ranges, while `zsh` paints a bounded list using shell-owned display state instead of moving terminal control into Rust.

## Shell Strategy

Start with `zsh` only. The initial bridge should be explicit and easy to remove.

Good early properties:

- predictable hook points
- minimal shell script surface area
- clear handoff between shell state and Rust logic
- an explicit fallback path to native `zsh` completion

Initial parser boundary:

- support single-line buffers, plain token boundaries, quotes, and backslash escaping first
- reject or degrade unsupported shell constructs instead of guessing

## Ranking Strategy

Start deterministic:

- exact prefix matches first
- common command frequency rules second
- CLI-local structure and scope rules third
- light fuzzy matching only after provider selection and only as a bounded fallback
- context-sensitive suggestions later

Avoid opaque ranking heuristics before the basic interaction is stable.

## Provider Strategy

Keep CLI-specific knowledge behind a shared provider interface.

Good early properties:

- adding `aws`, `kubectl`, `argo`, or `oci` should mostly mean adding provider data and tests
- AI-generated provider source data should compile into runtime artifacts before use
- built-in providers should be embedded in the binary first, while external providers remain a later extension
- external plugin providers should eventually lazy-load after exact root-command selection using a small provider index or manifest
- provider code should not leak shell-specific concerns
- dynamic providers should make subprocess and cache behavior explicit
- dynamic providers must support scoped runtime lookup for value positions such as `kubectl describe pod <name>`
- dynamic lookup should use an explicit request/response contract with slot id, scope, budget, and bounded results
- the runtime should not parse large JSON, YAML, or TOML provider files on the keystroke path

Recommended package bias:

- use compact schema-versioned binary provider packages
- start with a `postcard`-style encoded artifact unless random-access needs force a custom indexed format

## Performance Constraints

Treat these as design constraints, not polish:

- fast startup
- low allocation on the keystroke path
- bounded work per suggestion refresh
- no unnecessary subprocess spawning while typing

Initial measurement gates:

- warm static suggest path target p95 under 20 ms for `git` on a warm development machine
- no subprocesses on the default static suggestion path
- revisit the single-process stateless model only if those budgets are missed after straightforward optimization

## Future Expansion

These can come later if justified:

- branch and ref suggestions
- Kubernetes resource and context suggestions
- live Kubernetes object lookup during value completion
- cloud profile and region suggestions
- learned ranking from local usage
- `bash`, far later after the `zsh` model is stable
- `fish`, only after the shell contract is mature
- richer metadata and inline help text
