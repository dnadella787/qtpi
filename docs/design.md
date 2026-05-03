# qtpi Design Doc

## Summary

`qtpi` is a local Rust application that adds IntelliSense-style autocomplete to the terminal without replacing the shell. It is designed as a multi-CLI suggestion system with a thin shell bridge, a shared provider model, deterministic ranking, and lightweight dropdown rendering.

The product starts with `zsh` as the first shell target and `git` as the first validation target, but the architecture is intended to support additional CLI ecosystems such as `aws`, `kubectl`, `argo`, and `oci` without redesigning the shell integration layer.

## Problem

Terminal users often know the tool they want but not the exact command tree, subcommand sequence, or flag combination. Existing tab completion helps only when the user already knows the shape of the command.

This gets worse for large CLI surfaces such as:

- `aws`
- `kubectl`
- `argo`
- `oci`
- `git`

These tools have deep subcommand trees, inconsistent flag naming, and context-sensitive workflows. Users lose time searching help output, consulting docs, or repeatedly trial-and-erroring commands.

## Goals

- provide real-time dropdown suggestions while the user types in the shell
- support multiple CLI ecosystems through a shared provider interface
- keep installation simple and local-first
- preserve normal terminal and shell workflows
- keep startup time and per-keystroke latency low enough to feel immediate
- support richer contextual suggestions later without changing the core shell contract

## Non-Goals

- replacing the user’s terminal or shell
- building a general-purpose AI agent inside the shell
- depending on network services for baseline suggestions
- supporting every shell equally from day one
- implementing full personalization or learned ranking in the first release

## Primary Use Cases

- type `git ch` and see ranked suggestions like `checkout` and `cherry-pick`
- type `kubectl get ` and see relevant resource-oriented continuations
- type `aws s3 ` and see common subcommands and likely next steps
- type `argo ` and discover verbs and object categories without leaving the shell
- type `oci iam ` and navigate the command tree without memorizing it

## User Experience Principles

- the shell remains the source of truth for the input line
- suggestions augment typing; they do not take over the whole screen
- the interface must be keyboard-friendly and reversible
- failure should degrade quietly back to normal shell behavior
- the product must remain useful even when only static command metadata is available

## Product Constraints

- local binary, no hosted dependency on the critical path
- shell-specific integration must stay thin and removable
- CLI-specific knowledge must not leak into rendering or shell bridge logic
- dynamic context collection must be bounded, explicit, and cache-aware
- installation must be understandable to a single developer on their own machine

## High-Level Architecture

The system is split into five layers:

1. `shell bridge`: captures enough shell state to request suggestions and apply accepted completions
2. `core runtime`: orchestrates provider loading, ranking, rendering, and lifecycle behavior
3. `provider layer`: exposes normalized command metadata and optional dynamic context for each CLI
4. `ranking engine`: scores and filters candidate suggestions based on typed input and context
5. `render layer`: paints and clears a dropdown-style suggestion view without replacing the shell

This is intentionally a single local process by default. A background daemon is not the baseline design. The extra complexity is not justified until startup or context gathering measurements demand it.

## Latency and Interaction Budgets

The architecture should be judged against explicit interactive budgets rather than vague speed goals.

Initial budgets:

- keystroke-to-suggestion refresh must stay within normal interactive typing tolerance on a warm machine
- a static-only suggestion path should avoid any subprocess requirement
- dynamic providers must have bounded execution time and must not block the baseline static path
- live resource lookup such as `kubectl describe pod <name>` must use scoped queries and bounded result sets
- rendering work must remain proportional to visible suggestion rows, not full terminal size

These are design constraints now. Exact numeric budgets can be locked during implementation once measured on a real machine.

## Component Model

### Shell Bridge

The shell bridge is shell-specific code, starting with `zsh`, that:

- hooks into interactive line editing at controlled points
- gathers the current input line, cursor position, and shell context
- invokes `qtpi` with a structured request
- renders or coordinates rendering of suggestion output
- accepts a chosen suggestion and updates the shell buffer predictably

The bridge must stay thin. It should not duplicate provider logic, ranking rules, or command parsing that properly belong in Rust.

## End-to-End Interaction Model

The default execution model is stateless per request:

1. user types in the shell
2. `zsh` hook captures the current editable buffer and cursor position
3. shell bridge sends a request to `qtpi`
4. Rust engine parses input, selects a provider, computes suggestions, and emits a structured response
5. shell bridge renders the response and applies user selection back into the shell buffer

This stateless model remains the baseline until measurements show it cannot meet interactive budgets.

### Trigger For A Resident Helper

A long-lived helper process is not introduced preemptively. It becomes reasonable only if at least one of these is true:

