# Coding standards

What rustfmt and the workspace Clippy lints in `Cargo.toml` do not catch. Those remain the source of truth for format and lint. New Clippy suppressions are `#[expect(clippy::lint)]` with a why, not `#[allow]`.

## Names

Accessors omit `get_`: `name()`, `as_str()`. Wire RPC method names that already exist on the contract (`get_caddy_config`) stay as they are.

Conversion prefixes:

- `as_` — cheap borrow
- `to_` — allocates or copies
- `into_` — consumes `self`

Iterators are `iter()` / `iter_mut()` / `into_iter()`.

## Types

A `CONTEXT.md` term is a newtype, not `String` or a primitive. `MachineId` and `MachineName` are different types.

Fallible construction is `parse` → `Result`. Cheap views are `as_str`. Shadow the binding across a transform: `let value = value.parse()?`.

Name a lifetime for the borrow (`'store`, `'src`) when the role is known.

Parameters borrow: `&str`, `&[T]`, `&T`. Clone only when the callee must own. Small `Copy` values pass by value.

## Errors

Library and daemon errors are `thiserror` types. The CLI maps those into `Failure`.

Recoverable failures use `?`. Early return without the error value is `let Ok(x) = ... else { return ... }`. `expect("why this is a programmer bug")` is for invariants. `unwrap` belongs in tests.

## Async

Async is for I/O. CPU-bound work stays sync.

Drop a `std::sync` lock before `.await`. Use `tokio::sync::Mutex` when the guard must live across an await.

## Dispatch

Generics (`T: Trait` / `impl Trait`) until a mixed collection or object-safe trait needs `dyn`. Own the `Box`/`Arc` at that boundary.

## Macros

Reuse the existing newtype and RPC macros for repeated mechanical shapes. A one-off stays a function or generic.

## Docs

`//!` on crate and module files. `///` on public items. `//` explains why (workaround, design), not what the next line does. TODOs carry an issue id: `// TODO(UT-123):`.
