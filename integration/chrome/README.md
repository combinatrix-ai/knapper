# Knapper Chrome fill bridge

This Manifest V3 extension turns one user-selected text control into a narrow,
local write-only target for `knapper://` external references. It uses
`activeTab`, `scripting`, and Native Messaging; it does not request broad host
access or the Chrome debugger permission.

## Workflow

1. Open the target page and click the Knapper Fill extension action. A
   three-minute, tab-bound selection session starts immediately and the badge
   changes to **…**. The worker injects `content_script.js` into that document's
   isolated world; it is guarded so repeated injections do not duplicate
   listeners.
2. Click one visible editable text-like `input` or `textarea` on the page. The
   badge changes to **ON** and the host receives only the tab/origin/document
   metadata for the armed session.
3. Run:

   ```sh
   knapper-chrome-client knapper://personal/address.home \
     --expected-origin https://example.test
   ```

   At any point during the session, inspect its safe state without resolving a
   reference or exposing the page URL:

   ```sh
   knapper-chrome-client status
   ```

   The JSON response contains only the mode (`selecting`, `armed`, `resolving`,
   or `filling`) and origin. With no active extension session, the client emits
   a one-line `not_connected` JSON error and exits non-zero.

4. While that tab, URL, and selected element remain valid, the request fills
   that one element without a per-request approval popup. The client receives
   status-only JSON; it never receives the resolved value.
5. After a successful fill, the selected element reference is discarded, the
   badge returns to **…**, and the same session waits for the next text-field
   click. The extension action does not need to be pressed again between
   fields. Each fill therefore requires a fresh click, and the same element
   cannot be filled twice accidentally.
6. Click the extension action again to turn the capability off. Pressing Esc,
   a three-minute period without activity, reloading/navigating, or closing the
   tab also turns it off. The native connection is disconnected when the
   session ends.

The local caller cannot supply a CSS selector. The target is the exact element
the user clicked in Chrome, held only by the isolated content script. No
`data-*` marker, selector, value, or control DOM is placed in the page. Fills
are limited to visible, enabled, non-readonly `input` elements of type `text`,
`email`, `tel`, `search`, or `url`, plus `textarea`. Passwords, hidden
controls, selects, checkboxes, radios, file inputs, buttons, and contenteditable
elements are rejected. A fill dispatches `input` and `change`; it never clicks,
presses Enter, submits, navigates, or reads a field value. Content-script
responses contain only safe status/error codes and a bounded control
descriptor; values are checked for retention inside the isolated world and are
never returned.

## Development installation

1. Build and install the three binaries where the Native Messaging host can
   find `knapper` beside `knapper-chrome-host`:

   ```sh
   cargo build --release --bin knapper --bin knapper-chrome-host --bin knapper-chrome-client
   ```

   Provider commands run from Chrome's Native Messaging environment, whose
   `PATH` may be narrower than an interactive shell's on macOS. Configure an
   absolute executable path when the command is installed outside the system
   paths, for example:

   ```console
   knapper provider set personal -- /opt/homebrew/bin/op read 'op://Knapper/{locator}'
   ```

2. Open `chrome://extensions`, enable **Developer mode**, choose **Load
   unpacked**, and select this `integration/chrome/` directory. `manifest.json`
   fixes the unpacked extension ID as `deebiomaecdepcdgeedbohdedkhnflkc`.
3. Copy `native-host-manifest.example.json`, replacing its `path` with the
   absolute path to `knapper-chrome-host`.
4. Install that manifest as Chrome's per-user Native Messaging manifest. On
   macOS the directory is:

   ```text
   ~/Library/Application Support/Google/Chrome/NativeMessagingHosts/
   ```

   Name the file `com.knapper.chrome.json`.

The manifest allowlist must keep the extension ID above. The native host owns
a mode-0700 runtime directory and a user-only Unix socket. Provider output is
carried only from `knapper resolve` through the host and Chrome Native
Messaging; it is never returned over the client socket or logged.

## Verification

```sh
node --check integration/chrome/service_worker.js
node --test integration/chrome/service_worker.test.mjs
cargo test --test chrome_bridge
```
