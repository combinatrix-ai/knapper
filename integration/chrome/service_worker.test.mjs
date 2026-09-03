import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function eventTarget() {
  const listeners = [];
  return {
    listeners,
    addListener(listener) { listeners.push(listener); }
  };
}

function loadHarness() {
  const actionClicked = eventTarget();
  const runtimeMessages = eventTarget();
  const onStartup = eventTarget();
  const onInstalled = eventTarget();
  const onRemoved = eventTarget();
  const onUpdated = eventTarget();
  const sentNative = [];
  const sentTabs = [];
  const tabs = new Map([[7, { ...TAB, active: true }]]);
  const tabList = [tabs.get(7)];
  const grantedOrigins = new Set(["https://example.test/*"]);
  const storageState = { knapperMode: "off" };
  const actionState = { badge: null, title: null };
  let port = null;
  let disconnects = 0;
  let executeScriptCalls = [];

  const context = vm.createContext({
    URL,
    chrome: {
      action: {
        onClicked: actionClicked,
        setBadgeBackgroundColor: async () => {},
        setBadgeText: async ({ text }) => { actionState.badge = text; },
        setTitle: async ({ title }) => { actionState.title = title; }
      },
      runtime: {
        connectNative() {
          port = {
            onMessage: eventTarget(),
            onDisconnect: eventTarget(),
            postMessage(message) { sentNative.push(message); },
            disconnect() {
              disconnects += 1;
              for (const listener of port.onDisconnect.listeners) listener();
            }
          };
          return port;
        },
        onMessage: runtimeMessages,
        onStartup,
        onInstalled
      },
      storage: {
        local: {
          async get(defaults = {}) { return { ...defaults, ...storageState }; },
          async set(values) { Object.assign(storageState, values); }
        }
      },
      permissions: {
        async contains({ origins }) { return origins.every((origin) => grantedOrigins.has(origin)); },
        async request({ origins }) { origins.forEach((origin) => grantedOrigins.add(origin)); return true; }
      },
      scripting: {
        executeScript: async (details) => {
          executeScriptCalls.push(details);
          return [{ result: { ok: true } }];
        }
      },
      tabs: {
        onRemoved,
        onUpdated,
        query: async (query = {}) => query.active ? tabList : tabList,
        get: async (tabId) => tabs.get(tabId),
        sendMessage: async (tabId, message) => {
          sentTabs.push({ tabId, message });
          const tab = tabs.get(tabId);
          if (tab?.sendMessage) return tab.sendMessage(message);
          return { ok: true };
        }
      }
    },
    crypto: { randomUUID: () => "session-1" },
    // Selection expiry is covered by the worker's TTL tests. Content-script
    // tests use inert timers so a deliberately unselected control does not
    // hold the Node test process open for three minutes.
    setTimeout: () => 1,
    clearTimeout: () => {}
  });

  const source = readFileSync(new URL("./service_worker.js", import.meta.url), "utf8");
  vm.runInContext(source, context, { filename: "service_worker.js" });
  return {
    context,
    actionClicked,
    runtimeMessages,
    sentNative,
    sentTabs,
    tabs,
    actionState,
    executeScriptCalls,
    tabList,
    grantedOrigins,
    storageState,
    evaluate(source) { return vm.runInContext(source, context); },
    get disconnects() { return disconnects; },
    async cleanup() { await context.setMode("off"); }
  };
}

function assertVmResponse(actual, expected) {
  assert.equal(JSON.stringify(actual), JSON.stringify(expected));
}

