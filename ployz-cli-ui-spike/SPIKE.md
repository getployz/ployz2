# CLI presentation spike (throwaway)

**Do not merge.** Answers whether these crates can be Ployz’s text adapter.

## Run

```bash
cargo run -p ployz-cli-ui-spike -- catalog --look iocraft
cargo run -p ployz-cli-ui-spike -- catalog --look classic
cargo run -p ployz-cli-ui-spike -- catalog --look compact
cargo run -p ployz-cli-ui-spike -- catalog -o json
cargo run -p ployz-cli-ui-spike -- catalog --look compact | cat
cargo run -p ployz-cli-ui-spike -- deploy --look iocraft --yes
cargo run -p ployz-cli-ui-spike -- deploy --look classic --yes
cargo run -p ployz-cli-ui-spike -- error --look classic
```

`--look` is the variant switcher: **iocraft** (Ink boxes), **classic** (tabled + cliclack + anstream), **compact** (pipe/log tables). `-o json` ignores look.

## Question

Can one typed View (list / plan / error) feed:

1. iocraft on a TTY
2. tabled + cliclack + anstream on a TTY
3. compact text when piped
4. serde JSON

without Session or napi depending on any of those crates?

## Crates in the mix

| Role | Crate | Verdict |
| --- | --- | --- |
| TTY static layout | iocraft 0.8.4 | **Maybe later**, not the first list adapter |
| Tables | tabled 0.20 | **Yes** |
| Confirm / deploy chrome | cliclack 0.5.6 | **Yes for confirm** |
| Color vs pipe | anstream 1 + clap `--color` | **Yes** |
| Color clap mixin | colorchoice-clap | **Skip** — clap 4 already has `--color` |
| Errors | miette 7 fancy | **Yes**, with a Ployz reporter |
| JSON | serde_json | **Yes** |

Skipped: Ratatui, inquire, indicatif (cliclack already pulls it).

## Verdict

**Yes, one View / two adapters is the right shape.** The spike crate never touches Session. JSON is the same DTOs. Text is look-specific.

**First real slice:** `tabled` compact/rounded tables + `anstream` + clap `--color` + `miette` + `-o json` on one list command (`ployz images` already has JSON). Not iocraft.

**iocraft** is a good Ink-style *layout* crate:

- `write_to_is_terminal` / `to_string` print a boxed `ls` and image cards to a pipe **without ANSI**. That is the log-friendly claim, and it holds.
- `render_loop` is a live TUI. When the canvas is as tall as the terminal, iocraft **clears the screen and scrollback** (`ClearType::All` + `Purge`, issue 118). That is a bad default for `ployz deploy`.
- `render_loop` wants `'static` trees. Borrowed View data does not compile; you clone or leak into the component.
- Color does **not** honor clap `--color always` on a pipe. iocraft keys off `IsTerminal` only.
- tokio works. smol is not required.

Use iocraft later for an interactive surface (confirm screen, dashboard). Do not make it the `ployz ls` renderer.

**tabled** is the list crate. Compact `Style::empty()` is the pipe look. Rounded boxes are the TTY look. Wide `images` columns fit; iocraft cards were prettier for that row but worse for grep.

**cliclack** matches a Clack confirm/plan (`◇ plan`, intro/outro). Do not use it for tables. Do not mix its raw mode with iocraft `render_loop` in one process.

**anstream** headings respected `--color always` on a pipe. **colorchoice-clap** is a 3-arm map you can write against clap’s `ColorChoice`.

**miette** codes + help are what we want (`ployz::cli::ambiguous_service`). `#[related]` printed as three stacked `Error:` blocks. Keep miette; wrap related matches in one diagnostic.

## Captures on this branch

Pipe and TTY runs of `catalog`, `ls -o json`, `error`, and `deploy --yes --instant` for each look. See the PR artifacts.
