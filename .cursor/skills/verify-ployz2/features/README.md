# Feature map

Preconditions for every drive:

1. Binaries from this checkout (`target/debug/ployz`, `target/debug/ployzd`), same version string.
2. Isolated instance via `helpers/launch.sh <instance>` unless the feature is CLI-only and you already have those binaries.
3. `--ployz-config` pointing at the instance config. Never `~/.config/ployz/config.yaml`.
4. Ambient `PLOYZ_CONFIG` / `PLOYZ_CONNECT` / `PLOYZ_CONTEXT` unset (helpers do this).

Driving conventions:

- Prefer `helpers/drive.sh` so transcripts land in `/opt/cursor/artifacts/verify-ployz2/<instance>/`.
- Stable handles are clap command names, prompt strings, and table headers in each feature file.
- Interactive prompts need a TTY (tmux). Non-interactive paths: pass the name/connection argument, or `--yes` / `PLOYZ_AUTO_CONFIRM` where documented.

Proof / skip:

- **Proved:** user command, captured stdout/stderr/exit, and a side effect (file, table row, or daemon log) that matches the feature file.
- **Skipped:** name the missing precondition (participating Machine, Docker, SSH destination, nested Docker for testkit). Do not fake Cluster output from an uninitialized daemon.

## Features

| File | User surface | Isolated uninitialized `ployzd` |
| --- | --- | --- |
| [version.md](version.md) | `ployz version` / `--version` | Not required |
| [contexts.md](contexts.md) | `ployz ctx` | Not required (seeded config stands in for `machine init`) |
| [machine-init.md](machine-init.md) | `ployz machine init` | Local destination is a stub; remote Linux Docker host required to found a Cluster |
| [machine-ls.md](machine-ls.md) | `ployz machine ls` | RPC errors until the Machine participates |
| [deploy.md](deploy.md) | `ployz deploy` | Needs a participating Cluster plus Compose |

## Feature entry contract

Each file: one H1, one paragraph of user-visible behavior, then exactly these H2s in order: `Sub-features`, `How to get to it (user POV)`, `Driving it with drive.sh`, `Gotchas`. No implementation internals. Name user paths, clap handles, required state, commands, and observable proof.