function loadContentScriptHarness() {
  const runtimeMessages = eventTarget();
  const sentRuntime = [];
  const documentListeners = new Map();

  class Element {
    constructor() {
      this.id = "";
      this.isConnected = true;
      this.isContentEditable = false;
      this.disabled = false;
      this.readOnly = false;
      this.events = [];
      this.attributes = new Map();
      this.style = { display: "block", visibility: "visible", opacity: "1", outline: "", outlineOffset: "" };
    }

    matches(selector) { return selector === "[contenteditable]" && this.attributes.has("contenteditable"); }
    getAttribute(name) { return this.attributes.get(name) ?? null; }
    setAttribute(name, value) { this.attributes.set(name, String(value)); }
    removeAttribute(name) { this.attributes.delete(name); }
    getBoundingClientRect() { return { width: 160, height: 28 }; }
    focus() { this.focused = true; }
    dispatchEvent(event) { this.events.push(event.type); }
  }

  class HTMLInputElement extends Element {
    constructor(type = "text") {
      super();
      this.type = type;
      this.tagName = "INPUT";
    }
  }
  Object.defineProperty(HTMLInputElement.prototype, "value", {
    configurable: true,
    get() { return this.assignedValue ?? ""; },
    set(value) { this.assignedValue = value; }
  });

  class HTMLTextAreaElement extends Element {
    constructor() {
      super();
      this.type = "textarea";
      this.tagName = "TEXTAREA";
    }
  }
  Object.defineProperty(HTMLTextAreaElement.prototype, "value", {
    configurable: true,
    get() { return this.assignedValue ?? ""; },
    set(value) { this.assignedValue = value; }
  });

  class HTMLSelectElement extends Element {
    constructor(options = []) {
      super();
      this.tagName = "SELECT";
      this.options = options;
      this.selectedIndex = -1;
      for (const option of options) option.parentSelect = this;
    }

    querySelectorAll() { return []; }
  }

  class HTMLFormElement extends Element {
    constructor(controls = []) {
      super();
      this.tagName = "FORM";
      this.method = "post";
      this.action = "https://example.test/submit";
      this.controls = controls;
      this.submitted = false;
      for (const control of controls) control.form = this;
    }

    querySelectorAll() { return this.controls; }
    requestSubmit() { this.submitted = true; }
  }

  class Event {
    constructor(type) { this.type = type; }
  }

  const document = {
    forms: [],
    controls: [],
    addEventListener(type, listener) {
      const listeners = documentListeners.get(type) ?? [];
      listeners.push(listener);
      documentListeners.set(type, listeners);
    },
    removeEventListener(type, listener) {
      const listeners = documentListeners.get(type) ?? [];
      documentListeners.set(type, listeners.filter((candidate) => candidate !== listener));
    },
    dispatchEvent(event) {
      for (const listener of [...(documentListeners.get(event.type) ?? [])]) listener(event);
    },
    querySelectorAll(selector) {
      if (selector === "form") return this.forms;
      if (selector === "input, textarea, select") return this.controls;
      return [];
    }
  };
  const location = { origin: "https://example.test", href: "https://example.test/form" };
  let uuidCounter = 0;
  const context = vm.createContext({
    Element,
    HTMLInputElement,
    HTMLTextAreaElement,
    HTMLSelectElement,
    HTMLFormElement,
    Event,
    document,
    location,
    crypto: { randomUUID: () => `content-id-${++uuidCounter}` },
    getComputedStyle: (element) => element.style,
    chrome: {
      runtime: {
        onMessage: runtimeMessages,
        sendMessage(message) { sentRuntime.push(message); }
      }
    },
    // Keep selection expiry inert in this synchronous VM harness. The worker
    // session TTL is exercised through its lifecycle tests above.
    setTimeout: () => 1,
    clearTimeout: () => {}
  });
  const source = readFileSync(new URL("./content_script.js", import.meta.url), "utf8");
  vm.runInContext(source, context, { filename: "content_script.js" });

  function send(message) {
    let response;
    const listener = runtimeMessages.listeners[0];
    listener(message, { tab: { id: 7 } }, (value) => { response = value; });
    return response;
  }
  function click(element) {
    const event = {
      type: "click",
      target: element,
      preventDefault() { this.defaultPrevented = true; },
      stopImmediatePropagation() { this.stopped = true; }
    };
    document.dispatchEvent(event);
    return event;
  }
  return {
    context,
    document,
    location,
    sentRuntime,
    send,
    click,
    input(type = "text") { return new HTMLInputElement(type); },
    textarea() { return new HTMLTextAreaElement(); },
    select(options = []) { return new HTMLSelectElement(options); },
    form(controls = []) { return new HTMLFormElement(controls); },
    addForm(form) {
      document.forms.push(form);
      document.controls.push(...form.controls);
      return form;
    },
    addControl(control) {
      document.controls.push(control);
      return control;
    }
  };
}

const TAB = { id: 7, url: "https://example.test/form" };
const CONTEXT = { tabId: 7, url: TAB.url, origin: "https://example.test" };

