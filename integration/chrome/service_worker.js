/* global chrome */

// Knapper Fill keeps browser access in this worker and keeps DOM references in
// the isolated content script. PICK is an activeTab, one-document session;
// ALL is persistent but can only see origins granted by Chrome's optional host
// permission store. Resolved values travel only on the Native Messaging pipe
// and are never returned in any worker response.
const HOST_NAME = "com.knapper.chrome";
const ALL_MODE_KEY = "knapperMode";
const DEFAULT_TITLE = "Knapper Fill: choose a mode";
const SESSION_TTL_MS = 3 * 60 * 1000;
const MAX_ACTIONS = 128;

let port = null;
let session = null;
let pending = null;
let sessionTimer = null;
let allMode = false;

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

function originPattern(origin) {
  return `${origin}/*`;
}

function validRequestId(value) {
  return typeof value === "string" && value.length > 0 && value.length <= 128 && /^[A-Za-z0-9_-]+$/.test(value);
}

function validReference(reference) {
  if (typeof reference !== "string" || reference.length > 4096) return false;
  const match = /^knapper:\/\/([a-z0-9][a-z0-9_-]*)\/([A-Za-z0-9][A-Za-z0-9._/-]{0,127})$/.exec(reference);
  if (!match) return false;
  return match[2].split("/").every((part) => part !== "." && part !== ".." && part.length > 0);
}

function sameContext(left, right) {
  return Boolean(
    left && right && left.tabId === right.tabId && left.url === right.url && left.origin === right.origin
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

function showPick(state = "selecting") {
  if (state === "armed") {
    setBadge("ON", "#18794e");
    void chrome.action.setTitle({ title: "Knapper Fill: PICK is armed" }).catch(() => {});
  } else {
    setBadge("…", "#9a6700");
    void chrome.action.setTitle({ title: "Knapper Fill: PICK: choose a text field" }).catch(() => {});
  }
}

function showAll() {
  setBadge("ALL", "#2457a6");
  void chrome.action.setTitle({ title: "Knapper Fill: ALL mode" }).catch(() => {});
}

function touchSession() {
  if (sessionTimer) clearTimeout(sessionTimer);
  if (!session) {
    sessionTimer = null;
    return;
  }
  const id = session.id;
  sessionTimer = setTimeout(() => {
    if (session && session.id === id) void setMode("off");
  }, SESSION_TTL_MS);
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
    if (session) void disablePick(false);
    // ALL is persistent; it will reconnect on the next wakeup or mode/status
    // request. Do not spin a reconnect loop while Chrome is shutting down.
  });
  try {
    connectedPort.postMessage({ type: "hello" });
  } catch (_) {
    port = null;
    return null;
  }
  return connectedPort;
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
    await chrome.scripting.executeScript({ target: { tabId, frameIds: [0] }, files: ["content_script.js"] });
    return true;
  } catch (_) {
    return false;
  }
}

async function disablePick(disconnect = true) {
  const oldSession = session;
  session = null;
  if (sessionTimer) clearTimeout(sessionTimer);
  sessionTimer = null;
  if (pending) {
    send({ type: "target_rejected", request_id: pending.requestId, code: "accepting_disabled" });
    pending = null;
  }
  if (!allMode) showOff();
  if (oldSession) await sendTabMessage(oldSession.tabId, { type: "cancel" });
  if (disconnect && port && !allMode) {
    const oldPort = port;
    port = null;
    try { oldPort.disconnect(); } catch (_) {}
  }
}

async function setStoredMode(mode) {
  try {
    await chrome.storage.local.set({ [ALL_MODE_KEY]: mode });
    return true;
  } catch (_) {
    return false;
  }
}

async function isGranted(origin) {
  if (!origin || !chrome.permissions?.contains) return false;
  try {
    return await chrome.permissions.contains({ origins: [originPattern(origin)] });
  } catch (_) {
    return false;
  }
}

