# Oino Extension Community Registry Policy

Status: draft for the extension-kernel feature branch.

## Metadata contract

Community registry indexes are local/fixture JSON documents for now. A future hosted service must expose the same data shape: package id, publisher, version, display name, description, categories, license, source link, package path or artifact URL, included assets, Oino compatibility, dependencies, permissions, trust metadata, update policy, changelog, deprecation state, and security advisory references.

## Trust and review

- Packages are untrusted until reviewed by maintainers or a configured local trust source.
- Default publishing validation requires review and a package checksum.
- Signature metadata is supported and can be required by policy, but the v1 local fixture client does not define a hosted signing authority.
- Install flows surface requested permissions and trust metadata before writing files, and reload the extension manager after lifecycle operations.

## Checksums and signatures

- Local install/update verifies manifest trust checksums when present.
- Checksums are deterministic package-directory checksums that normalize the manifest trust checksum/signature fields out of the hash input.
- Signature strings are treated as trust-policy metadata in this branch; unreviewed signed packages are rejected by the package lifecycle service until a signing authority model is added.

## Advisories, deprecation, and takedown

- Registry indexes may include active or withdrawn advisories with low, moderate, high, or critical severity.
- Default publishing validation rejects packages with active high/critical advisories.
- Deprecated packages are rejected by the default trust policy and should include a migration/supersession note.
- Takedown in the local fixture model is represented by removing package metadata from the index and/or marking the package deprecated with an advisory; hosted registry takedowns must preserve audit history.

## Compatibility and publishing gates

Publishing validation checks Oino compatibility against the current host version, publisher presence, checksum/review/signature requirements, deprecation state, and active advisories. Warnings are emitted for empty descriptions/categories and any active advisory count.