test("manifest stays on activeTab scripting without debugger or broad host access", () => {
  const manifest = JSON.parse(readFileSync(new URL("./manifest.json", import.meta.url), "utf8"));
  assert.deepEqual(manifest.permissions, ["activeTab", "nativeMessaging", "scripting", "storage"]);
  assert.equal(manifest.host_permissions, undefined);
  assert.equal(manifest.action.default_popup, "popup.html");
  assert.deepEqual(manifest.optional_host_permissions, ["http://*/*", "https://*/*"]);
  assert.ok(!manifest.permissions.includes("debugger"));
});

test("the content script is isolated and does not use a page-visible target marker", () => {
  const source = readFileSync(new URL("./content_script.js", import.meta.url), "utf8");
  assert.ok(source.includes("chrome.runtime.onMessage"));
  assert.ok(source.includes("state.selected"));
  assert.ok(!source.includes("data-knapper-fill-target"));
  assert.ok(!source.includes("createElement"));
});

test("content script accepts only visible editable text-like controls", () => {
  const acceptedTypes = ["text", "email", "tel", "search", "url"];
  const rejectedTypes = ["password", "hidden", "checkbox", "radio", "file", "submit", "button"];
  for (const type of acceptedTypes) {
    const harness = loadContentScriptHarness();
    const input = harness.input(type);
    assertVmResponse(harness.send({ type: "select" }), { ok: true });
    harness.click(input);
    assert.equal(harness.sentRuntime.at(-1).type, "selected", type);
    assertVmResponse(harness.send({
      type: "check",
      expected_origin: harness.location.origin,
      expected_url: harness.location.href
    }), { ok: true });
    harness.send({ type: "cancel" });
  }
  for (const type of rejectedTypes) {
    const harness = loadContentScriptHarness();
    const input = harness.input(type);
    assertVmResponse(harness.send({ type: "select" }), { ok: true });
    harness.click(input);
    assert.equal(harness.sentRuntime.length, 0, type);
    assertVmResponse(harness.send({
      type: "check",
      expected_origin: harness.location.origin,
      expected_url: harness.location.href
    }), { ok: false, code: "target_missing" });
  }

  for (const mutate of [
    (input) => { input.disabled = true; },
    (input) => { input.readOnly = true; },
    (input) => { input.isConnected = false; },
    (input) => { input.style.display = "none"; },
    (input) => { input.style.visibility = "hidden"; },
    (input) => { input.style.opacity = "0"; },
    (input) => { input.isContentEditable = true; },
    (input) => { input.setAttribute("contenteditable", "true"); }
  ]) {
    const harness = loadContentScriptHarness();
    const input = harness.input();
    mutate(input);
    harness.send({ type: "select" });
    harness.click(input);
    assert.equal(harness.sentRuntime.length, 0);
  }
});

test("content script dispatches input/change, retains no value, and discards the target after fill", () => {
  const harness = loadContentScriptHarness();
  const input = harness.input("email");
  harness.send({ type: "select" });
  harness.click(input);
  const args = { expected_origin: harness.location.origin, expected_url: harness.location.href };

  assertVmResponse(harness.send({ type: "fill", value: "private value", ...args }), { ok: true });
  assert.equal(input.assignedValue, "private value");
  assert.deepEqual(input.events, ["input", "change"]);
  assertVmResponse(harness.send({ type: "fill", value: "second value", ...args }), { ok: false, code: "target_missing" });
  assert.deepEqual(input.events, ["input", "change"]);
  assert.ok(![...input.attributes.keys()].some((name) => name.startsWith("data-knapper")));

  // A second fill is possible only after a fresh selection click.
  harness.send({ type: "select" });
  harness.click(input);
  assertVmResponse(harness.send({ type: "fill", value: "second value", ...args }), { ok: true });
  assert.deepEqual(input.events, ["input", "change", "input", "change"]);

  const notRetained = loadContentScriptHarness();
  const broken = notRetained.input("text");
  Object.defineProperty(broken, "value", { configurable: true, get() { return ""; }, set() {} });
  notRetained.send({ type: "select" });
  notRetained.click(broken);
  assertVmResponse(notRetained.send({ type: "fill", value: "private value", ...args }), {
    ok: false,
    code: "value_not_retained"
  });
  assert.deepEqual(broken.events, ["input", "change"]);
});