- warm-start invocation still misses interactive latency budgets after straightforward optimization
- one or more providers require expensive local initialization that cannot be cached cheaply in-process
- shell redraw coordination becomes materially simpler with a resident state holder

If a helper is introduced later, the shell-to-engine request and response contract should remain stable.

## Engine I/O Contract

The shell bridge and Rust engine must communicate through a canonical request and response model.

### Request Snapshot

Each request should include:

- `shell`: shell identifier such as `zsh`
- `buffer`: full editable line buffer
- `cursor_byte_offset`: cursor location in the buffer
- `cwd`: working directory
- `env_hints`: only a minimal allowlisted subset when required
- `terminal_capabilities`: color, cursor movement, and sizing hints if needed for rendering decisions
- `mode`: suggest, accept, dismiss, doctor, or debug

Important contract rule:

- the shell bridge passes raw buffer plus cursor state
- Rust owns tokenization, parse state, quoting rules, and provider resolution

This keeps parsing semantics centralized and prevents each shell adapter from inventing its own command model.

Initial parser support boundary:

- single-line interactive buffers only
- plain token boundaries
- single quotes, double quotes, and backslash escaping
- cursor-aware partial-token detection
- no command substitution, shell expansion, arrays, comments, or multiline continuation handling in the first interactive path
- unsupported syntax should produce an explicit degraded parse result instead of speculative completion

### Parse State

The engine derives a normalized parse result from the request:

- tokens before the cursor
- active token under the cursor
- quote state
- escape state
- whether the cursor is in command position, subcommand position, flag position, or value position
- selected provider root if any

Provider-root selection rule:

- the first command token must match a known provider root exactly
- matching expansion logic does not apply to the first command token
- prefix or fuzzy-style matching is allowed only after the provider root has been selected

For the first `zsh` implementation, parse correctness is intentionally narrower than general shell-language correctness. The engine should support the interactive subset above well before attempting broader shell syntax coverage.

### Response Shape

The engine response should include:

- `replace_range`: byte range in the shell buffer to replace on acceptance
- `suggestions`: ordered candidate list
- `selection_index`: default highlighted suggestion
- `render_model`: rows, annotations, truncation metadata, and style hints
- `status`: ok, no-match, degraded, or error
- `diagnostics`: optional debug-only timing and provider information

The response should describe what to render and what to replace, but it should not embed shell-specific imperative behavior.

### Core Runtime

The Rust runtime owns:

- request parsing
- provider registry construction
- suggestion candidate generation
- ranking and filtering
- render-model generation and response assembly
- diagnostics and developer commands

This is the stability boundary for the rest of the project. Shell adapters and CLI providers should depend on narrow contracts instead of cross-cutting internal behavior.

### Provider Layer

Providers model individual CLI ecosystems. A provider is responsible for:

- exposing a command tree or equivalent metadata
- mapping typed token context to a provider-local scope
- optionally supplying dynamic values such as branches, kube contexts, profiles, or regions
- supporting runtime search for scoped value positions when the CLI semantics require live objects
- declaring cache policy and refresh cost

The provider layer must support both:

- static metadata providers
- dynamic context providers

Static metadata is the default path. Dynamic sources should be additive and optional.

Provider metadata policy:

- AI generates provider source data
- a build tool compiles provider source data into a compact runtime artifact
- built-in providers are embedded in the main binary
- external plugins are loaded from compiled provider packages
- the runtime never parses large JSON, YAML, or TOML files on the keystroke path

Plugin loading policy:

- built-in providers are available immediately at process start
- external plugin providers should be lazy-loaded only after exact provider-root selection
- provider-root discovery for external plugins should come from a small startup index or manifest rather than package scanning on the hot path
- the runtime should not deserialize every plugin package at startup when only one CLI family is in use
- the initial implementation should prove the built-in provider path first; external plugin runtime loading can land after the first working `zsh` interaction loop

## Provider Capabilities And Cost Model

Providers need explicit capability boundaries.

Each provider should declare:

- whether it supports static command metadata
- whether it supports dynamic values
- which dynamic value classes it can supply
- whether it requires subprocess execution
- expected cacheability and invalidation signals

Dynamic providers should expose cost semantics such as:

- cheap and synchronous
- bounded subprocess
- cache-required
- unavailable in degraded mode

This prevents high-cost providers from silently contaminating the typing path.

Runtime lookup examples include:

- `kubectl describe pod <partial-name>` searching pods in the active namespace or selected scope
- `kubectl logs <pod>` searching live pod names
- cloud CLIs resolving profile, cluster, or region names from local state

### Ranking Engine

The ranking engine converts a typed input state into an ordered list of suggestions.

It should begin with deterministic rules:

- exact prefix match
- token position relevance
- CLI-local scope match
- common command priority
- context match when available

