/* global chrome */

// This file is injected once per document and remains in Chrome's isolated
// world. The page never receives selectors, markers, values, or element
// references. Targets returned by a snapshot are only handles into the maps
// below and are invalidated by DOM mutations or navigation.
if (!globalThis.__knapperFillContentScript) {
  const PICK_TEXT_TYPES = new Set(["text", "email", "tel", "search", "url"]);
  const UNSUPPORTED_INPUT_TYPES = new Set(["submit", "button", "reset", "image", "file"]);
  const state = {
    selected: null,
    cancelSelection: null,
    targets: new Map(),
    forms: new Map(),
    opaqueTargets: new WeakSet(),
    hasOpaqueValue: false
  };
  const documentId = typeof crypto?.randomUUID === "function" ? crypto.randomUUID() : `${Date.now()}-${Math.random()}`;

  function randomId(prefix) {
    const suffix = typeof crypto?.randomUUID === "function" ? crypto.randomUUID() : `${Date.now()}-${Math.random()}`;
    return `${prefix}-${suffix}`;
  }

  function inputType(element) {
    return element instanceof HTMLInputElement ? element.type.toLowerCase() : "textarea";
  }

  function isEligibleTextControl(element) {
    if (!element || !(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement)) return false;
    if (!element.isConnected || element.isContentEditable || element.matches("[contenteditable]")) return false;
    if (element.disabled || element.readOnly) return false;
    const type = inputType(element);
    if (element instanceof HTMLInputElement && !PICK_TEXT_TYPES.has(type)) return false;
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && Number(style.opacity) !== 0 && rect.width > 0 && rect.height > 0;
  }

  function supportedControl(element) {
    if (!element || !(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement || element instanceof HTMLSelectElement)) return false;
    if (element instanceof HTMLInputElement && UNSUPPORTED_INPUT_TYPES.has(inputType(element))) return false;
    return element.isConnected;
  }

  function labelFor(element) {
    try {
      const label = element.labels?.[0];
      const text = label?.textContent?.replace(/\s+/g, " ").trim();
      return text ? text.slice(0, 240) : undefined;
    } catch (_) {
      return undefined;
    }
  }

  function descriptor(element) {
    const tag = element.tagName.toLowerCase();
    const type = element instanceof HTMLInputElement ? `[type="${inputType(element)}"]` : "";
    const name = element.getAttribute("name");
    const id = element.id;
    if (name) return `${tag}${type}[name="${name.slice(0, 80)}"]`;
    if (id) return `${tag}${type}#${id.slice(0, 80)}`;
    return `${tag}${type}`;
  }

  function notify(message) {
    try { void chrome.runtime.sendMessage(message); } catch (_) {}
  }

  function clearSelection() {
    if (state.cancelSelection) state.cancelSelection(false);
    state.selected = null;
  }

  function beginSelection() {
    clearSelection();
    let highlighted = null;
    let oldOutline = "";
    let oldOutlineOffset = "";

    function restoreHighlight() {
      if (!highlighted) return;
      highlighted.style.outline = oldOutline;
      highlighted.style.outlineOffset = oldOutlineOffset;
      highlighted = null;
    }
    function cleanup() {
      restoreHighlight();
      document.removeEventListener("pointerover", onPointerOver, true);
      document.removeEventListener("pointerout", onPointerOut, true);
      document.removeEventListener("click", onClick, true);
      document.removeEventListener("keydown", onKeyDown, true);
      clearTimeout(timer);
      state.cancelSelection = null;
    }
    function candidate(event) {
      const target = event.target;
      return target instanceof Element && isEligibleTextControl(target) ? target : null;
    }
    function onPointerOver(event) {
      const element = candidate(event);
      if (!element || element === highlighted) return;
      restoreHighlight();
      highlighted = element;
      oldOutline = element.style.outline;
      oldOutlineOffset = element.style.outlineOffset;
      element.style.outline = "3px solid #18794e";
      element.style.outlineOffset = "2px";
    }
    function onPointerOut(event) {
      if (event.target === highlighted) restoreHighlight();
    }
    function onClick(event) {
      const element = candidate(event);
      if (!element) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      cleanup();
      state.selected = element;
      element.focus({ preventScroll: true });
      notify({ type: "selected", descriptor: descriptor(element), origin: location.origin, url: location.href });
    }
    function onKeyDown(event) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      cleanup();
      state.selected = null;
      notify({ type: "selection_cancelled" });
    }

    document.addEventListener("pointerover", onPointerOver, true);
    document.addEventListener("pointerout", onPointerOut, true);
    document.addEventListener("click", onClick, true);
    document.addEventListener("keydown", onKeyDown, true);
    const timer = setTimeout(() => {
      cleanup();
      state.selected = null;
      notify({ type: "selection_timeout" });
    }, 3 * 60 * 1000);
    state.cancelSelection = (notifyCancel = true) => {
      cleanup();
      state.selected = null;
      if (notifyCancel) notify({ type: "selection_cancelled" });
    };
    return { ok: true };
  }

  function checkSelection(message) {
    if (location.origin !== message.expected_origin || location.href !== message.expected_url) return { ok: false, code: "navigation" };
    return state.selected && isEligibleTextControl(state.selected) ? { ok: true } : { ok: false, code: "target_missing" };
  }

  function fillSelection(message) {
    const checked = checkSelection(message);
    if (!checked.ok) return checked;
    if (typeof message.value !== "string" || message.value.length > 1024 * 1024 - 4096) return { ok: false, code: "value_invalid" };
    const element = state.selected;
    const prototype = element instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
    if (!setter) return { ok: false, code: "script_failed" };
    setter.call(element, message.value);
    element.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
    element.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
    if (!element.isConnected || element.value !== message.value) return { ok: false, code: "value_not_retained" };
    state.selected = null;
    return { ok: true };
  }

  function clearTargets() {
    state.targets.clear();
    state.forms.clear();
  }

  function controlMetadata(element, targetId, formId) {
    const tag = element.tagName.toLowerCase();
    const type = element instanceof HTMLInputElement ? inputType(element) : tag === "textarea" ? "textarea" : undefined;
    const kind = element instanceof HTMLSelectElement ? "select" : element instanceof HTMLInputElement && ["checkbox", "radio"].includes(type) ? type : "text";
    const metadata = {
      target_id: targetId,
      form_id: formId,
      tag,
      kind,
      type,
      id_attr: element.id || undefined,
      name: element.getAttribute("name") || undefined,
      label: labelFor(element),
      autocomplete: element.getAttribute("autocomplete") || undefined,
      required: Boolean(element.required),
      disabled: Boolean(element.disabled),
      read_only: Boolean(element.readOnly),
      checked: ["checkbox", "radio"].includes(kind) ? Boolean(element.checked) : undefined,
      value_opaque: state.hasOpaqueValue || state.opaqueTargets.has(element)
    };
    if (kind === "select") {
      metadata.options = Array.from(element.options).map((option) => ({
        value: String(option.value).slice(0, 512),
        label: String(option.label || option.textContent || "").slice(0, 512),
        selected: Boolean(option.selected)
      }));
    } else if (!state.hasOpaqueValue && !state.opaqueTargets.has(element) && type !== "password" && type !== "hidden") {
      metadata.current_value = String(element.value ?? "").slice(0, 4096);
    }
    return metadata;
  }

  function snapshot() {
    clearTargets();
    const forms = [];
    const formElements = Array.from(document.querySelectorAll("form"));
    for (const form of formElements) {
      const formId = randomId("form");
      state.forms.set(formId, form);
      const controls = [];
      for (const element of form.querySelectorAll("input, textarea, select")) {
        if (!supportedControl(element)) continue;
        const targetId = randomId("target");
        state.targets.set(targetId, element);
        controls.push(controlMetadata(element, targetId, formId));
      }
      forms.push({
        form_id: formId,
        method: String(form.method || "get").toLowerCase(),
        action: String(form.action || location.href).slice(0, 4096),
        controls
      });
    }
    const formControls = Array.from(document.querySelectorAll("input, textarea, select")).filter((element) => supportedControl(element) && !element.form);
    if (formControls.length > 0) {
      // A pseudo form keeps the snapshot schema uniform. It is intentionally
      // not added to state.forms, so form_submit can never submit it.
      const pseudoFormId = randomId("formless");
      const controls = [];
      for (const element of formControls) {
        const targetId = randomId("target");
        state.targets.set(targetId, element);
        controls.push(controlMetadata(element, targetId, pseudoFormId));
      }
      forms.push({ form_id: pseudoFormId, controls });
    }
    return { ok: true, snapshot: { document_id: documentId, forms } };
  }

  function staleDocument(message) {
    return typeof message.document_id !== "string" || message.document_id !== documentId ? { ok: false, code: "stale_document" } : null;
  }

  function dispatchValue(element, value) {
    if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement)) return { ok: false, code: "wrong_control_kind" };
    if (element instanceof HTMLInputElement && ["checkbox", "radio"].includes(inputType(element))) return { ok: false, code: "wrong_control_kind" };
    const prototype = element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
    if (!setter) return { ok: false, code: "unsupported_control" };
    setter.call(element, String(value));
    element.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
    element.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
    return element.value === String(value) ? { ok: true } : { ok: false, code: "value_not_retained" };
  }

  function perform(message) {
    const stale = staleDocument(message);
    if (stale) return stale;
    if (!Array.isArray(message.actions) || message.actions.length > 128) return { ok: false, code: "invalid_actions" };
    const results = [];
    for (const action of message.actions) {
      if (!action || typeof action.target_id !== "string" || typeof action.op !== "string") {
        results.push({ target_id: action?.target_id || "invalid", op: action?.op || "invalid", status: "rejected", code: "invalid_action" });
        continue;
      }
      const element = state.targets.get(action.target_id);
      if (!element || !element.isConnected || !supportedControl(element)) {
        results.push({ target_id: action.target_id, op: action.op, status: "rejected", code: "stale_target" });
        continue;
      }
      let result = { ok: false, code: "unsupported_operation" };
      if (action.op === "set_value" || action.op === "set_from") {
        if (typeof action.value !== "string") result = { ok: false, code: "invalid_value" };
        else {
          result = dispatchValue(element, action.value);
          if (result.ok && (action.opaque === true || action.op === "set_from")) {
            state.opaqueTargets.add(element);
            state.hasOpaqueValue = true;
          }
        }
      } else if (action.op === "select_option" && element instanceof HTMLSelectElement) {
        const index = typeof action.value === "string" ? Array.from(element.options).findIndex((option) => option.value === action.value) : -1;
        if (index < 0 || index >= element.options.length || element.options[index].disabled) result = { ok: false, code: "option_not_found" };
        else {
          element.selectedIndex = index;
          element.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
          element.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
          result = { ok: true };
        }
      } else if (action.op === "set_checked" && element instanceof HTMLInputElement && ["checkbox", "radio"].includes(inputType(element))) {
        if (typeof action.checked !== "boolean") result = { ok: false, code: "invalid_checked" };
        else {
          element.checked = action.checked;
          element.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
          element.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
          result = { ok: true };
        }
      }
      results.push({ target_id: action.target_id, op: action.op, status: result.ok ? "verified" : "rejected", ...(result.code ? { code: result.code } : {}), value_returned: false });
    }
    return { ok: true, document_id: documentId, results };
  }

  function submit(message) {
    const stale = staleDocument(message);
    if (stale) return stale;
    const form = state.forms.get(message.form_id);
    if (!form || !form.isConnected) return { ok: false, code: "stale_form" };
    try {
      form.requestSubmit();
      return { ok: true, document_id: documentId };
    } catch (_) {
      return { ok: false, code: "submit_failed" };
    }
  }

  if (typeof MutationObserver === "function") {
    new MutationObserver(() => clearTargets()).observe(document.documentElement, { subtree: true, childList: true, attributes: true });
  }

  chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    if (!message || typeof message.type !== "string") return false;
    if (message.type === "select") {
      sendResponse(beginSelection());
      return false;
    }
    if (message.type === "cancel") {
      clearSelection();
      sendResponse({ ok: true });
      return false;
    }
    if (message.type === "check") {
      sendResponse(checkSelection(message));
      return false;
    }
    if (message.type === "fill") {
      sendResponse(fillSelection(message));
      return false;
    }
    if (message.type === "form_snapshot") {
      sendResponse(snapshot());
      return false;
    }
    if (message.type === "form_perform") {
      sendResponse(perform(message));
      return false;
    }
    if (message.type === "form_submit") {
      sendResponse(submit(message));
      return false;
    }
    return false;
  });

  globalThis.__knapperFillContentScript = true;
}