async function accessibleTabs(requestOrigin = null) {
  let tabs;
  try {
    tabs = await chrome.tabs.query({});
  } catch (_) {
    return [];
  }
  const result = [];
  for (const tab of tabs) {
    const context = webContext(tab);
    if (!context || (requestOrigin && context.origin !== requestOrigin) || !await isGranted(context.origin)) continue;
    result.push({ tab_id: context.tabId, url: context.url, origin: context.origin, active: Boolean(tab.active) });
  }
  return result;
}

async function enableAll(tabId, origin) {
  if (!allMode || typeof tabId !== "number" || typeof origin !== "string") return;
  if (!await isGranted(origin)) return;
  await ensureContentScript(tabId);
}

async function enableAllForTabs() {
  const tabs = await accessibleTabs();
  await Promise.all(tabs.map((tab) => enableAll(tab.tab_id, tab.origin)));
}

async function enterAllMode() {
  await disablePick();
  allMode = true;
  if (!await setStoredMode("all")) {
    allMode = false;
    showOff();
    return { mode: "off", code: "storage_unavailable" };
  }
  showAll();
  if (!connect()) return { mode: "all", code: "bridge_unavailable" };
  send({ type: "mode", mode: "all" });
  await enableAllForTabs();
  return { mode: "all" };
}

async function enterPickMode(tab) {
  if (allMode || port) await setMode("off");
  allMode = false;
  await setStoredMode("off");
  return startSelection(tab);
}

async function setMode(mode, tab = null) {
  if (mode === "off") {
    allMode = false;
    await setStoredMode("off");
    if (port) send({ type: "mode", mode: "off" });
    await disablePick();
    if (port) {
      const oldPort = port;
      port = null;
      try { oldPort.disconnect(); } catch (_) {}
    }
    showOff();
    return { mode: "off" };
  }
  if (mode === "all") return enterAllMode();
  if (mode === "pick") {
    const target = tab || await activeTab();
    if (!target) return { mode: "off", code: "no_active_tab" };
    return enterPickMode(target);
  }
  return { mode: "off", code: "invalid_mode" };
}

async function activeTab() {
  try {
    const tabs = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    return tabs.length === 1 ? tabs[0] : null;
  } catch (_) {
    return null;
  }
}

async function startSelection(tab) {
  const context = webContext(tab);
  if (!context) return { mode: "off", code: "unsupported_page" };
  await disablePick();
  if (!connect()) return { mode: "off", code: "bridge_unavailable" };
  const next = {
    ...context,
    id: typeof crypto?.randomUUID === "function" ? crypto.randomUUID() : `${Date.now()}-${Math.random()}`,
    state: "selecting",
    descriptor: null
  };
  session = next;
  pending = null;
  touchSession();
  showPick();
  if (!send({ type: "mode", mode: "pick" }) || !send({ type: "selecting", tab_id: context.tabId, url: context.url, origin: context.origin })) {
    await disablePick();
    return { mode: "off", code: "bridge_unavailable" };
  }
  if (!await ensureContentScript(context.tabId)) {
    await disablePick();
    return { mode: "off", code: "content_script_unavailable" };
  }
  const selected = await sendTabMessage(context.tabId, { type: "select" });
  if (!selected || selected.ok !== true) {
    await disablePick();
    return { mode: "off", code: "selection_failed" };
  }
  return { mode: "pick" };
}

async function armSelectedControl(message, sender) {
  if (!session || session.state !== "selecting" || sender?.tab?.id !== session.tabId) return;
  if (message.origin !== session.origin || message.url !== session.url) {
    await disablePick();
    return;
  }
  session.state = "armed";
  session.descriptor = typeof message.descriptor === "string" ? message.descriptor.slice(0, 160) : null;
  touchSession();
  if (!send({ type: "arm", tab_id: session.tabId, url: session.url, origin: session.origin })) {
    await disablePick();
    return;
  }
  showPick("armed");
}

async function returnToSelecting(filledSession, requestId) {
  if (!session || session.id !== filledSession.id) return false;
  session.state = "selecting";
  session.descriptor = null;
  pending = null;
  touchSession();
  showPick();
  if (!send({ type: "selecting", tab_id: session.tabId, url: session.url, origin: session.origin }) || !send({ type: "filled", request_id: requestId })) {
    await disablePick();
    return false;
  }
  const selected = await sendTabMessage(session.tabId, { type: "select" });
  if (!selected || selected.ok !== true) {
    await disablePick();
    return false;
  }
  return true;
}