The engine should remain explainable. If a suggestion appears above another, the scoring should be attributable to clear rules rather than opaque heuristics.

### Render Layer

The render layer controls a compact dropdown-style suggestion display. Its responsibilities are:

- paint suggestions near the current prompt context
- handle clearing and redraw without leaving terminal artifacts
- preserve cursor location correctly
- degrade cleanly if terminal capabilities are limited

This should use lightweight terminal control rather than a full-screen TUI model.

## Data Model

The system needs a normalized command-spec model shared across providers. At minimum:

- `ProviderId`
- `CommandPath`
- `CommandNode`
- `ArgumentSpec`
- `FlagSpec`
- `Suggestion`
- `SuggestionKind`
- `DynamicValueSource`
- `DynamicLookupRequest`
- `DynamicLookupScope`
- `DynamicLookupBudget`
- `DynamicLookupResult`
- `LookupMatch`
- `CachePolicy`

Suggested semantics:

- `CommandNode` represents a CLI node such as a root command, subcommand, or conceptual action group
- `FlagSpec` models long flags, short aliases, value expectations, and repeatability
- `ArgumentSpec` models positional arguments and their suggestion sources
- `Suggestion` is the rendered candidate unit, separate from provider internals

This model must preserve enough structure to support both narrow CLIs like `git` and very broad CLIs like `aws`.

The command model should also explicitly account for:

- aliases
- hidden or deprecated commands
- mutually exclusive flags
- repeatable flags
- positionally constrained values
- enumerated value domains
- shell quoting requirements for inserted values
- provider-specific annotations such as safety or scope labels

Dynamic lookup types should carry these semantics:

- `DynamicLookupRequest`: one value-slot lookup attempt, including provider scope, partial user input, active argument or flag slot, and per-request execution budget
- `DynamicLookupScope`: normalized lookup scope such as namespace, resource kind, profile, region, cluster, repository, or working directory
- `DynamicLookupBudget`: hard limits for timeout, candidate count, and subprocess allowance
- `LookupMatch`: one raw provider result before ranking normalization, including replacement text, display label, optional annotation, confidence, and freshness metadata
- `DynamicLookupResult`: provider output containing matches, cache status, degradation state, and timing metadata

## Provider Interface

Providers should implement a narrow interface conceptually similar to:

```rust
trait Provider {
    fn id(&self) -> ProviderId;
    fn command_tree(&self) -> &CommandNode;
    fn resolve_scope(&self, request: &SuggestRequest) -> ProviderScope;
    fn static_suggestions(&self, scope: &ProviderScope) -> Vec<Candidate>;
    fn dynamic_lookup(
        &self,
        request: &DynamicLookupRequest,
        ctx: &DynamicContext,
    ) -> DynamicLookupResult;
}
```

This is still an architectural sketch, but the request and response shape should be treated as contractual.

Suggested lookup types:

```rust
struct DynamicLookupRequest {
    provider_id: ProviderId,
    command_path: CommandPath,
    slot_id: SlotId,
    partial_input: String,
    scope: DynamicLookupScope,
    budget: DynamicLookupBudget,
    allow_stale_cache: bool,
}

struct DynamicLookupScope {
    namespace: Option<String>,
    resource_kind: Option<String>,
    profile: Option<String>,
    region: Option<String>,
    cwd: std::path::PathBuf,
}

struct DynamicLookupBudget {
    timeout_ms: u32,
    max_candidates: u16,
    allow_subprocess: bool,
}

struct LookupMatch {
    value: String,
    display: String,
    annotation: Option<String>,
    confidence: u16,
    requires_quoting: bool,
    is_stale: bool,
}

struct DynamicLookupResult {
    matches: Vec<LookupMatch>,
    cache_status: CacheStatus,
    degraded: bool,
    lookup_time_ms: u32,
}
```

Concrete contract rules:

- `dynamic_lookup` is only called for resolved value positions, not for root-command discovery
- the provider must receive a normalized `slot_id` so it knows exactly which argument or flag value is being completed
- `partial_input` is already tokenized relative to shell quoting rules before it reaches the provider
- `max_candidates` is a hard cap; providers must not return unbounded result sets
- providers may return stale cached results only when the request permits it
- degraded results must be explicit so ranking and rendering can label or suppress them appropriately

Important design rules:

- provider code must not know about shell-specific hooks
- provider output must be normalized before ranking
- dynamic behavior must declare cost and caching expectations
- providers should be testable without a real shell session

Providers should be split conceptually into:

- spec providers: static command tree and flag structure
- value providers: dynamic values for arguments and flags

One concrete CLI integration may implement both, but the architecture should keep those concerns separable.

## Dynamic Lookup Lifecycle

Dynamic lookup should follow a fixed lifecycle:

1. parser resolves the current token as a value slot
2. provider scope is resolved from command path and context
3. engine constructs `DynamicLookupRequest` with bounded budget
4. provider returns `DynamicLookupResult`
5. engine normalizes `LookupMatch` items into ranked suggestions
6. rendering receives only bounded normalized suggestions

This keeps live search contractual and measurable instead of ad hoc.

### Slot Resolution Rules

Dynamic lookup should only run when all of the following are true:

- the provider root has already been selected exactly
- the parser knows the current slot expects a dynamic value
- the provider advertises support for that lookup class
- the request budget allows the needed lookup path

Examples:

- `kubectl describe pod <partial>` resolves a pod-name slot and invokes live lookup scoped by active namespace and resource kind
- `kubectl config use-context <partial>` resolves a context-name slot and may use cached local context data
- `aws --profile <partial>` resolves a profile slot and may read local configured profiles

## Static Versus Dynamic Suggestions

The design should distinguish three classes of suggestions:

1. static command structure
2. local dynamic context
3. user-state or history-derived hints

Initial priority:

- static command structure is required
- local dynamic context is optional
- live scoped value search is supported where it materially improves usability
- user-state and history-derived hints are deferred

Examples:

- static: `git checkout`, `kubectl get`, `aws s3 ls`
- local dynamic: git branch names, kube namespaces, AWS profile names
- live search: pod names for `kubectl describe pod`, deployment names for targeted resource commands
- deferred: personal frequency ranking, recent commands, learned workflows

## Parsing and Request Model

The request from the shell bridge to the Rust binary should include:

- current full input line
- cursor position
- shell identifier
- terminal capability hints when relevant
- working directory
- environment-derived context only when needed and safe

The Rust side must parse:

- tokens before the cursor
- partial token under the cursor
- provider root selection
- whether the user is completing a command, subcommand, flag, or argument value

This parsing should be lightweight and incremental in spirit, even if the first implementation reparses the visible input line each time.

Quoting and escaping correctness are part of the core engine contract, not an implementation detail. This is especially important for cloud CLIs, where values frequently contain slashes, colons, equal signs, or shell-sensitive characters.

Parse output must also identify the active `slot_id` for value completion so providers can perform targeted dynamic lookup instead of inspecting raw token lists themselves.

The v1 parser should explicitly reject or degrade unsupported shell constructs rather than attempting partial shell emulation. Shipping a smaller correct subset is preferable to returning misleading replacements from an ambiguous parse.

## Ranking Strategy

The initial ranking system should be deterministic and modular.

The ranking path should be staged:

1. parse request into normalized scope
2. retrieve raw candidates from the active provider
3. filter invalid candidates for the current slot
4. score surviving candidates deterministically
5. group and annotate display rows
6. truncate to a bounded visible set

Suggested scoring factors:

- prefix quality
- light fuzzy scoring only after prefix quality is weak and only after provider selection
- position fitness for the current token slot
- command popularity weight curated per provider
- context affinity for the current directory or scope
- provider confidence when matching dynamic values

Hard rules:

- suggestions that are invalid in the current scope should not surface
- the first command token is not matched fuzzily or by partial expansion; it is used only for exact provider selection
- fuzzy matching after the first token must stay lightweight and bounded; it is not typo-tolerant freeform search
- live value lookup must be scoped by the resolved command context before fuzzy matching is applied
- noisy breadth should be capped aggressively
- deterministic ranking must come before fuzzy cleverness

Broad CLIs such as `aws` and `kubectl` need strong scope filtering to avoid useless menus.

Display truncation is part of the ranking pipeline, not just rendering. The engine should decide the bounded candidate set before terminal paint work begins.

Dynamic lookup results should enter the ranking pipeline after provider normalization, not through a separate ad hoc code path.

## Shell Integration Strategy

`zsh` is the first shell target because its interactive completion and line editor hooks are mature enough to prove the product.

The initial `zsh` bridge should:

- install with a clearly named script
- register explicit hooks
- allow easy disablement
- fail open back to normal shell behavior if `qtpi` is unavailable

The shell bridge should pass only structured state, not CLI-specific logic. Command-specific intelligence belongs in providers.

## Zsh Bridge Contract

The `zsh` bridge needs to be specified as a product contract, not deferred to implementation.

Design assumptions:

- `zle` remains the owner of the editable shell buffer
- `qtpi` augments normal editing; it does not replace the prompt or become a replacement shell
- suggestion lifecycle is driven by explicit `zle` hook points and key handlers

Minimum bridge responsibilities:

- capture `BUFFER`, `CURSOR`, and current working directory
- invoke `qtpi` on bounded suggestion triggers
- render the dropdown from Rust-provided row data without taking ownership of the full terminal session
- map accept, dismiss, and movement keys to shell-buffer-safe operations
- clear stale UI on prompt redraw or known invalidation events

