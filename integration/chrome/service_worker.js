/* global chrome */

// Knapper Fill is a deliberately narrow, write-only browser capability. A
// session is fixed to the tab, origin, and document URL that were current when
// the extension action was pressed. The isolated content script keeps the
// actual element reference; this worker only sends commands to that script.
const HOST_NAME = "com.knapper.chrome";
const DEFAULT_TITLE = "Knapper Fill: select a text field";
const SESSION_TTL_MS = 3 * 60 * 1000;

let port = null;
let session = null;
let pending = null;
let sessionTimer = null;

function webContext(tab) {
  if (!tab || typeof tab.id !== "number" || typeof tab.url !== "string") return null;
  try {
    const parsed = new URL(tab.url);
    if (parsed.protocol !== "https:" && parsed.protocol !== "http:") return null;
    return { tabId: tab.id, url: tab.url, origin: parsed.origin };
  } catch (_) {
    return null;
  }
}

function validReference(reference) {
  if (typeof reference !== "string" || reference.length > 4096) return false;
  const match = /^knapper:\/\/([a-z0-9][a-z0-9_-]*)\/([A-Za-z0-9][A-Za-z0-9._/-]{0,127})$/.exec(reference);
  if (!match) return false;
  return match[2].split("/").every((part) => part !== "." && part !== ".." && part.length > 0);
}

function sameContext(left, right) {
  return Boolean(
    left && right &&
    left.tabId === right.tabId &&
    left.url === right.url &&
    left.origin === right.origin
  );
}

function setBadge(text, color) {
  void chrome.action.setBadgeText({ text }).catch(() => {});
  if (color) void chrome.action.setBadgeBackgroundColor({ color }).catch(() => {});
}

function showOff() {
  setBadge("");
  void chrome.action.setTitle({ title: DEFAULT_TITLE }).catch(() => {});
}

function showSelecting() {
  setBadge("…", "#9a6700");
  void chrome.action.setTitle({ title: "Knapper Fill: select a text field" }).catch(() => {});
}

function showAccepting() {
  setBadge("ON", "#18794e");
  void chrome.action.setTitle({ title: "Knapper Fill: accepting local input" }).catch(() => {});
}

function touchSession() {
  if (sessionTimer) clearTimeout(sessionTimer);
  if (!session) {
    sessionTimer = null;
    return;
  }
  const id = session.id;
  sessionTimer = setTimeout(() => {
    if (session && session.id === id) void disableSession();
  }, SESSION_TTL_MS);
}

function connect() {
  if (port) return port;
  let connectedPort;
  try {
    connectedPort = chrome.runtime.connectNative(HOST_NAME);
    port = connectedPort;
  } catch (_) {
    port = null;
    return null;
  }
  connectedPort.onMessage.addListener((message) => { void onNativeMessage(message); });
  connectedPort.onDisconnect.addListener(() => {
    if (port !== connectedPort) return;
    port = null;
    void disableSession(false);
  });
  try {
    connectedPort.postMessage({ type: "hello" });
  } catch (_) {
    port = null;
    return null;
  }
  return connectedPort;
}

function send(message) {
  if (!port) return false;
  try {
    port.postMessage(message);
    return true;
  } catch (_) {
    return false;
  }
}

async function sendTabMessage(tabId, message) {
  try {
    return await chrome.tabs.sendMessage(tabId, message);
  } catch (_) {
    return null;
  }
}

async function ensureContentScript(tabId) {
  try {
    await chrome.scripting.executeScript({
      target: { tabId, frameIds: [0] },
      files: ["content_script.js"]
    });
    return true;
  } catch (_) {
    return false;
  }
}

async function cancelContentScript(oldSession) {
  if (!oldSession) return;
  await sendTabMessage(oldSession.tabId, { type: "cancel" });
}

async function disableSession(disconnect = true) {
  const oldSession = session;
  session = null;
  if (sessionTimer) clearTimeout(sessionTimer);
  sessionTimer = null;
  if (pending) {
    send({ type: "target_rejected", request_id: pending.requestId, code: "accepting_disabled" });
    pending = null;
  }
  showOff();
  await cancelContentScript(oldSession);
  if (disconnect && port) {
    const oldPort = port;
    port = null;
    try { oldPort.disconnect(); } catch (_) {}
  }
}

async function selectInTab(expectedSession) {
  if (!session || session.id !== expectedSession.id) return false;
  const result = await sendTabMessage(expectedSession.tabId, { type: "select" });
  if (!session || session.id !== expectedSession.id) return false;
  if (!result || result.ok !== true) {
    await disableSession();
    return false;
  }
  return true;
}