test("content script rejects origin or document URL changes before touching the target", () => {
  const harness = loadContentScriptHarness();
  const input = harness.textarea();
  harness.send({ type: "select" });
  harness.click(input);
  const expected = { expected_origin: harness.location.origin, expected_url: harness.location.href };
  harness.location.href = "https://example.test/other";
  assertVmResponse(harness.send({ type: "fill", value: "x", ...expected }), { ok: false, code: "navigation" });
  assert.deepEqual(input.events, []);

  harness.location.href = "https://attacker.test/form";
  harness.location.origin = "https://attacker.test";
  assertVmResponse(harness.send({ type: "check", ...expected }), { ok: false, code: "navigation" });
  assert.deepEqual(input.events, []);
});

test("content script snapshots temporary form targets and performs semantic operations only", () => {
  const harness = loadContentScriptHarness();
  const text = harness.input("text");
  text.setAttribute("name", "display_name");
  text.setAttribute("autocomplete", "name");
  text.required = true;
  text.assignedValue = "ordinary value";
  const checkbox = harness.input("checkbox");
  checkbox.setAttribute("name", "terms");
  checkbox.checked = false;
  const hidden = harness.input("hidden");
  hidden.setAttribute("name", "csrf_token");
  hidden.assignedValue = "hidden synthetic";
  const options = [
    { value: "jp", label: "Japan", textContent: "Japan", disabled: false },
    { value: "us", label: "United States", textContent: "United States", disabled: false }
  ];
  const select = harness.select(options);
  select.setAttribute("name", "country");
  const form = harness.form([text, checkbox, select, hidden]);
  harness.addForm(form);

  const initial = harness.send({ type: "form_snapshot" });
  assert.equal(initial.ok, true);
  assert.equal(typeof initial.snapshot.document_id, "string");
  assert.equal(initial.snapshot.forms.length, 1);
  const controls = initial.snapshot.forms[0].controls;
  assert.equal(controls.length, 4);
  const textTarget = controls.find((control) => control.name === "display_name");
  const checkboxTarget = controls.find((control) => control.name === "terms");
  const selectTarget = controls.find((control) => control.name === "country");
  const hiddenTarget = controls.find((control) => control.name === "csrf_token");
  assert.equal(textTarget.kind, "text");
  assert.equal(textTarget.current_value, "ordinary value");
  assert.equal(checkboxTarget.kind, "checkbox");
  assert.equal(checkboxTarget.checked, false);
  assert.equal(hiddenTarget.type, "hidden");
  assert.equal(Object.hasOwn(hiddenTarget, "current_value"), false);
  assert.equal(selectTarget.kind, "select");
  assert.equal(selectTarget.options[0].value, "jp");
  assert.deepEqual(Object.keys(selectTarget.options[0]).sort(), ["label", "selected", "value"]);

  const performed = harness.send({
    type: "form_perform",
    document_id: initial.snapshot.document_id,
    actions: [
      { op: "set_value", target_id: textTarget.target_id, value: "opaque value", opaque: true },
      { op: "select_option", target_id: selectTarget.target_id, value: "jp" },
      { op: "set_checked", target_id: checkboxTarget.target_id, checked: true },
      { op: "set_value", target_id: hiddenTarget.target_id, value: "opaque hidden", opaque: true },
      { op: "set_value", target_id: selectTarget.target_id, value: "wrong kind", opaque: true },
      { op: "click", target_id: textTarget.target_id }
    ]
  });
  assert.equal(performed.ok, true);
  assert.equal(performed.results[0].status, "verified");
  assert.equal(performed.results[1].status, "verified");
  assert.equal(performed.results[2].status, "verified");
  assert.equal(performed.results[3].status, "verified");
  assert.equal(performed.results[4].status, "rejected");
  assert.equal(performed.results[4].code, "wrong_control_kind");
  assert.equal(performed.results[5].status, "rejected");
  assert.equal(performed.results[5].code, "unsupported_operation");
  assert.equal(performed.results.every((result) => result.value_returned === false), true);
  assert.equal(text.assignedValue, "opaque value");
  assert.equal(hidden.assignedValue, "opaque hidden");
  assert.equal(select.selectedIndex, 0);
  assert.equal(checkbox.checked, true);
  assert.deepEqual(text.events, ["input", "change"]);
  assert.deepEqual(select.events, ["input", "change"]);
  assert.deepEqual(checkbox.events, ["input", "change"]);

  const afterOpaque = harness.send({ type: "form_snapshot" });
  const opaqueMetadata = afterOpaque.snapshot.forms[0].controls.find((control) => control.name === "display_name");
  assert.equal(opaqueMetadata.value_opaque, true);
  assert.equal(Object.hasOwn(opaqueMetadata, "current_value"), false);
  for (const control of afterOpaque.snapshot.forms[0].controls) {
    assert.equal(control.value_opaque, true);
    assert.equal(Object.hasOwn(control, "current_value"), false);
  }

  const submitted = harness.send({
    type: "form_submit",
    document_id: afterOpaque.snapshot.document_id,
    form_id: afterOpaque.snapshot.forms[0].form_id
  });
  assert.equal(submitted.ok, true);
  assert.equal(form.submitted, true);

  const staleDocument = harness.send({ type: "form_perform", document_id: "different-document", actions: [] });
  assert.equal(staleDocument.ok, false);
  assert.equal(staleDocument.code, "stale_document");

  const staleTarget = afterOpaque.snapshot.forms[0].controls.find((control) => control.name === "terms");
  checkbox.isConnected = false;
  const stale = harness.send({
    type: "form_perform",
    document_id: afterOpaque.snapshot.document_id,
    actions: [{ op: "set_checked", target_id: staleTarget.target_id, checked: false }]
  });
  assert.equal(stale.ok, true);
  assert.equal(stale.results[0].status, "rejected");
  assert.equal(stale.results[0].code, "stale_target");
});