Chosen `zsh` interaction contract:

- stay on `zsh` only for now; no active `bash` or `fish` design work is in scope
- request suggestions on a short debounce during normal typing, not on every raw keystroke
- request immediately on explicit movement into a new token boundary such as space-delimited subcommand progression
- use arrow keys for dropdown movement when the menu is visible
- use `Enter` to accept the highlighted suggestion when the token is already narrowed or after explicit row movement
- use `Esc` to dismiss the dropdown
- use `Enter` to submit the shell buffer normally when no dropdown is visible
- preserve access to native `zsh` completion on a separate explicit path instead of silently replacing it

Recommended hook bias:

- a custom `zle` widget should wrap self-insert-driven refresh behavior behind debounce
- acceptance and dismissal should be handled by dedicated widgets while the menu is active
- prompt redraw and line-finish hooks should clear any visible dropdown state

Lifecycle states:

1. idle
2. suggestion requested
3. dropdown visible
4. selection moved
5. suggestion accepted or dismissed
6. cleanup completed

Ownership rules:

- `zle` owns the authoritative line buffer
- Rust owns parse state, candidate generation, ranking, and replacement semantics
- Rust returns a shell-agnostic `render_model` instead of issuing terminal control directly in the first interactive path
- the bridge owns dropdown painting, key mapping, and shell-safe application of accepted edits

Coexistence rules:

- the bridge must not permanently break native completion widgets
- users must be able to disable `qtpi` and fall back to normal completion
- prompt frameworks or custom widgets should not need deep integration to function

Future shell targets:

- `bash`, far later if the `zsh` model proves stable
- `fish`, only after the provider and rendering contracts are already mature

They should reuse the same Rust request and response model as much as possible.

## Rendering Strategy

The preferred UI is a lightweight dropdown anchored to the active shell prompt.

Requirements:

- does not switch to an alternate screen
- handles repeated redraws safely
- preserves prompt and cursor integrity
- keeps suggestion count bounded
- supports highlight movement and selection
- clears itself on acceptance, dismissal, or shell redraw

If terminal capabilities are insufficient, degraded behavior may include:

- inline textual suggestions
- reduced styling
- fewer visible rows
- temporary disablement with a clear diagnostic path

The rendering boundary should be explicit:

- the engine decides candidate order, labels, annotations, and replacement semantics
- the shell bridge decides how those rows are painted within shell-specific constraints

This allows the same engine response to support multiple shells later.

For v1 specifically:

- Rust should not own terminal painting or cursor control
- `zsh` should paint and clear the dropdown using the structured response
- any future move toward shared terminal-control code should be justified by repeated shell-level rendering pain, not assumed up front

The repaint algorithm should also define:

- how many rows may be drawn
- where the dropdown appears relative to the prompt and cursor
- how cursor restoration is computed after each draw
- what happens when the prompt redraws independently
- how stale rows are cleared after resize, interrupt, or background output

## Installation and Distribution

The preferred installation model is:

- one Rust binary
- one shell integration script per supported shell
- built-in provider metadata embedded in the binary
- optional plugin provider artifacts installed alongside it in a runtime-efficient binary format

Distribution options, in likely order:

1. local `cargo install` during development
2. prebuilt release binaries
3. package-manager formulas later if adoption warrants it

Installation must be reversible. The user should be able to remove a shell hook and return to normal behavior without manual cleanup across many files.

Provider packaging should distinguish between:

- authoring artifacts meant for humans or AI generation
- runtime artifacts meant for fast loading during interactive use

The expected flow is:

1. AI generates provider source data
2. a build tool compiles that source data into a compact runtime artifact
3. built-in providers are embedded into the binary at build time
4. external plugins are shipped as compiled provider packages

Early implementation priority:

- prove the built-in provider path first
- defer external plugin runtime loading until after the first working `zsh` plus `git` interaction loop
- if lazy external loading is introduced later, pair it with a compact provider index that maps exact root commands to package locations and compatibility metadata

JSON, YAML, or TOML are acceptable authoring formats, but they are build inputs only. The runtime should never parse large text metadata files on the interactive suggestion path.

Recommended runtime package format:

- compiled provider packages should use a compact binary table format optimized for fast deserialization and indexed lookup
- `postcard`-style compact encoding is a good baseline if the schema stays simple
- if provider packages need stronger random-access behavior later, move to a custom indexed binary format without changing the authoring flow

The immediate default is a compact `postcard`-encoded provider package with an explicit schema version.

### Install Shape

The install story should eventually support:

- binary installation
- `zsh` hook installation
- uninstall and cleanup
- upgrade-safe config preservation

Expected install surfaces:

