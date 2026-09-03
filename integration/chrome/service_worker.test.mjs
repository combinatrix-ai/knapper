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
  const onRemoved = eventTarget();
  const onUpdated = eventTarget();
  const sentNative = [];
  const sentTabs = [];
  const tabs = new Map();
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
        onMessage: runtimeMessages
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
        query: async () => [],
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
    evaluate(source) { return vm.runInContext(source, context); },
    get disconnects() { return disconnects; },
    async cleanup() { await context.disableSession(); }
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

  class Event {
    constructor(type) { this.type = type; }
  }

  const document = {
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
    }
  };
  const location = { origin: "https://example.test", href: "https://example.test/form" };
  const context = vm.createContext({
    Element,
    HTMLInputElement,
    HTMLTextAreaElement,
    Event,
    document,
    location,
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
    textarea() { return new HTMLTextAreaElement(); }
  };
}

const TAB = { id: 7, url: "https://example.test/form" };
const CONTEXT = { tabId: 7, url: TAB.url, origin: "https://example.test" };

test("manifest stays on activeTab scripting without debugger or broad host access", () => {
  const manifest = JSON.parse(readFileSync(new URL("./manifest.json", import.meta.url), "utf8"));
  assert.deepEqual(manifest.permissions, ["activeTab", "nativeMessaging", "scripting"]);
  assert.equal(manifest.host_permissions, undefined);
  assert.equal(manifest.action.default_popup, undefined);
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

test("the extension action starts one tab-bound selecting session", async () => {
  const harness = loadHarness();
  try {
    await harness.context.handleActionClick(TAB);
    assert.equal(harness.evaluate("session.state"), "selecting");
    assert.equal(harness.evaluate("session.tabId"), TAB.id);
    assert.equal(harness.evaluate("session.url"), TAB.url);
    assert.deepEqual(harness.sentNative.map((message) => message.type), ["hello", "selecting"]);
    assert.equal(JSON.stringify(harness.sentTabs), JSON.stringify([{ tabId: TAB.id, message: { type: "select" } }]));
    assert.equal(JSON.stringify(harness.executeScriptCalls.map((call) => call.files)), JSON.stringify([["content_script.js"]]));
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
    assert.deepEqual(harness.sentNative.map((message) => message.type), ["hello", "selecting", "arm"]);

    await harness.context.onNativeMessage({
      type: "prepare_fill",
      request_id: "request-1",
      reference: "knapper://personal/identity.emails/signup_default",
      expected_origin: CONTEXT.origin,
      expected_url: CONTEXT.url,
      timeout_ms: 30_000
    });
    assert.equal(harness.evaluate("pending.requestId"), "request-1");
    assert.deepEqual(harness.sentNative.map((message) => message.type), ["hello", "selecting", "arm", "target_ready"]);

    await harness.context.onNativeMessage({ type: "fill", request_id: "request-1", value: "private value" });
    assert.equal(harness.evaluate("session.state"), "selecting");
    assert.equal(harness.evaluate("pending"), null);
    assert.deepEqual(harness.sentNative.map((message) => message.type), [
      "hello", "selecting", "arm", "target_ready", "selecting", "filled"
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