test("the extension action starts one tab-bound selecting session", async () => {
  const harness = loadHarness();
  try {
    await harness.context.handleActionClick(TAB);
    assert.equal(harness.evaluate("session.state"), "selecting");
    assert.equal(harness.evaluate("session.tabId"), TAB.id);
    assert.equal(harness.evaluate("session.url"), TAB.url);
    assert.deepEqual(harness.sentNative.map((message) => message.type), ["hello", "mode", "selecting"]);
    assert.equal(JSON.stringify(harness.sentTabs), JSON.stringify([{ tabId: TAB.id, message: { type: "select" } }]));
    assert.equal(JSON.stringify(harness.executeScriptCalls.map((call) => call.files)), JSON.stringify([["content_script.js"]]));
  } finally {
    await harness.cleanup();
  }
});

test("OFF, PICK, and ALL are explicit modes and ALL requires Chrome's exact origin permission", async () => {
  const harness = loadHarness();
  try {
    harness.grantedOrigins.clear();
    const denied = await harness.context.handleSetMode({ mode: "all" });
    assert.equal(denied.code, "permission_required");
    assert.equal(harness.evaluate("allMode"), false);
    assert.equal(harness.storageState.knapperMode, "off");

    harness.grantedOrigins.add("https://example.test/*");
    const picked = await harness.context.handleSetMode({ mode: "pick" });
    assert.equal(picked.mode, "pick");
    assert.equal(harness.evaluate("session.state"), "selecting");
    assert.equal(harness.storageState.knapperMode, "off");
    await harness.context.setMode("off");
    assert.equal(harness.evaluate("session"), null);
    assert.equal(harness.evaluate("allMode"), false);

    const all = await harness.context.enterAllMode();
    assert.equal(all.mode, "all");
    assert.equal(harness.evaluate("allMode"), true);
    assert.equal(harness.storageState.knapperMode, "all");
    assert.equal(harness.evaluate("session"), null);
    assert.equal(harness.sentNative.some((message) => message.type === "mode" && message.mode === "all"), true);
    assert.equal(harness.actionState.badge, "ALL");
  } finally {
    await harness.cleanup();
  }
});

