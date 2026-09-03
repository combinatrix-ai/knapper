# Knapper Chrome integration

This Manifest V3 extension gives a local Knapper Native Messaging host a
narrow, write-oriented browser bridge. The extension uses `activeTab`,
`scripting`, `nativeMessaging`, and `storage`; `ALL` uses Chrome's optional
host-permission store for the exact origins the user has granted. It does not
use `debugger` or a custom origin allowlist.

## Modes

The toolbar action opens a small mode picker:

- **OFF** disconnects Native Messaging and clears the temporary PICK target.
- **PICK** retains the original `activeTab` workflow. The worker binds one
  session to the current tab, origin, and document URL, then waits for a click
  on a visible editable `text`, `email`, `tel`, `search`, or `url` input, or a
  `textarea`. After a successful fill, the target reference is discarded and
  PICK waits for a fresh field click. Navigation, tab close, Esc, or three
  minutes of inactivity turns it off.
- **ALL** asks Chrome for the current page's exact origin pattern (for example,
  `https://example.test/*`). If granted, ALL is persisted in `chrome.storage`
  and the Native Messaging connection stays available for permitted background
  tabs. Chrome's permission store is the only source of truth for which tabs
  are visible.

The popup sends `set_mode` messages to the worker; it never handles resolved
values.

## Background API

In ALL mode, the host may request:

```text
tabs_list       { request_id, origin? }
form_snapshot   { request_id, tab_id }
form_perform    { request_id, tab_id, document_id, actions }
form_submit     { request_id, tab_id, document_id, form_id }
```

Responses are typed results:

```text
tabs_listed       { request_id, tabs: [{ tab_id, url, origin, active }] }
form_snapshotted  { request_id, snapshot }
form_performed    { request_id, document_id, results }
form_submitted    { request_id, document_id, status: "submitted" }
api_rejected      { request_id, code }
```

The local client reads one request JSON object from stdin and prints one typed
response JSON object:

```sh
printf '%s' '{"op":"tabs_list","origin":"https://example.test"}' |
  knapper-chrome-client api
printf '%s' '{"op":"form_snapshot","tab_id":41}' |
  knapper-chrome-client api
printf '%s' '{"op":"form_perform","tab_id":41,"document_id":"document_1","actions":[{"op":"set_from","target_id":"target_1","reference":"knapper://personal/address.home"},{"op":"select_option","target_id":"target_2","value":"jp"},{"op":"set_checked","target_id":"target_3","checked":true}]}' |
  knapper-chrome-client api
```

`set_from` is resolved inside the Native Messaging host and becomes an opaque
`set_value` on the Chrome-owned pipe. Literal `set_value` is also available for
non-secret input. `form_submit` is deliberately a separate request; its
`submitted` result only means the browser dispatched submission, not that the
remote service accepted it.

`form_snapshot` returns a temporary `document_id`, a string `form_id` for
every form (including a non-submit-capable pseudo-form for controls outside a
form), and temporary `target_id` values. Controls carry semantic metadata such
as `type`, `tag`, `name`, `label`, `required`, `disabled`, `read_only`, and
`checked`; select options are `{ value, label, selected }`. Target references
are held only in the isolated content script and are invalidated by DOM
mutation, detach, or navigation. A stale target or document is rejected.

The only background actions are:

```text
set_value     { target_id, value, opaque? }
select_option { target_id, value }
set_checked   { target_id, checked }
```

There is no generic click operation. Submission is only possible through the
explicit `form_submit` request and a previously snapshotted real `form_id`.
Hidden controls may be set, but their current value is never included in a
snapshot. When the host marks a value `opaque: true` (the Knapper-resolved
path), all current values in that document are conservatively omitted from
later snapshots. This also prevents a framework rerender from copying an
opaque value into a replacement element and making it readable. Resolved
values are never echoed in any response, badge, title, log, or local-client
result.

