# asvc packaging model

`asvc` has two mutually exclusive installation modes. Desktop is the CLI mode plus a GUI; users
should not keep a second standalone installation in the same `PATH`:

| Product | Source | Includes | Entry point |
| --- | --- | --- | --- |
| Headless CLI | root `Cargo.toml` | CLI + daemon only | `asvc` |
| Desktop | `src-tauri/` + `desktop/` | Tauri UI + the same-version CLI sidecar | Asvc application and `asvc` |

The root binary never opens a GUI and has no Tauri dependency. This keeps the native CLI small and
usable in agent, CI, server, and container environments. The desktop crate depends on the root
crate, bundles the exact same CLI build, and owns the GUI process; both processes use the same
daemon protocol and state directory.

## Build flow

1. Build the root crate as the headless CLI (`cargo build --release --locked`).
2. `scripts/prepare-desktop.mjs` copies that exact binary to
   `src-tauri/binaries/asvc-<target-triple>`.
3. Tauri's `externalBin` bundles the sidecar next to the desktop executable. The sidecar is the
   user-facing `asvc` command; it is not a second implementation.
4. `scripts/package-desktop.mjs` collects installers and writes a per-platform checksum file.

The sidecar is deliberately built before Tauri, so the desktop UI and CLI cannot drift to different
versions. `scripts/check-release-version.mjs` checks the root crate, desktop crate, Tauri config,
desktop frontend, and npm packages together.

## Installed command

- macOS `.dmg`: contains `Asvc.app` and its same-version CLI sidecar. On startup the app resolves the
  user's login-shell `PATH`, compares the selected `asvc` version, and gates daemon access until the
  bundled version is installed. Missing or outdated commands require an explicit in-app confirmation;
  protected destinations then use the macOS authorization dialog. Homebrew/npm commands are removed
  through their package manager before Desktop takes ownership of the resolved path. Multiple PATH
  candidates are reported as a conflict rather than overwritten. The app remains ad-hoc signed, so
  macOS may still require a one-time approval in Privacy & Security after a browser download.
- Linux `.deb`: Tauri places the sidecar beside the application executable, so it is available as
  `/usr/bin/asvc`.
- Windows NSIS: installs the sidecar beside the app and adds the current-user application directory
  to `PATH` through `src-tauri/windows/hooks.nsh`.

Portable AppImage output is intentionally not published because it cannot own the user's CLI path;
Linux Desktop is distributed as `.deb`, while AppImage users should choose the CLI-only mode.

## Desktop updates

macOS and Windows use Tauri's signed updater. After the startup compatibility checks finish, Desktop
checks `latest.json` from the latest GitHub Release and offers a standard Later / Update and restart
prompt with release notes and download progress. Linux `.deb` updates remain owned by the system
package manager.

`src-tauri/tauri.release.conf.json` enables `bundle.createUpdaterArtifacts` to produce the platform
updater payload and `.sig` file without requiring a release key for ordinary local builds. The release
workflow expects `TAURI_SIGNING_PRIVATE_KEY` (and, when applicable,
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) as GitHub Actions secrets, copies the signed payloads, and runs
`scripts/generate-updater-manifest.mjs` to create the published `latest.json`. Never commit or lose the
private key: clients embed the matching public key and reject payloads signed by anything else.

The macOS DMG does not modify the host shell's `PATH` during drag-and-drop. The first application
launch performs the visible, confirmed CLI migration because a dragged `.app` has no installer hook.
Later launches are silent while the resolved CLI version matches the bundled version.

The daemon ping includes its package version. Before loading services, Desktop gates a mismatched or
legacy daemon behind a separate confirmation. Migration records only running/starting services,
stops the old daemon, starts the current Desktop executable in daemon mode, and restores that set;
intentionally stopped services remain stopped.

## Release outputs

The tag workflow publishes both families. Headless assets retain the existing names such as
`asvc-v<version>-linux-x64.tar.gz`; desktop assets are named
`asvc-desktop-v<version>-<platform>-...`. Both are covered by the final `SHA256SUMS` asset.

For a local build, use `npm run package:headless` for the CLI-only artifact or
`npm run desktop:build` for the desktop bundle. The latter requires `npm install --prefix desktop`.
