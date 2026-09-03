/* global chrome */

// This file is injected once per active document and remains in Chrome's
// isolated world. The page never receives a selector, marker, or element
// reference; only this content-script closure can access the selected control.
if (!globalThis.__knapperFillContentScript) {
  const state = { selected: null, cancelSelection: null };
  const TEXT_TYPES = new Set(["text", "email", "tel", "search", "url"]);

  function isEligibleTextControl(element) {
    if (!element || !(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement)) return false;
    if (!element.isConnected || element.isContentEditable || element.matches("[contenteditable]")) return false;
    if (element.disabled || element.readOnly) return false;
    const type = element instanceof HTMLInputElement ? element.type.toLowerCase() : "textarea";
    if (type !== "textarea" && !TEXT_TYPES.has(type)) return false;
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && Number(style.opacity) !== 0 && rect.width > 0 && rect.height > 0;
  }

  function describe(element) {
    const tag = element.tagName.toLowerCase();
    const type = element instanceof HTMLInputElement ? `[type="${element.type.toLowerCase()}"]` : "";
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
      notify({
        type: "selected",
        descriptor: describe(element),
        origin: location.origin,
        url: location.href
      });
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
    return false;
  });

  globalThis.__knapperFillContentScript = true;
}
