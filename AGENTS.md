# knapper development

## Obsidian and plugin compatibility

Read `spec.md` before changing Obsidian or plugin behavior. It is the canonical
feature contract and capability registry. Update the applicable entry in the
same change as implementation and tests, including input formats, source/plugin
versions, missing or malformed input, known gaps, and verification.

Distinguish native knapper adapters from the embedded Dataview adapter. Passing
upstream parser/expression tests does not establish Obsidian metadata parity.
Use synthetic committed fixtures and record actual-Obsidian comparisons with
explicit app/plugin versions. Use the isolated App + official CLI differential
workflow in `spec.md` for metadata-dependent checks; keep recorded-reference
CI replay distinct from a live desktop run. Keep private Vault/account data out of this repo.
Do not execute arbitrary installed plugins to implement data compatibility.

Build and verification requirements are in `.github/workflows/ci.yml`.
For changes to the embedded engine, also follow `dql/README.md` for deterministic
bundle regeneration, upstream tests and native-runtime comparisons. Documentation
changes alone do not require rebuilding unchanged code.