test("ALL inventories permitted background tabs and serves snapshot/perform/submit requests", async () => {
  const harness = loadHarness();
  const secondTab = { id: 8, url: "https://other.test/account", active: false };
  harness.tabs.set(secondTab.id, secondTab);
  harness.tabList.push(secondTab);
  harness.grantedOrigins.add("https://other.test/*");
  const requests = [];
  secondTab.sendMessage = async (message) => {
    requests.push({ tabId: secondTab.id, message });
    if (message.type === "form_snapshot") return { ok: true, snapshot: { document_id: "document-8", forms: [] } };
    if (message.type === "form_perform") return { ok: true, document_id: message.document_id, results: [{ target_id: "target-1", op: "set_value", status: "verified", value_returned: false }] };
    if (message.type === "form_submit") return { ok: true, document_id: message.document_id };
    return { ok: true };
  };

  try {
    await harness.context.enterAllMode();
    assert.equal(harness.sentNative.some((message) => message.type === "tabs_update"), false);
    assert.deepEqual(harness.sentNative.map((message) => message.type), ["hello", "mode"]);
    await harness.context.onNativeMessage({ type: "tabs_list", request_id: "tabs-1" });
    const listed = harness.sentNative.at(-1);
    assert.equal(listed.type, "tabs_listed");
    assert.deepEqual(JSON.parse(JSON.stringify(listed.tabs)), [
      { tab_id: 7, url: TAB.url, origin: CONTEXT.origin, active: true },
      { tab_id: 8, url: secondTab.url, origin: "https://other.test", active: false }
    ]);

    await harness.context.onNativeMessage({ type: "form_snapshot", request_id: "snapshot-1", tab_id: 8 });
    const snapshot = harness.sentNative.at(-1);
    assert.equal(snapshot.type, "form_snapshotted");
    assert.equal(snapshot.request_id, "snapshot-1");
    assert.deepEqual(JSON.parse(JSON.stringify(snapshot.snapshot)), {
      document_id: "document-8",
      forms: [],
      tab_id: 8,
      url: secondTab.url,
      origin: "https://other.test"
    });

    await harness.context.onNativeMessage({
      type: "form_perform",
      request_id: "perform-1",
      tab_id: 8,
      document_id: "document-8",
      actions: [{ op: "set_value", target_id: "target-1", value: "opaque synthetic", opaque: true }]
    });
    const performed = harness.sentNative.at(-1);
    assert.equal(performed.type, "form_performed");
    assert.equal(performed.results[0].value_returned, false);
    assert.equal(Object.hasOwn(performed.results[0], "value"), false);
    assert.equal(Object.hasOwn(performed, "value"), false);

    await harness.context.onNativeMessage({ type: "form_submit", request_id: "submit-1", tab_id: 8, document_id: "document-8", form_id: "form-1" });
    assert.deepEqual(JSON.parse(JSON.stringify(harness.sentNative.at(-1))), {
      type: "form_submitted", request_id: "submit-1", document_id: "document-8", status: "submitted"
    });
    assert.deepEqual(requests.map(({ message }) => message.type), ["form_snapshot", "form_perform", "form_submit"]);
  } finally {
    await harness.cleanup();
  }
});

test("ALL rejects stale documents and generic click actions without dispatching them", async () => {
  const harness = loadHarness();
  harness.tabs.get(7).sendMessage = async (message) => {
    if (message.type === "form_snapshot") return { ok: true, snapshot: { document_id: "document-7", forms: [] } };
    return { ok: false, code: "stale_document" };
  };
  try {
    await harness.context.enterAllMode();
    await harness.context.onNativeMessage({
      type: "form_perform",
      request_id: "perform-stale",
      tab_id: 7,
      document_id: "old-document",
      actions: []
    });
    assert.deepEqual(JSON.parse(JSON.stringify(harness.sentNative.at(-1))), {
      type: "api_rejected", request_id: "perform-stale", code: "stale_document"
    });

    await harness.context.onNativeMessage({
      type: "form_perform",
      request_id: "perform-click",
      tab_id: 7,
      document_id: "document-7",
      actions: [{ op: "click", target_id: "target-1" }]
    });
    assert.deepEqual(JSON.parse(JSON.stringify(harness.sentNative.at(-1))), {
      type: "api_rejected", request_id: "perform-click", code: "invalid_actions"
    });
  } finally {
    await harness.cleanup();
  }
});

test("ALL tab updates and restore do not send unsolicited Native Message types", async () => {
  const harness = loadHarness();
  try {
    await harness.context.enterAllMode();
    const initialMessages = harness.sentNative.length;
    await harness.context.handleTabUpdate(TAB.id, { status: "complete" }, TAB);
    assert.equal(harness.sentNative.length, initialMessages);
    harness.storageState.knapperMode = "all";
    await harness.context.restoreAllMode();
    assert.equal(harness.sentNative.some((message) => message.type === "tabs_update"), false);
    assert.equal(harness.sentNative.every((message) => ["hello", "mode"].includes(message.type)), true);
  } finally {
    await harness.cleanup();
  }
});

