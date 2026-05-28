# Oino Footer Status

Oino Footer Status is an optional built-in package. When installed and enabled, it contributes two host-rendered UI surfaces:

- `footer_status_top` — a one-line surface intended to render directly above the composer input.
- `footer_status_bottom` — a one-line surface intended to render directly below the composer input.

The host runtime owns the live values and renders the package only when its surfaces are active:

- top line: selected model name and selected thinking level
- bottom line: working directory and approximate context usage as percentage plus `used / context-length`

Context usage is based on Oino's inspect/full-prompt token estimate. Context length comes from the selected model catalog when available; if the provider does not report a limit, the line shows `unknown` rather than inventing a value.

## Install during development

Open `/extensions`, press `i` for a project install or `I` for a global install, and enter:

```text
builtin:footer-status
```

During development, the package can also be installed directly from its repository path:

```text
extensions/built-in/footer-status
```

After install, use the `/extensions` project/global toggles to enable or disable the package, extension, or individual footer contributions.

## Safety

This package declares only UI permissions. It has no tools, shell access, filesystem writes, network access, secrets, package-management rights, or persistence permissions.