async function tabForId(tabId) {
  try {
    return await chrome.tabs.get(tabId);
  } catch (_) {
    try {
      const tabs = await chrome.tabs.query({});
      return tabs.find((tab) => tab.id === tabId) || null;
    } catch (_) {
      return null;
    }
  }
}

async function rejectApi(requestId, code) {
  send({ type: "api_rejected", request_id: validRequestId(requestId) ? requestId : "invalid", code });
}

async function prepareAllTab(tabId) {
  if (!allMode || typeof tabId !== "number") return null;
  const tab = await tabForId(tabId);
  const context = webContext(tab);
  if (!context || !await isGranted(context.origin)) return null;
  if (!await ensureContentScript(tabId)) return null;
  return context;
}

async function handleTabsList(message) {
  if (!allMode || !validRequestId(message.request_id)) return rejectApi(message.request_id, "not_all");
  const tabs = await accessibleTabs(typeof message.origin === "string" ? message.origin : null);
  send({ type: "tabs_listed", request_id: message.request_id, tabs });
}

async function handleFormSnapshot(message) {
  if (!allMode || !validRequestId(message.request_id)) return rejectApi(message.request_id, "not_all");
  const context = await prepareAllTab(message.tab_id);
  if (!context) return rejectApi(message.request_id, "tab_unavailable");
  const result = await sendTabMessage(context.tabId, { type: "form_snapshot" });
  if (!result || result.ok !== true || !result.snapshot) return rejectApi(message.request_id, result?.code || "snapshot_failed");
  send({
    type: "form_snapshotted",
    request_id: message.request_id,
    snapshot: { ...result.snapshot, tab_id: context.tabId, url: context.url, origin: context.origin }
  });
}

function validActions(actions) {
  const allowed = new Set(["set_value", "select_option", "set_checked"]);
  return Array.isArray(actions) && actions.length <= MAX_ACTIONS && actions.every((action) => action && typeof action === "object" && typeof action.op === "string" && allowed.has(action.op) && typeof action.target_id === "string");
}

async function handleFormPerform(message) {
  if (!allMode || !validRequestId(message.request_id)) return rejectApi(message.request_id, "not_all");
  if (typeof message.document_id !== "string") return rejectApi(message.request_id, "invalid_request");
  if (!validActions(message.actions)) return rejectApi(message.request_id, "invalid_actions");
  const context = await prepareAllTab(message.tab_id);
  if (!context) return rejectApi(message.request_id, "tab_unavailable");
  const result = await sendTabMessage(context.tabId, {
    type: "form_perform",
    document_id: message.document_id,
    actions: message.actions
  });
  if (!result || result.ok !== true) return rejectApi(message.request_id, result?.code || "perform_failed");
  send({ type: "form_performed", request_id: message.request_id, document_id: result.document_id || message.document_id, results: result.results || [] });
}

async function handleFormSubmit(message) {
  if (!allMode || !validRequestId(message.request_id)) return rejectApi(message.request_id, "not_all");
  if (typeof message.document_id !== "string" || typeof message.form_id !== "string") return rejectApi(message.request_id, "invalid_request");
  const context = await prepareAllTab(message.tab_id);
  if (!context) return rejectApi(message.request_id, "tab_unavailable");
  const result = await sendTabMessage(context.tabId, {
    type: "form_submit",
    document_id: message.document_id,
    form_id: message.form_id
  });
  if (!result || result.ok !== true) return rejectApi(message.request_id, result?.code || "submit_failed");
  send({ type: "form_submitted", request_id: message.request_id, document_id: result.document_id || message.document_id, status: "submitted" });
}

