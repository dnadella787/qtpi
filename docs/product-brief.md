# Product Brief

## Problem

Most terminals still make users memorize command trees, flags, and workflows. Tab completion helps only when the user already knows roughly what to type.

For tools like `git`, `aws`, `kubectl`, `argo`, and `oci`, users often know the intent but not the exact command sequence.

## Target Experience

As the user types in the shell, `2cp` should surface a compact, keyboard-friendly dropdown of likely continuations.

Examples:

- `git ` suggests common subcommands
- `git ch` suggests `checkout` and `cherry-pick`
- `git checkout ` can later suggest branches and common flags
- `kubectl ` can suggest high-value verbs and resource-oriented continuations
- `kubectl describe pod ` can search live pod names and offer matching objects in real time
- `aws s3 ` can surface common subcommands and flags

## MVP

The first version should:

- support `zsh`
- prove the product on `git`
- be designed so additional CLIs can be added without changing the shell bridge
- rank command and subcommand suggestions sensibly
- install with minimal friction
- feel fast enough to use continuously while typing

## Non-Goals

- replacing the shell
- comprehensive support for every CLI on day one
- remote inference or hosted dependencies
- deep command history personalization in the first pass

## Product Risks

- shell hooks may limit how rich the interaction can be
- terminal redraw behavior can become fragile across environments
- contextual suggestions can become expensive if not cached carefully
- large CLI surfaces such as `aws` and `kubectl` can become noisy without strong ranking and scoping

## Decision

The project is Rust-first. Future work should optimize for:

- low startup time
- predictable interactive latency
- single-binary installation where practical
- explicit shell integration boundaries
- an extensible provider model for multiple CLI ecosystems
