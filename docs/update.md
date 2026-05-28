# Oino install and update releases

Oino updates are based on GitHub tags and release assets. `main` raw file URLs are only bootstrap conveniences for installer scripts; the installer and `oino update` resolve tagged release metadata before installing core binaries.

## Release manifest

Each release should upload `release-manifest.json` next to binary assets:

```json
{
  "tag": "v0.1.0",
  "version": "0.1.0",
  "generated_at": "2026-05-28T00:00:00Z",
  "artifacts": [
    {
      "target": "x86_64-unknown-linux-gnu",
      "kind": "binary",
      "url": "https://github.com/Neuron-Mr-White/Oino/releases/download/v0.1.0/oino-x86_64-unknown-linux-gnu",
      "sha256": "...",
      "size": 12345678
    }
  ],
  "source": {
    "url": "https://github.com/Neuron-Mr-White/Oino/archive/refs/tags/v0.1.0.tar.gz",
    "sha256": "..."
  },
  "extensions": {
    "url": "https://github.com/Neuron-Mr-White/Oino/releases/download/v0.1.0/oino-extension-bundles.tar.gz",
    "sha256": "...",
    "built_in_root": "extensions/built-in",
    "additional_root": "extensions/additional"
  }
}
```

Artifact `target` values follow Rust target-like names such as:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

`kind: "binary"` means the asset bytes are the executable itself. The in-app updater intentionally only hot-installs direct binary artifacts; installer scripts may support archives and source fallbacks.

## Core update behavior

- `oino update check` and `/update check` fetch the release manifest and report whether a matching binary exists.
- `oino update` and `/update` download the matching binary, verify `sha256` when present, atomically replace the current executable on Unix-like systems, and ask the user to restart Oino.
- `oino update --source` reports the source/cargo fallback command instead of replacing the running binary.
- On Windows, updating the running executable is not supported because the binary is usually locked; use the installer script after closing Oino.

## Extension update behavior

Extension packages are hot-updateable. `oino update extensions`, `/update extensions`, and the existing `/extensions update` reuse the package lifecycle update path and reload Oino resources/contributions afterward.

## Extension layout

Repository-shipped optional built-ins live in:

```text
extensions/built-in/
```

Separately distributed optional extensions live in:

```text
extensions/additional/
```

The extension SDK remains in `crates/oino-extension-sdk` and should not contain shipped package content.
