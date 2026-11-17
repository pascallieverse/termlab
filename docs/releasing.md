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

### Self-signed (current)

Since 0.2.3 the macOS app is signed with Termlab's own self-signed certificate,
"Termlab Code Signing" (valid until 2046). macOS keys an app's permissions (Accessibility,
Screen Recording, …) to its signature: an ad hoc signature is the build's hash, so every
update looked like a new app and lost them; with the certificate they carry over. First
installs still need "Open anyway" in Privacy & Security, as with ad hoc signing.

- The certificate and its key: `~/.tauri/macos-signing/termlab-signing.p12` on the
  maintainer's Mac; its password is in the macOS Keychain as "Termlab code signing
  certificate".
- Repository secrets: `MAC_SELF_SIGNED_CERTIFICATE` (the `.p12`, base64:
  `base64 -i termlab-signing.p12`) and `MAC_SELF_SIGNED_CERTIFICATE_PASSWORD`.
- The workflow imports it into a temporary keychain on the runner, trusts it for code
  signing there (`codesign` refuses an untrusted certificate) and passes it to Tauri as
  `APPLE_SIGNING_IDENTITY`. A check after the build fails the run if the app's designated
  requirement isn't the certificate.

Keep the `.p12` and its password backed up. A new certificate means one more round of
granting permissions again after that update.

### Developer ID

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

The workflow signs with the Developer ID when `APPLE_CERTIFICATE` is set, else with the
self-signed certificate when that is set, and otherwise falls back to ad hoc.

## Updates from inside the app

Termlab looks for a newer release ten seconds after it starts and every six hours after
that. When there is one, a notice appears under the header with **Restart to update**;
Settings → About has **Check for updates**. Nothing installs until that button is clicked.
Installing downloads the package, checks its signature, replaces the app and starts it
again. Sessions keep running (on Windows the installer has to stop the background service;
resumable sessions reopen).

This is switched on since 0.2.2. The update signing key was set up once (2026-11-17):

1. The key was created with
   `cd apps/desktop && npm run tauri signer generate -- -w ~/.tauri/termlab.key`. The
   private key is `~/.tauri/termlab.key` on the maintainer's Mac; its password is in the
   macOS Keychain as "Termlab update signing key".
2. Its public key (`~/.tauri/termlab.key.pub`) is `pubkey` in
   `apps/desktop/src-tauri/updater.conf.json`.
3. Two repository secrets hold the private side: `TAURI_SIGNING_PRIVATE_KEY` (the contents
   of `~/.tauri/termlab.key`) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Keep the key file and its password backed up somewhere safe. Installed copies only accept
updates signed with it: if it is lost, they can't be updated any more and need a manual
reinstall. To replace the key, repeat the three steps; every installed copy then needs one
manual install of a build with the new public key.

With the secrets set, the workflow adds `updater.conf.json` to the build, signs the update
packages and uploads `latest.json`, which is what installed copies read. A release only
reaches them once its draft is published. Builds from before 0.2.2, and development
builds, say "This build doesn't update itself": install 0.2.2 or later by hand once.

## Not there yet

- **The update flow has not been run end to end.** The notice and buttons were checked
  against mocked data; a real download and install needs two published releases signed
  with the key (0.2.2 and the one after it).
- **Windows code signing.**
- GitHub's Apple Silicon runners are sometimes short: a macOS build can be cancelled with
  "The job was not acquired by Runner". Re-run the failed job
  (`gh run rerun <run id> --failed`).
