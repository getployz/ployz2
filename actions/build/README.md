# getployz/build

Builds one Ployz Image Build on a GitHub Actions runner and pushes the image into the Machine that Ployz Cloud chose. Cloud starts it; nothing runs on push.

## Set up

Commit [`ployz-build.yml`](ployz-build.yml) to the default branch as `.github/workflows/ployz-build.yml`. Ployz Cloud's **Add workflow** button opens GitHub's new-file page with it filled in. Keep it exactly as is: Cloud dispatches its inputs.

The GitHub App needs **Actions: write** and **Contents: read**, plus the **Push** and **Workflow run** events.

## Dispatch inputs

| Input | Set by Cloud to |
| --- | --- |
| `build` | Image Build id (`[A-Za-z0-9_-]`, up to 128 characters) |
| `cloud` | Cloud origin, for example `https://ployz.dev`. It is also the OIDC audience. |
| `ployz_version` | Cloud's own SDK version, pinned exactly so the build fingerprint matches |
| `runner` | The native runner for the one platform the Service needs: `ubuntu-latest` (amd64) or `ubuntu-24.04-arm` (arm64) |

Inputs are visible in GitHub, so none of them is secret.

## What it does

1. Exports the Actions cache runtime (`crazy-max/ghaction-github-runtime`), so `ployz build` uses the GitHub Actions cache.
2. Turns on Docker's containerd image store if it is off. This restarts Docker and needs `sudo`.
3. Installs exactly `ployz_version` from `https://ployz.sh`. The installer verifies the release checksum.
4. Gets a GitHub OIDC token with audience `cloud` and checks in: `POST {cloud}/api/builds/{build}/check-in` with `Authorization: Bearer <token>`. Cloud answers:
   ```json
   {"grant": "ployzgrant1:…", "commit": "<40 hex>", "fingerprint": "<64 hex>", "deployment": { "snapshots": [{ "resolvedEnv": {} }] }}
   ```
   The token, the grant, and every `resolvedEnv` value are masked (`::add-mask::`, per line) before anything else runs.
5. Checks out `commit` without persisting credentials.
6. Runs `PLOYZ_BUILD_GRANT=… ployz build --deployment <file> --commit <commit> --fingerprint <fingerprint> --events <file>`. The deployment file lives in `$RUNNER_TEMP` and is deleted when the job ends, pass or fail.
7. Reports the Build Steps while it builds, every few seconds, each time with a fresh OIDC token: `POST {cloud}/api/builds/{build}/steps` with `{"from": <line>, "events": [<new ployz build --events lines>]}`, where `from` is the 0-based line the batch starts at. Cloud files only lines it hasn't taken, so a retried batch is harmless. When the build ends, pass or fail, the last batch adds `"platforms": [...]`; empty means the build failed, and Cloud takes no more.

Output `digest` is the manifest digest the Machine received. Cloud does not trust it: it reads the pushed digest from the Machine when it ends the grant.

## Runner needs

Linux, Docker with Buildx, rootful Docker on the runner's network (the push goes to `127.0.0.1`), and outbound access to `relay.ployz.dev`. GitHub-hosted Ubuntu runners have all of these.

## Develop

`./test.sh` runs both scripts against stubbed `curl`, `docker`, and `ployz`. `shellcheck *.sh` must pass. `oidc.sh` holds the OIDC token helper both scripts source.

## Publish

This directory is the source of `getployz/build`. Copy it to the root of that repository, then tag the release and move the major tag:

```sh
git tag v1.0.0 && git tag -f v1 && git push origin v1.0.0 && git push -f origin v1
```
