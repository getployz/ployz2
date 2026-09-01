# Contexts

`ployz ctx` (alias `context`) manages the local Ployz config: which Cluster context is current, and which connection of that context is the default. It does not call a Machine. Contexts are created by `machine init`, `machine add`, and `cloud enroll`; this feature drives the later list/select/show path.

## Sub-features

- `ctx ls` (`list`): table `NAME	CURRENT	CONNECTIONS`, current row marked `*`. Empty file or missing config prints `No contexts found` and exits 0.
- `ctx show`: prints the current context name (blank stdout when there are no contexts).
- `ctx use [context-name]`: with a name, persists `current_context` and prints `Current context is now "<name>".`. Without a name, interactive `Select a context:` then `  N. <name>` (current tagged ` (current)`) and prompt `> `.
- `ctx connection` (alias `conn`): with no argument, prints the default connection URI (works without a TTY). With a URI that already exists on the current context, rotates it to first and prints `Default connection for context "<name>" is now "<uri>".`.
- `--ployz-config` / `PLOYZ_CONFIG` select the file; the flag wins over the env. Default file is `~/.config/ployz/config.yaml`.

## How to get to it (user POV)

After a successful `ployz machine init USER@HOST -c prod` the user has a named context. Day to day they run `ployz ctx ls`, `ployz ctx use dev`, `ployz ctx show`, and `ployz ctx connection` (or pick a `unix://` / `ssh://` / `tcp://` URI). `--connect` is incompatible with context management.

On an isolated verifier, `helpers/seed-contexts.sh` writes the same yaml shape `machine init` would persist (two contexts, two connections on `prod`) so `ctx` can be driven without SSH.

## Driving it with drive.sh

```sh
helpers/launch.sh proof
helpers/doctor.sh proof
helpers/drive.sh proof ctx ls          # empty: "No contexts found"
helpers/seed-contexts.sh proof
helpers/drive.sh proof ctx ls
helpers/drive.sh proof ctx use dev
helpers/drive.sh proof ctx show
helpers/drive.sh proof ctx connection
helpers/drive.sh proof ctx connection "unix://$SOCKET"
```

Proof:

- After seed, `ctx ls` stdout contains `dev` and `prod` and the header `NAME	CURRENT	CONNECTIONS`; `prod` is current (`*`).
- `ctx use dev` stdout is `Current context is now "dev".` and `*-config-after.yaml` has `current_context: dev`.
- `ctx show` stdout is `dev`.
- `ctx connection` stdout is `unix://` plus this instance's socket.
- Selecting the second `prod` connection (after `ctx use prod`) prints the `Default connection for context "prod" is now ...` line and reorders `connections:` in the yaml.

Expected failures (still evidence): `ctx use` with no name and no TTY → `cannot Select a context interactively without a terminal`; `ctx connection unix:///tmp/missing.sock` → `connection "unix:///tmp/missing.sock" not found` and config unchanged; `ctx` with `--connect` → `context management is unavailable with a direct connection`.

Interactive `ctx use` (optional): tmux session, wait for `Select a context:`, send `1` then Enter, expect `Current context is now`.

## Gotchas

- `ctx` never creates a context. Empty config is not an error for `ls`/`show`; `use` on an empty config errors `no contexts found in Ployz config <path>`.
- Connection URIs must match the stored spelling exactly (`unix://` requires an absolute path).
- Seeded yaml is a stand-in for `machine init`. Do not claim a Cluster exists because `ctx ls` listed names.
