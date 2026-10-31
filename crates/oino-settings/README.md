<img src="https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-128.png" alt="oino" width="64">

# oino-settings

Settings manager for Oino: global/project layering, migrations, typed getters, locking.

* `SettingsManager` — trust gating, reload, scoped writes, typed setters
* `Settings` — typed getters with defaults and layer access
* `paths` — agent dir (`~/.oino`), project dir (`<cwd>/.oino`), environment overrides
* `storage` — locked, atomic file storage
* `secrets` — `<agent>/secrets.json` (0600, locked), extension secrets