async function startSelection(tab) {
  const context = webContext(tab);
  if (!context) return { mode: "error", code: "unsupported_page" };

  await disableSession();
  if (!connect()) return { mode: "error", code: "bridge_unavailable" };

  const next = {
    ...context,
    id: typeof crypto?.randomUUID === "function" ? crypto.randomUUID() : `${Date.now()}-${Math.random()}`,
    state: "selecting",
    descriptor: null
  };
  session = next;
  pending = null;
  touchSession();
  showSelecting();

  // This status is metadata-only. It lets the host expose a safe status to
  // the local client while the user is choosing a field.
  if (!send({ type: "selecting", tab_id: context.tabId, url: context.url, origin: context.origin })) {
    await disableSession();
    return { mode: "error", code: "bridge_unavailable" };
  }
  if (!await ensureContentScript(context.tabId)) {
    await disableSession();
    return { mode: "error", code: "content_script_unavailable" };
  }
  if (!await selectInTab(next)) return { mode: "error", code: "selection_failed" };
  return { mode: "selecting", origin: context.origin };
}

async function armSelectedControl(message, sender) {
  if (!session || session.state !== "selecting" || sender?.tab?.id !== session.tabId) return;
  if (message.origin !== session.origin || message.url !== session.url) {
    await disableSession();
    return;
  }
  session.state = "armed";
  session.descriptor = typeof message.descriptor === "string" ? message.descriptor.slice(0, 160) : null;
  touchSession();
  if (!send({ type: "arm", tab_id: session.tabId, url: session.url, origin: session.origin })) {
    await disableSession();
    return;
  }
  showAccepting();
}

async function returnToSelecting(filledSession, requestId) {
  if (!session || session.id !== filledSession.id) return false;
  session.state = "selecting";
  session.descriptor = null;
  pending = null;
  touchSession();
  showSelecting();
  if (!send({ type: "selecting", tab_id: session.tabId, url: session.url, origin: session.origin })) {
    await disableSession();
    return false;
  }
  if (!send({ type: "filled", request_id: requestId })) {
    await disableSession();
    return false;
  }
  return selectInTab(session);
}

async function handleActionClick(tab) {
  const context = webContext(tab);
  if (context && session && sameContext(context, session)) {
    await disableSession();
    return;
  }
  await startSelection(tab);
}

chrome.action.onClicked.addListener((tab) => { void handleActionClick(tab); });

chrome.runtime.onMessage.addListener((message, sender) => {
  if (!message || typeof message.type !== "string") return false;
  if (message.type === "selected") {
    void armSelectedControl(message, sender);
    return false;
  }
  if (message.type === "selection_cancelled" || message.type === "selection_timeout") {
    if (session && session.state === "selecting" && sender?.tab?.id === session.tabId) {
      void disableSession();
    }
    return false;
  }
  return false;
});

chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (!session || tabId !== session.tabId) return;
  if (changeInfo.status === "loading" || (typeof changeInfo.url === "string" && changeInfo.url !== session.url)) {
    void disableSession();
  }
});

chrome.tabs.onRemoved.addListener((tabId) => {
  if (session && tabId === session.tabId) void disableSession();
});

async function onNativeMessage(message) {
  if (!message || typeof message.type !== "string") return;
  if (message.type === "hello_ack" || message.type === "armed") return;

  if (message.type === "target_rejected" && message.request_id === "arm") {
    await disableSession();
    return;
  }

  if (message.type === "prepare_fill") {
    const currentSession = session;
    pending = null;
    if (
      !currentSession || currentSession.state !== "armed" ||
      message.expected_url !== currentSession.url || message.expected_origin !== currentSession.origin ||
      typeof message.request_id !== "string" || !validReference(message.reference) ||
      typeof message.timeout_ms !== "number" || message.timeout_ms < 1 || message.timeout_ms > 120000
    ) {
      send({ type: "target_rejected", request_id: message.request_id || "invalid", code: "not_accepting" });
      return;
    }
    touchSession();
    const targetReady = await sendTabMessage(currentSession.tabId, {
      type: "check",
      expected_origin: currentSession.origin,
      expected_url: currentSession.url
    });
    if (!session || session.id !== currentSession.id || !targetReady || targetReady.ok !== true) {
      send({ type: "target_rejected", request_id: message.request_id, code: targetReady?.code || "target_missing" });
      await disableSession();
      return;
    }
    pending = { requestId: message.request_id, session: currentSession };
    if (!send({ type: "target_ready", request_id: message.request_id })) {
      pending = null;
      await disableSession();
    }
    return;
  }

  if (message.type === "fill") {
    if (
      !pending || !session || pending.session.id !== session.id ||
      pending.requestId !== message.request_id || typeof message.value !== "string" ||
      message.value.length > 1024 * 1024 - 4096
    ) {
      send({ type: "fill_rejected", request_id: message.request_id || "invalid", code: "not_accepted" });
      return;
    }
    const request = pending;
    const filledSession = session;
    const result = await sendTabMessage(filledSession.tabId, {
      type: "fill",
      value: message.value,
      expected_origin: filledSession.origin,
      expected_url: filledSession.url
    });
    if (!result || result.ok !== true) {
      pending = null;
      send({ type: "fill_rejected", request_id: request.requestId, code: result?.code || "script_failed" });
      await disableSession();
      return;
    }
    await returnToSelecting(filledSession, request.requestId);
  }
}
