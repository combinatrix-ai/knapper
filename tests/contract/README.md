# The contract

`cases.yaml` describes what the `knapper` command line does, as data rather
than as code. Each case names a fixture vault, an argument list, and what the
result must look like.

It exists so the contract is not tied to the current implementation. These
cases were what made the Rust port checkable rather than hopeful -- both
implementations answered to this file, and every divergence between them
showed up here as a failing case rather than as a surprise in somebody's
vault. The Python implementation is gone; the cases outlived it, which is the
argument for writing them this way.

## Running

```sh
cargo test --test contract
```

The runner is `tests/contract.rs`. It builds the binary, copies each fixture
to a temporary directory, and runs the case against that copy.

## Case format

```yaml
- name: orphans lists notes nothing links to
  vault: obsidian              # a directory under tests/fixtures/
  args: [orphans, --format, json]
  expect:
    exit: 0                    # default 0
    json: ["Daily/2026-07-28.md"]   # exact match after normalisation
```

Assertions, all optional, all applied:

| key | meaning |
|---|---|
| `exit` | expected exit status, default `0` |
| `json` | stdout parsed as JSON and compared exactly |
| `json_contains` | every entry must appear in the parsed list |
| `json_excludes` | no entry may appear in the parsed list |
| `json_length` | length of the parsed list |
| `json_at` | `{pointer: value}`, checked against a dotted path into the JSON |
| `stdout_contains` | substrings that must be present |
| `stdout_excludes` | substrings that must be absent |
| `stderr_contains` | substrings that must be present on stderr |
| `stderr_excludes` | substrings that must be absent from stderr |
| `lines` | exact stdout lines, after stripping blanks |
| `file` | `{path: {contains: [...], excludes: [...]}}` checked after the run |
| `providers` | `{contains: [...], excludes: [...]}` against the local provider config |

Lists compared with `json` and `lines` are order-insensitive when the command
does not promise an order; those cases set `sorted: true`.

Each case runs against a **copy** of its fixture, so a case may mutate the
vault. `file:` assertions read from that copy.

## Providers

Every case also gets an `XDG_CONFIG_HOME` of its own, so no run can reach the
provider config of whoever is running the suite. A case that needs a provider
declares one — outside the vault, because that is the rule being tested:

```yaml
- name: broker / resolve prints what the provider command wrote
  vault: bare
  providers: |                       # written to <tmp>/config/knapper/providers.yaml
    providers:
      personal:
        command: [printf, "%s\n", "value-of-{locator}"]
  args: [resolve, "knapper://personal/address.home"]
  expect:
    lines: ["value-of-address.home"]
    providers:                       # the same file, read back after the run
      contains: ["personal:"]
```

The provider commands in these cases are `printf`, `true` and `false`.
knapper is provider-agnostic, so a fixture provider is just a command that
produces known bytes.

## Fixtures

`fixtures/flavors/` holds one tiny vault per ecosystem -- Obsidian, Foam,
Logseq, org-mode and the rest -- and backs the compatibility claims in
`docs/COMPATIBILITY.md`.

`fixtures/behaviour/` holds vaults that belong to no ecosystem and exist to
pin behaviour: `bare` has no config at all, `configured` exercises what
`knapper.config.md` can change, and `refactor` collects every form a link can
take so a rename has something to get wrong.

Every fixture must have at least one case; a test asserts it, because a
fixture nothing exercises is an untested claim.

## Adding a case

Prefer a case here over a unit test whenever the behaviour is something a
caller can observe. A behaviour that only shows up through an internal
function is, by definition, not part of the contract.
