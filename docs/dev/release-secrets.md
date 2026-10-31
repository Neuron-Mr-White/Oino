# Release secrets (Android signing)

The release workflow (`.github/workflows/release.yml`, job `app-android`) signs
the apk from repository secrets. Without them it signs with a **generated debug
key** and names the artifact `UniPi_<version>_universal-unsigned-debug.apk` —
installable for testing, but phones that already trust the release key will
refuse it as an *update* (different signer), and Play-style distribution is not
possible.

## Create a release keystore (once)

```sh
keytool -genkeypair -v \
  -keystore unipi-release.keystore \
  -alias unipi \
  -keyalg RSA -keysize 4096 -validity 10000
```

Keep the keystore file and both passwords somewhere safe **outside the repo**
(the owner's copy lives under the credentials dir; it is never committed). If
the keystore is ever lost, a new one means a new app identity for updates.

Record the certificate fingerprint for verification:

```sh
keytool -list -v -keystore unipi-release.keystore -alias unipi | grep SHA256
```

## Add the GitHub secrets

Repo → Settings → Secrets and variables → **Actions** → New repository secret:

| Secret | Value |
|---|---|
| `ANDROID_KEYSTORE_B64` | the keystore, base64: `base64 -w0 unipi-release.keystore` |
| `ANDROID_KEYSTORE_PASSWORD` | the keystore password |
| `ANDROID_KEY_ALIAS` | `unipi` (the `-alias` above) |
| `ANDROID_KEY_PASSWORD` | the key password |

The workflow decodes `ANDROID_KEYSTORE_B64` and feeds the other three to the
gradle build as `UNIPI_ANDROID_KEYSTORE*` env (see
`apps/unipi/src-tauri/gen/android/app/build.gradle.kts` — the signing config
appears only when `UNIPI_ANDROID_KEYSTORE` is set).

## Verify

After a release run, check the signer of the published apk:

```sh
"$ANDROID_HOME"/build-tools/*/apksigner verify --print-certs UniPi_<version>_universal.apk
```

The SHA-256 must match the fingerprint recorded above. The debug-key fallback
prints a `::notice::` in the workflow run and suffixed the artifact name with
`-unsigned-debug`.