- binary path
- shell integration script
- user config file
- optional cache directory

Recommended default locations:

- config under the user config directory for the platform
- cache under the user cache directory for the platform
- shell hook as a clearly named sourced file

The project should provide explicit commands or scripts for:

- install
- uninstall
- doctor
- print-shell-hook

The first development path can be manual, but the design should preserve a clean path to release binaries and package-manager distribution.

## Performance Targets

The project should define explicit performance budgets early.

Initial target budgets:

- warm end-to-end suggest path should target p95 under 20 ms for the static `git` path on a warm development machine
- startup on a warm machine should feel near-instant and remain comfortably below the typing cadence budget
- steady-state suggestion refresh should stay within interactive typing tolerance
- provider lookup should avoid repeated expensive subprocess calls
- rendering should avoid excessive full-line redraw churn

Decision gates:

- the default static suggestion path should require no subprocesses
- if the stateless per-request model misses the warm-path budget after straightforward optimization, revisit a resident helper with measurements in hand
- if shell-side painting proves too fragile, revisit the render ownership boundary explicitly instead of drifting into mixed responsibilities

The exact thresholds should be measured in implementation, but the architecture should already assume:

- low allocation on hot paths
- bounded candidate sets
- caching for expensive dynamic sources
- no network I/O in the typing path

## Caching Strategy

Caching should be explicit, provider-scoped, and conservative.

Good early cache candidates:

- git branches for the current repo
- kube contexts and namespaces
- recently fetched Kubernetes resource names scoped by command kind and namespace
- cloud profile names
- parsed static command metadata

Cache rules must specify:

- key
- lifetime
- invalidation trigger
- refresh policy
- stale behavior

Avoid hidden background refresh machinery in the first version.

Provider caches must also specify whether stale reads are acceptable during active typing or whether the engine should fall back to static-only suggestions instead.

For live object lookup, caches should also specify:

- query scope such as namespace, resource kind, or profile
- maximum result count returned to the ranking layer
- whether a stale cached result is better than issuing a fresh lookup on the current keystroke
- whether lookup failures should suppress results entirely or surface cached degraded matches

## Error Handling and Failure Modes

The product must fail safely under:

- missing binary
- broken shell hook installation
- malformed provider data
- terminal redraw mismatch
- dynamic provider timeout or subprocess failure

Expected failure posture:

- do not corrupt the shell input buffer
- do not leave persistent visual garbage when possible
- do not block normal typing waiting on expensive dynamic context
- emit diagnostics through explicit debug or doctor commands rather than noisy prompt output

High-risk failure classes that must be treated explicitly:

- tokenization or quoting mismatch between shell expectations and engine parsing
- stale dynamic values that lead to misleading replacements
- overly broad live lookups that block typing or return unusable result sets
- provider metadata drift across CLI versions
- redraw desynchronization after prompt updates or terminal resize
- broad provider scopes producing noisy, low-value menus

## Failure Matrix

The design should explicitly cover these cases:

- terminal resize while dropdown is visible
- prompt repaint from shell theme or async prompt component
- `Ctrl-C` during active suggestion display
- background job output interleaving with the dropdown
- nested terminals such as `tmux`
- remote sessions over SSH
- non-interactive or non-TTY execution
- unsupported terminal capabilities
- crashed or missing `qtpi` binary

Expected recovery behavior:

- clear the dropdown if safe to do so
- return control to the normal shell buffer immediately
- auto-disable rich rendering if repeated failures are detected
- preserve enough local diagnostics to debug the event later

## Security and Trust Model

The product runs locally inside an interactive shell context, so its trust surface must stay small.

Guidelines:

- no network dependency on the hot path
- do not execute arbitrary shell fragments from provider metadata
- keep provider subprocess usage allowlisted and auditable
- treat environment and working-directory derived context as untrusted input
- make installation changes explicit

Cloud CLIs may expose account, region, or context information. Dynamic providers should avoid collecting more data than needed for the immediate suggestion scope.

## Observability and Diagnostics

The system should support explicit developer-facing diagnostics.

Recommended early commands:

- `qtpi doctor`
- `qtpi debug request`
- `qtpi debug provider <name>`
- `qtpi debug render`

Useful diagnostics:

- active shell integration status
- provider load status
- cache hit and miss behavior
- live lookup query scope and result count
- measured timing for suggestion generation
- recent rendering failures

Diagnostics must be opt-in and separate from the normal interactive UX.

The design should also capture decision-trigger metrics, especially:

- cold and warm request latency
- time spent in provider resolution
- time spent in dynamic value fetches
- rendered row count and truncation frequency
- error and degraded-mode rates

Recommended observability surfaces:

