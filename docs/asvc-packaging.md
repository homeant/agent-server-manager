# asvc packaging model

`asvc` has two independent distribution products:

| Product | Source | Includes | Entry point |
| --- | --- | --- | --- |
| Headless CLI | root `Cargo.toml` | CLI + daemon only | `asvc` |
| Desktop | `src-tauri/` + `desktop/` | Tauri UI + the same-version CLI sidecar | Asvc application and `asvc` |

The root binary never opens a GUI and has no Tauri dependency. This keeps the native CLI small and
usable in agent, CI, server, and container environments. The desktop crate depends on the root
crate and owns the GUI process; both processes use the same daemon protocol and state directory.

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

- macOS `.dmg`: contains `Asvc.app` and its same-version CLI sidecar. After dragging the app to
  `Applications`, the Settings screen can explicitly install `/usr/local/bin/asvc`; macOS asks for
  administrator authorization only when the destination is not writable.
- Linux `.deb`: Tauri places the sidecar beside the application executable, so it is available as
  `/usr/bin/asvc`.
- Windows NSIS: installs the sidecar beside the app and adds the current-user application directory
  to `PATH` through `src-tauri/windows/hooks.nsh`.

The macOS DMG does not modify the host shell's `PATH` during drag-and-drop. The explicit Settings
action makes that change visible and reversible by replacing only `/usr/local/bin/asvc` on the next
install/update.

## Release outputs

The tag workflow publishes both families. Headless assets retain the existing names such as
`asvc-v<version>-linux-x64.tar.gz`; desktop assets are named
`asvc-desktop-v<version>-<platform>-...`. Both are covered by the final `SHA256SUMS` asset.

For a local build, use `npm run package:headless` for the CLI-only artifact or
`npm run desktop:build` for the desktop bundle. The latter requires `npm install --prefix desktop`.