If a `form_perform` uses a target from a previous snapshot and the target was
detached by a same-document rerender, the content script takes exactly one
fresh snapshot and remaps the entire batch by form identity plus
`name`/`label`/`type` (with `tag`/`kind` as structural checks). The remap must
be unique for every action; an ambiguous or missing target rejects the whole
batch before any write. Navigation or a different `document_id` is never
retried. Successful results continue to use the original requested
`target_id` values, so callers do not need to rewrite their action list.

## Live fixture E2E

The hermetic Node tests above exercise the worker and content-script contracts
without Chrome. A separate local fixture can exercise the complete installed
path: fixture page → loaded extension → Native Messaging host → Unix socket →
`knapper-chrome-client`.

```bash
node integration/chrome/e2e/live_bridge_test.mjs \
  --client "$HOME/.local/bin/knapper-chrome-client"
```

The runner serves
`http://knapper-e2e.localhost:48173/bridge-fixture.html` from the loopback
interface and waits for one matching tab. The dedicated `.localhost` hostname
keeps its Chrome host permission separate from ordinary `127.0.0.1` tools.
Open that exact URL in Chrome, click **Knapper Fill**, and choose **ALL** once
for the local origin. The rest is automatic. It uses only fixed, non-secret
literal values and never submits a form. It verifies:

- exact-origin tab discovery through the real CLI;
- one successful same-document stale-target remap with the original
  `target_id` in the result;
- `ambiguous_target` for a duplicate semantic target; and
- no partial write to either the unambiguous or ambiguous part of the rejected
  batch.

Use `--port`, `--timeout`, or `--client` to override the defaults. If a local
resolver does not support the dedicated hostname, `--host 127.0.0.1` is the
only fallback; note that Chrome then grants the extension access to that
loopback host rather than the fixture-specific hostname. The page is
served with `Cache-Control: no-store` and a restrictive Content Security
Policy. The runner stops its server on success or failure; the Chrome tab can
then be closed normally. If the tab remains open, it notices the next runner's
local token and reloads once. That reload wakes a suspended MV3 worker, which
restores persisted ALL mode and Native Messaging, so later runs need no popup
interaction.

## PICK client workflow

1. Open an HTTP(S) page and choose **PICK** in the extension popup.
2. Click one eligible text field. The action badge changes from **…** to
   **ON**.
3. Run the local client with a `knapper://` reference:

   ```sh
   knapper-chrome-client knapper://personal/address.home \
     --expected-origin https://example.test
   ```

4. The client receives one-line status JSON only. No approval popup is needed
   per field, and no form is submitted automatically.
5. Click the next text field and repeat. The extension action does not need to
   be pressed between fields.

## Development installation

1. Build the binaries so the Native Messaging host can find `knapper` beside
   `knapper-chrome-host`:

   ```sh
   cargo build --release --bin knapper --bin knapper-chrome-host --bin knapper-chrome-client
   ```

   Provider commands run from Chrome's Native Messaging environment, whose
   `PATH` may be narrower than an interactive shell's on macOS. Configure an
   absolute executable path when needed, for example:

   ```console
   knapper provider set personal -- /opt/homebrew/bin/op read 'op://Knapper/{locator}'
   ```

2. Open `chrome://extensions`, enable **Developer mode**, choose **Load
   unpacked**, and select this `integration/chrome/` directory. The fixed
   development key gives the extension ID
   `deebiomaecdepcdgeedbohdedkhnflkc`.
3. Copy `native-host-manifest.example.json`, replacing its `path` with the
   absolute path to `knapper-chrome-host`.
4. Install that manifest as Chrome's per-user Native Messaging manifest. On
   macOS the directory is:

   ```text
   ~/Library/Application Support/Google/Chrome/NativeMessagingHosts/
   ```

   Name the file `com.knapper.chrome.json`.

The native host manifest must keep the extension ID above. The host owns a
mode-0700 runtime directory and a user-only Unix socket. The resolved value is
carried only on the Chrome-owned Native Messaging pipe and is never returned
over the client socket or logged.

## Verification

```sh
node --check integration/chrome/service_worker.js
node --check integration/chrome/content_script.js
node --check integration/chrome/popup.js
node --test integration/chrome/service_worker.test.mjs
cargo test --test chrome_bridge
```
