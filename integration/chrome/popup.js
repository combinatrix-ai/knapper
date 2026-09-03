/* global chrome */

const status = document.querySelector("#status");
const buttons = Array.from(document.querySelectorAll("button[data-mode]"));

function setStatus(message) {
  status.textContent = message;
}

async function currentOriginPattern() {
  const tabs = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  const tab = tabs.length === 1 ? tabs[0] : null;
  if (!tab || typeof tab.url !== "string") return null;
  try {
    const url = new URL(tab.url);
    if (url.protocol !== "https:" && url.protocol !== "http:") return null;
    return { tab, origin: url.origin, pattern: `${url.origin}/*` };
  } catch (_) {
    return null;
  }
}

async function refresh() {
  try {
    const state = await chrome.runtime.sendMessage({ type: "get_state" });
    const mode = state?.mode || "off";
    for (const button of buttons) button.setAttribute("aria-pressed", String(button.dataset.mode === mode));
    setStatus(mode === "all" ? "ALL is active" : mode === "pick" ? "PICK is waiting" : "OFF");
  } catch (_) {
    setStatus("Mode unavailable");
  }
}

async function chooseMode(mode) {
  for (const button of buttons) button.disabled = true;
  try {
    if (mode === "all") {
      const current = await currentOriginPattern();
      if (!current) {
        setStatus("Open an HTTP(S) page first");
        return;
      }
      let granted = await chrome.permissions.contains({ origins: [current.pattern] });
      if (!granted) granted = await chrome.permissions.request({ origins: [current.pattern] });
      if (!granted) {
        setStatus("Chrome permission was not granted");
        return;
      }
    }
    const result = await chrome.runtime.sendMessage({ type: "set_mode", mode });
    if (result?.code === "permission_required") setStatus("Chrome permission is required");
    else if (result?.code) setStatus("Could not change mode");
    else setStatus(mode === "all" ? "ALL is active" : mode === "pick" ? "PICK is waiting" : "OFF");
  } catch (_) {
    setStatus("Could not change mode");
  } finally {
    for (const button of buttons) button.disabled = false;
    await refresh();
    if (mode === "pick") window.close();
  }
}

for (const button of buttons) button.addEventListener("click", () => { void chooseMode(button.dataset.mode); });
void refresh();