async function onNativeMessage(message) {
  if (!message || typeof message.type !== "string") return;
  if (message.type === "hello_ack" || message.type === "armed") return;

  if (message.type === "target_rejected" && message.request_id === "arm") {
    await disablePick();
    return;
  }

  if (message.type === "tabs_list") return handleTabsList(message);
  if (message.type === "form_snapshot") return handleFormSnapshot(message);
  if (message.type === "form_perform") return handleFormPerform(message);
  if (message.type === "form_submit") return handleFormSubmit(message);

  if (message.type === "prepare_fill") {
    const currentSession = session;
    pending = null;
    if (
      !currentSession || currentSession.state !== "armed" ||
      message.expected_url !== currentSession.url || message.expected_origin !== currentSession.origin ||
      !validRequestId(message.request_id) || !validReference(message.reference) ||
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
      await disablePick();
      return;
    }
    pending = { requestId: message.request_id, session: currentSession };
    if (!send({ type: "target_ready", request_id: message.request_id })) {
      pending = null;
      await disablePick();
    }
    return;
  }

  if (message.type === "fill") {
    if (
      !pending || !session || pending.session.id !== session.id || pending.requestId !== message.request_id ||
      typeof message.value !== "string" || message.value.length > 1024 * 1024 - 4096
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
      await disablePick();
      return;
    }
    await returnToSelecting(filledSession, request.requestId);
  }
}

chrome.action.onClicked.addListener((tab) => { void handleActionClick(tab); });

async function handleActionClick(tab) {
  const context = webContext(tab);
  if (context && session && sameContext(context, session)) {
    await setMode("off");
  } else if (context && allMode) {
    await setMode("off");
  } else {
    await startSelection(tab);
  }
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (!message || typeof message.type !== "string") return false;
  if (message.type === "selected") {
    void armSelectedControl(message, sender);
    return false;
  }
  if (message.type === "selection_cancelled" || message.type === "selection_timeout") {
    if (session && session.state === "selecting" && sender?.tab?.id === session.tabId) void disablePick();
    return false;
  }
  if (message.type === "get_state") {
    void getState().then((state) => sendResponse(state));
    return true;
  }
  if (message.type === "set_mode") {
    void handleSetMode(message).then(sendResponse);
    return true;
  }
  return false;
});

async function getState() {
  try {
    const stored = await chrome.storage.local.get({ [ALL_MODE_KEY]: "off" });
    if (stored[ALL_MODE_KEY] === "all") {
      if (!allMode || !port) await restoreAllMode();
    }
  } catch (_) {}
  return { mode: allMode ? "all" : session ? "pick" : "off", connected: Boolean(port) };
}

async function handleSetMode(message) {
  if (message.mode === "all") {
    const tab = await activeTab();
    const context = webContext(tab);
    if (!context || !await isGranted(context.origin)) return { mode: "off", code: "permission_required", origin: context?.origin };
    return enterAllMode();
  }
  if (message.mode === "pick") return setMode("pick");
  if (message.mode === "off") return setMode("off");
  return { mode: "off", code: "invalid_mode" };
}

async function handleTabUpdate(tabId, changeInfo, tab) {
  if (session && tabId === session.tabId && (changeInfo.status === "loading" || (typeof changeInfo.url === "string" && changeInfo.url !== session.url))) {
    await disablePick();
  }
  if (allMode && tab && (changeInfo.status === "complete" || typeof changeInfo.url === "string")) {
    const context = webContext(tab);
    if (context && await isGranted(context.origin)) {
      await enableAll(tabId, context.origin);
    }
  }
}

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => { void handleTabUpdate(tabId, changeInfo, tab); });
chrome.tabs.onRemoved.addListener((tabId) => {
  if (session && tabId === session.tabId) void disablePick();
});

async function restoreAllMode() {
  try {
    const stored = await chrome.storage.local.get({ [ALL_MODE_KEY]: "off" });
    if (stored[ALL_MODE_KEY] !== "all") return;
    allMode = true;
    showAll();
    if (connect()) {
      send({ type: "mode", mode: "all" });
      await enableAllForTabs();
    }
  } catch (_) {}
}

if (chrome.runtime.onStartup) chrome.runtime.onStartup.addListener(() => { void restoreAllMode(); });
if (chrome.runtime.onInstalled) chrome.runtime.onInstalled.addListener(() => { void restoreAllMode(); });
