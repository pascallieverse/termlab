# Releasing Termlab

Releases are built by GitHub Actions (`.github/workflows/release.yml`) and published as
GitHub releases.

## Cutting a release

1. Set the new version in three files: `Cargo.toml` (`[workspace.package]`),
   `apps/desktop/package.json` and `apps/desktop/src-tauri/tauri.conf.json`.
   `node apps/desktop/scripts/check-version.mjs` checks that they agree.
2. Commit, then tag and push: `git tag v0.2.0 && git push origin main v0.2.0`.
3. The workflow creates a **draft** release and builds into it:
   - macOS, Apple Silicon: `.dmg`
   - Linux, x86-64: `.deb`, `.rpm`
   - Windows, x86-64: `-setup.exe`
4. Check the draft on GitHub, then publish it. Nothing is public before that.
5. Publishing starts `.github/workflows/screenshots.yml`: it retakes the README screenshots
   from the mocked UI, commits them to `docs/screenshots/` on the default branch and attaches
   them to the release. If the default branch is protected against pushes from GitHub
   Actions, that commit fails: allow it, or run `npm run screenshots` and commit them yourself.

To build the same package locally: `cd apps/desktop && npm run bundle`.

## Signing

Without any secrets the workflow still builds everything:

- macOS is signed ad hoc. The app only opens after the user allows it in
  System Settings → Privacy & Security.
- Windows is unsigned. SmartScreen warns before the installer runs.

For a signed and notarized macOS build, add these repository secrets
(Settings → Secrets and variables → Actions):

| Secret | What it is |
|---|---|
| `APPLE_CERTIFICATE` | The Developer ID Application certificate with its private key, exported from Keychain Access as `.p12`, then base64: `openssl base64 -A -in certificate.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | The password chosen for that export |
| `APPLE_SIGNING_IDENTITY` | The certificate's name, e.g. `Developer ID Application: Company (TEAMID)` |
| `APPLE_API_KEY` | Key ID of an App Store Connect API key (for notarization) |
| `APPLE_API_ISSUER` | That key's issuer ID |
| `APPLE_API_PRIVATE_KEY` | The contents of the key's `.p8` file |

The workflow signs when `APPLE_CERTIFICATE` is set and otherwise falls back to ad hoc.

## Updates from inside the app

Termlab looks for a newer release ten seconds after it starts and every six hours after
that. When there is one, a notice appears under the header with **Restart to update**;
Settings → About has **Check for updates**. Nothing installs until that button is clicked.
Installing downloads the package, checks its signature, replaces the app and starts it
again. Sessions keep running (on Windows the installer has to stop the background service;
resumable sessions reopen).

This is switched off until the update signing key exists. To switch it on, once:

1. Create the key. It asks for a password:
   `cd apps/desktop && npm run tauri signer generate -- -w ~/.tauri/termlab.key`
2. Put the public key it prints (also in `~/.tauri/termlab.key.pub`) into `pubkey` in
   `apps/desktop/src-tauri/updater.conf.json`, and commit that.
3. Add two repository secrets: `TAURI_SIGNING_PRIVATE_KEY` (the contents of
   `~/.tauri/termlab.key`) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Keep the key file and its password somewhere safe. Installed copies only accept updates
signed with it: if it is lost, they can't be updated any more and need a manual reinstall.

From the next release on, the workflow adds `updater.conf.json` to the build, signs the
update packages and uploads `latest.json`, which is what installed copies read. A release
only reaches them once its draft is published. Builds made before the key existed, and
development builds, say "This build doesn't update itself".

## Not there yet

- **The update flow has not been run end to end.** The notice and buttons were checked
  against mocked data; a real download and install needs two published releases.
- **Windows code signing.**
- The workflows have not run yet. The Linux and Windows builds in particular are untested.