- local debug log file
- trace mode for a single request lifecycle
- structured timing output for provider and render stages
- a user-safe bug-report workflow based on local artifacts rather than prompt spam

## Testing Strategy

Testing should be layered.

### Unit Tests

Cover:

- tokenization and scope resolution
- command-spec parsing
- ranking rules
- provider-specific metadata interpretation
- render diff or state logic where practical
- quoting and escaping edge cases
- replacement range correctness
- scoped live-lookup query construction
- slot resolution for dynamic lookup requests
- budget enforcement and result truncation

### Integration Tests

Cover:

- request and response contracts between shell bridge and core binary
- provider registration and selection
- dynamic provider timeout handling
- install and uninstall behavior for shell integration
- CLI version drift handling where provider metadata is generated or imported
- PTY-driven `zsh` interaction flows
- failure cleanup after redraw and interrupt scenarios
- live lookup behavior for commands such as `kubectl describe pod`
- degraded lookup behavior when subprocess or cache policy limits are hit

### Manual Validation

Required for:

- actual terminal redraw behavior
- cursor preservation
- prompt interaction
- behavior in at least one large non-`git` CLI

Manual testing should include at least:

- `git`
- one of `aws`, `kubectl`, `argo`, or `oci`

### Platform Coverage

The rollout plan should eventually cover:

- macOS, because terminal-shell tooling is common there
- Linux, because CI and many developer environments live there

CI can remain lightweight initially, but the design should assume that shell integration and packaging smoke tests will need platform coverage beyond a single Linux Rust build.

## Recommended Repository Evolution

The current single-crate scaffold is acceptable for now, but the likely medium-term layout is:

- `crates/qtpi-cli`: CLI entrypoint and developer commands
- `crates/qtpi-core`: request model, provider registry, ranking engine
- `crates/qtpi-render`: render-model shaping and any future shared terminal-control helpers justified by real shell-painting pain
- `crates/qtpi-shell-zsh`: packaging or assets for zsh integration
- `crates/qtpi-providers`: shared provider interfaces and built-in providers

This split should happen only when real code pressure justifies it. Premature crate fragmentation will slow the project down.

## Dependency Direction

Allowed dependency direction should remain one-way:

- shell-specific adapters depend on core request and response types
- providers depend on shared command model types
- ranking depends on normalized candidate types, not shell code
- rendering depends on normalized render models, not provider internals
- CLI entrypoint can compose everything, but deeper layers should not depend upward on it

Disallowed coupling:

- providers importing shell hook code
- rendering code reading raw provider metadata directly
- shell adapters owning provider-specific parsing
- ranking code spawning arbitrary subprocesses

## Delivery Plan

### Phase 1: Core Contracts And Build Pipeline

Scope:

- establish the crate and module boundaries needed for future work
- define the canonical request and response types
- define the command-spec model and dynamic lookup types
- define the initial parser support boundary and degraded-mode behavior for unsupported shell syntax
- implement the provider trait boundary and registry skeleton
- add the provider build tool that compiles source data into runtime artifacts
- wire the built-in provider embedding path

Deliverables:

- `qtpi-core`-style core model modules or their equivalent inside the existing crate layout
- request, parse, suggestion, and render-model types compiled and tested
- provider interface with `dynamic_lookup` request and response contracts
- compiled-provider artifact schema with explicit versioning
- build-tool scaffold that turns provider source data into runtime artifacts
- at least one minimal built-in provider fixture compiled into the binary for smoke validation

Exit criteria:

- the workspace builds and tests cleanly
- provider source data can be compiled into a runtime artifact
- the binary can load an embedded built-in provider fixture
- core contracts are documented in code and aligned with this design doc
- no shell integration code is required yet

### Phase 2: First Interactive Path With Git

Scope:

- build the first `zsh` bridge
- implement static `git` provider behavior using the compiled-provider pipeline
- return structured suggestions to shell glue
- support dropdown movement, acceptance, and dismissal
- validate the bounded parser subset and degraded behavior in real `zsh` interaction
- measure the warm static path against the initial latency budget

Deliverables:

- `zsh` hook script or packaging assets
- debounced request path from `zsh` into the Rust binary
- static `git` command and subcommand suggestions
- bounded suggestion list with selection metadata and replacement ranges
- acceptance flow using `Enter`, dismissal via `Esc`, and movement via arrow keys
- explicit fallback path to native `zsh` completion
- initial latency measurements for the warm static path

Exit criteria:

- typing `git ` or `git ch` in `zsh` shows stable structured suggestions
- accepting a suggestion updates the shell buffer correctly
- the menu clears correctly on dismissal, prompt redraw, and submission
- native `zsh` completion still has an explicit fallback path
- unsupported syntax degrades safely instead of producing misleading completions
- the warm static path is within budget or triggers an explicit follow-up decision on process model

