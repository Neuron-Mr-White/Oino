# Release update feeds

Every shipped binary updates itself from one of three feeds. A `v*` tag build
of `.github/workflows/release.yml` produces all three; two of them are then
published on oino.dev by the site deploy.

| Binary | Mechanism | Feed | Verification |
|---|---|---|---|
| Desktop AppImage / NSIS `.exe` / macOS `.app` | Tauri updater plugin (`src-tauri/src/app_update.rs`) | `https://oino.dev/app/desktop/latest.json` | minisign signature against `plugins.updater.pubkey` (key id `9A572EBF36FD9DA0`) |
| Desktop `.deb` | none in-app (needs root); the app links `https://oino.dev/download/` | same feed, "manual" kind | apt / SHA256SUMS |
| Android APK | `src/lib/update.ts` + `UpdateBridge.kt` | `https://oino.dev/app/android/latest.json` | APK host must be `oino.dev`, sha256 from the feed, then Android checks the APK is signed by the installed app's certificate |
| `unipi-oino-host` | `unipi-oino-host update` / `serve` check | `https://github.com/Neuron-Mr-White/Oino/releases/latest/download/release-manifest.json` | URL must be this repo's Releases, sha256, new binary's `--version` |
| `oino` CLI | `oino update` | same `release-manifest.json` | sha256 |

## What the release workflow does

- Desktop jobs merge `bundle.createUpdaterArtifacts: true` into the release
  config and sign with the `TAURI_SIGNING_PRIVATE_KEY` /
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets. The `.sig` files are uploaded
  as `UniPi_<v>_amd64.AppImage.sig`, `UniPi_<v>_x64-setup.exe.sig` and
  `UniPi_<v>_universal.app.tar.gz.sig`.
- The Android job signs with `ANDROID_KEYSTORE_B64`, `ANDROID_KEYSTORE_PASSWORD`,
  `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD`, then checks the APK
  certificate's SHA-256 against `RELEASE_CERT_SHA256`.
- A tag build **fails** if any of those secrets is missing. Only a
  `workflow_dispatch` rehearsal falls back to unsigned updater artifacts and a
  `-unsigned-debug` APK.
- `release-manifest.json` lists the bare binaries (`oino-<v>-<target>.bin|.exe`,
  `unipi-oino-host-<v>-<target>.bin|.exe`), because both updaters write the
  downloaded bytes over their own executable. The Linux host binary is static
  musl, so it is listed under both the `-musl` and the `-gnu` triple.
- `scripts/release/make-feeds.sh` builds both app feeds. They are attached to
  the release as `app-desktop-latest.json` and `app-android-latest.json`.
  On a tag it is strict: every desktop platform must be present and its
  signature must verify against the pubkey in `tauri.conf.json`.

## Site deploy step (after each release)

Copy three files from the GitHub release to the oino.dev docroot:

```sh
tag=v1.2.3; v=${tag#v}
gh release download "$tag" -R Neuron-Mr-White/Oino \
  -p app-desktop-latest.json -p app-android-latest.json -p "UniPi_${v}_universal.apk"
# -> /app/desktop/latest.json
# -> /app/android/latest.json
# -> /download/UniPi_${v}_universal.apk   (the Android feed's url)
```

To rebuild the feeds from any published release:
`scripts/release/make-feeds.sh --tag "$tag" --out <dir>`.

Then check that all four URLs return 200: both `latest.json` files, the APK
URL inside the Android feed, and `https://oino.dev/download/`. Serve the JSON
with `Cache-Control: no-cache` so clients see a new release quickly.

## Keys

The Android release keystore (alias `unipi-release`, certificate SHA-256
`23:B1:42:E7:9D:A5:47:82:32:00:4A:60:99:BF:A0:1F:41:F7:A8:91:C9:44:14:8B:D7:83:23:88:9C:3E:BB:48`)
and the Tauri updater private key are kept offline by the maintainer, with an
encrypted backup, and stored as the repo secrets above. If either is lost,
installed apps can no longer update: Android refuses an APK signed with a
different certificate, and the desktop updater refuses a signature from a
different key.