test("stored ALL mode reconnects when the worker is awakened for state", async () => {
  const harness = loadHarness();
  harness.storageState.knapperMode = "all";
  try {
    const state = await harness.context.getState();
    assert.equal(state.mode, "all");
    assert.equal(state.connected, true);
    assert.deepEqual(harness.sentNative.map((message) => message.type), ["hello", "mode"]);
  } finally {
    await harness.cleanup();
  }
});

test("selection arms the fixed session and a fill returns to selecting", async () => {
  const harness = loadHarness();
  try {
    await harness.context.handleActionClick(TAB);
    await harness.context.armSelectedControl({
      type: "selected",
      descriptor: 'input[type="text"][name="email"]',
      origin: CONTEXT.origin,
      url: CONTEXT.url
    }, { tab: TAB });
    assert.equal(harness.evaluate("session.state"), "armed");
    assert.deepEqual(harness.sentNative.map((message) => message.type), ["hello", "mode", "selecting", "arm"]);

    await harness.context.onNativeMessage({
      type: "prepare_fill",
      request_id: "request-1",
      reference: "knapper://personal/identity.emails/signup_default",
      expected_origin: CONTEXT.origin,
      expected_url: CONTEXT.url,
      timeout_ms: 30_000
    });
    assert.equal(harness.evaluate("pending.requestId"), "request-1");
    assert.deepEqual(harness.sentNative.map((message) => message.type), ["hello", "mode", "selecting", "arm", "target_ready"]);

    await harness.context.onNativeMessage({ type: "fill", request_id: "request-1", value: "private value" });
    assert.equal(harness.evaluate("session.state"), "selecting");
    assert.equal(harness.evaluate("pending"), null);
    assert.deepEqual(harness.sentNative.map((message) => message.type), [
      "hello", "mode", "selecting", "arm", "target_ready", "selecting", "filled"
    ]);
    assert.deepEqual(harness.sentTabs.map(({ message }) => message.type), ["select", "check", "fill", "select"]);
    assert.equal(harness.sentTabs.some(({ message }) => Object.hasOwn(message, "value")), true);
  } finally {
    await harness.cleanup();
  }
});

test("prepare_fill is checked against the fixed session without querying activeTab", async () => {
  const harness = loadHarness();
  let queries = 0;
  harness.context.chrome.tabs.query = async () => { queries += 1; return [TAB]; };
  try {
    await harness.context.handleActionClick(TAB);
    await harness.context.armSelectedControl({ type: "selected", origin: CONTEXT.origin, url: CONTEXT.url }, { tab: TAB });
    await harness.context.onNativeMessage({
      type: "prepare_fill",
      request_id: "request-2",
      reference: "knapper://personal/address.home",
      expected_origin: CONTEXT.origin,
      expected_url: "https://example.test/other",
      timeout_ms: 30_000
    });
    assert.equal(queries, 0);
    assert.equal(JSON.stringify(harness.sentNative.at(-1)), JSON.stringify({ type: "target_rejected", request_id: "request-2", code: "not_accepting" }));
  } finally {
    await harness.cleanup();
  }
});

test("navigation and tab close turn the session off", async () => {
  const harness = loadHarness();
  try {
    await harness.context.handleActionClick(TAB);
    for (const listener of harness.context.chrome.tabs.onUpdated.listeners) listener(TAB.id, { status: "loading" });
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(harness.evaluate("session"), null);
    assert.equal(harness.disconnects, 1);

    await harness.context.handleActionClick(TAB);
    for (const listener of harness.context.chrome.tabs.onRemoved.listeners) listener(TAB.id);
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(harness.evaluate("session"), null);
    assert.equal(harness.disconnects, 2);
  } finally {
    await harness.cleanup();
  }
});

test("development key stays bound to the native host allowlist", () => {
  const manifest = JSON.parse(readFileSync(new URL("./manifest.json", import.meta.url), "utf8"));
  const nativeManifest = JSON.parse(readFileSync(new URL("./native-host-manifest.example.json", import.meta.url), "utf8"));
  const digest = createHash("sha256").update(Buffer.from(manifest.key, "base64")).digest().subarray(0, 16);
  let extensionId = "";
  for (const byte of digest) extensionId += String.fromCharCode(97 + (byte >> 4), 97 + (byte & 15));
  assert.deepEqual(nativeManifest.allowed_origins, [`chrome-extension://${extensionId}/`]);
});