### Phase 3: Multi-CLI Validation With Kubectl

Scope:

- add `kubectl` as the second validation CLI after `git`
- validate provider scale and ranking quality on a larger command tree
- harden exact-root selection, scoped matching, and provider-root indexing for future lazy plugin loading
- introduce external provider loading only after the built-in interactive path is stable

Deliverables:

- compiled `kubectl` provider artifact or built-in provider data
- ranking and truncation behavior tuned for a broader command surface
- tests for provider selection and `kubectl` command-path completion
- provider-root index or manifest contract for lazy external-provider discovery

Exit criteria:

- `kubectl` command and subcommand suggestions are usable in the same `zsh` flow
- the runtime does not eagerly load unrelated providers at startup
- ranking remains bounded and predictable on a large command tree
- exact root selection and provider discovery do not require scanning plugin packages on the hot path

### Phase 4: Dynamic Context And Live Lookup

Scope:

- add dynamic value lookup for high-value slots
- start with `kubectl` object-name completion and other local context sources
- introduce cache policy, degraded lookup behavior, and diagnostics for live search

Deliverables:

- dynamic lookup execution path using `DynamicLookupRequest` and `DynamicLookupResult`
- scoped live lookup for commands such as `kubectl describe pod <name>`
- cache keys, stale-read rules, timeout budgets, and degraded-mode behavior
- diagnostics for lookup timing, scope, and result counts

Exit criteria:

- live lookup works for at least one real `kubectl` value slot
- typing remains responsive under lookup budget limits
- stale or failed lookups degrade safely instead of blocking the shell

### Phase 5: Plugin Maturity, Packaging, And Hardening

Scope:

- stabilize plugin packaging and compatibility rules
- improve installation and uninstall flows
- harden observability, failure recovery, and release packaging
- finalize lazy external provider loading behavior on top of the provider-root index or manifest

Deliverables:

- stable plugin package versioning and compatibility checks
- install, uninstall, and doctor commands
- packaging for release binaries and reproducible plugin artifacts
- stronger PTY and platform validation
- external provider install and discovery flow built on the finalized index or manifest

Exit criteria:

- external compiled providers can be installed and loaded safely
- the install and uninstall flow is reversible and documented
- diagnostics are sufficient to debug shell, provider, and rendering failures

### Phase 6: Expansion And Refinement

Scope:

- expand provider coverage
- refine ranking and contextual behavior
- evaluate when additional shells are worth the support cost

Deliverables:

- more providers beyond `git` and `kubectl`
- better ranking policies for broad CLIs
- explicit decision record for or against later shell targets such as `bash`

Exit criteria:

- the core architecture supports additional providers without shell-bridge redesign
- new provider additions mostly involve source data, build artifacts, and focused tests

## Key Tradeoffs

### Single Process Versus Daemon

Start with a single process.

Reason:

- simpler installation
- fewer lifecycle problems
- easier debugging

Move to a daemon only if measured startup cost or repeated expensive provider work justifies it.

### Lightweight Terminal Control Versus Full TUI

Prefer lightweight terminal control.

Reason:

- the product augments shell input instead of owning the terminal session
- full-screen abstractions are more likely to fight prompt behavior

### Deterministic Ranking Versus Learned Ranking

Start deterministic.

Reason:

- easier to debug
- easier to test
- safer for large CLI surfaces

### Static Metadata Versus Dynamic Discovery

Start with high-quality static metadata and layer dynamic context selectively.

Reason:

- static metadata is cheap and reliable
- dynamic discovery adds latency, cache complexity, and failure modes

## Risk Register

- `zsh` hook behavior may vary across prompt plugins and line editor customizations
- quoting and escape handling can be subtly wrong even when basic tokenization looks correct
- broad CLIs such as `aws` can flood the candidate pool unless scope resolution is strict
- dynamic context may become stale or too expensive without provider-specific cache rules
- upstream CLI surface changes can drift away from bundled metadata
- rendering can desynchronize after resize, prompt repaint, or failed clears
- startup overhead may eventually justify a resident helper, but introducing one too early would add avoidable complexity

## Open Questions

- how provider source data should be validated before compilation into runtime artifacts
- what schema-versioning and compatibility guarantees plugin packages should provide across releases
- which live lookup sources beyond `kubectl` should be prioritized after the first runtime-search path lands
- whether broad CLIs like `aws` need command-family-specific ranking policies from the start

## Recommendation

Build `qtpi` as a Rust-first, single-process, multi-provider autocomplete engine with a thin `zsh` bridge and deterministic ranking. Use `git` to prove the interaction loop quickly, but design the provider model, request contract, and ranking engine from day one around broader CLI families such as `aws`, `kubectl`, `argo`, and `oci`.
