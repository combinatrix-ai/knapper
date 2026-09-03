#!/usr/bin/env node

import { createServer } from "node:http";
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import assert from "node:assert/strict";

const DEFAULT_PORT = 48173;
const DEFAULT_TIMEOUT_SECONDS = 120;
const FIXTURE_HOST = "knapper-e2e.localhost";
const FIXTURE_TITLE = "Knapper Chrome bridge fixture";
const RERENDER_VALUE = "fixture-rerender-now";
const RESET_VALUE = "fixture-reset";
const DUPLICATE_VALUE = "fixture-create-duplicates";
const RECOVERED_VALUE = "fixture-recovered-ok";
const BLOCKED_VALUE = "fixture-must-not-write";
const BLOCKED_TRIGGER_VALUE = "fixture-trigger-must-not-write";
const MATRIX_NOTES = "Fixed fixture note\nwith a second line.";
const MATRIX_EMAIL = "e2e@example.test";
const MATRIX_TELEPHONE = "+81-3-1234-5678";
const MATRIX_QUANTITY = "42";
const MATRIX_PASSWORD = "fixture-password-value";
const MATRIX_BIRTH_DATE = "2004-02-29";
const MAX_OUTPUT_BYTES = 2 * 1024 * 1024;

export function parseArgs(argv) {
  const options = {
    host: FIXTURE_HOST,
    port: DEFAULT_PORT,
    timeoutSeconds: DEFAULT_TIMEOUT_SECONDS,
    client: process.env.KNAPPER_CHROME_CLIENT || "knapper-chrome-client"
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const value = argv[index + 1];
    if (argument === "--host" && value) {
      options.host = value;
      index += 1;
    } else if (argument === "--port" && value) {
      options.port = Number(value);
      index += 1;
    } else if (argument === "--timeout" && value) {
      options.timeoutSeconds = Number(value);
      index += 1;
    } else if (argument === "--client" && value) {
      options.client = value;
      index += 1;
    } else {
      throw new Error(`unknown or incomplete argument: ${argument}`);
    }
  }
  if (!Number.isInteger(options.port) || options.port < 1024 || options.port > 65535) {
    throw new Error("--port must be an integer between 1024 and 65535");
  }
  if (!new Set([FIXTURE_HOST, "127.0.0.1"]).has(options.host)) {
    throw new Error(`--host must be ${FIXTURE_HOST} or 127.0.0.1`);
  }
  if (!Number.isFinite(options.timeoutSeconds) || options.timeoutSeconds < 5 || options.timeoutSeconds > 600) {
    throw new Error("--timeout must be between 5 and 600 seconds");
  }
  if (typeof options.client !== "string" || options.client.length === 0) {
    throw new Error("--client must name knapper-chrome-client");
  }
  return options;
}

export function parseClientOutput(stdout) {
  const lines = stdout.split(/\r?\n/).filter((line) => line.length > 0);
  if (lines.length !== 1) throw new Error(`client returned ${lines.length} JSON lines`);
  let response;
  try {
    response = JSON.parse(lines[0]);
  } catch (_) {
    throw new Error("client did not return valid JSON");
  }
  if (!response || typeof response !== "object" || typeof response.status !== "string") {
    throw new Error("client returned an invalid response shape");
  }
  return response;
}

export function controlsFrom(snapshot) {
  if (!snapshot || !Array.isArray(snapshot.forms)) throw new Error("snapshot has no forms");
  return snapshot.forms.flatMap((form) => Array.isArray(form.controls) ? form.controls : []);
}

export function uniqueControl(snapshot, name) {
  const matches = controlsFrom(snapshot).filter((control) => control.name === name);
  if (matches.length !== 1) throw new Error(`expected one ${name} control, found ${matches.length}`);
  return matches[0];
}

export function matchingControl(snapshot, predicate, description) {
  const matches = controlsFrom(snapshot).filter(predicate);
  if (matches.length !== 1) throw new Error(`expected one ${description} control, found ${matches.length}`);
  return matches[0];
}

export function fixtureTokenMatches(snapshot, runToken) {
  const matches = controlsFrom(snapshot).filter((control) => control.name === "fixture-run-token");
  return matches.length === 1 && matches[0].current_value === runToken;
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function runClient(client, request, timeoutSeconds) {
  return new Promise((resolve, reject) => {
    const child = spawn(client, ["api", "--timeout", String(Math.min(timeoutSeconds, 120))], {
      stdio: ["pipe", "pipe", "pipe"]
    });
    let stdout = "";
    let stderr = "";
    let outputBytes = 0;
    let settled = false;
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      finish(new Error("knapper-chrome-client timed out"));
    }, Math.ceil(Math.min(timeoutSeconds, 120) * 1000) + 2_000);

    function finish(error, result) {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) reject(error);
      else resolve(result);
    }

    child.on("error", (error) => finish(new Error(`could not start ${client}: ${error.message}`)));
    child.stdout.on("data", (chunk) => {
      outputBytes += chunk.length;
      if (outputBytes > MAX_OUTPUT_BYTES) {
        child.kill("SIGKILL");
        finish(new Error("client output exceeded the test limit"));
        return;
      }
      stdout += chunk.toString("utf8");
    });
    child.stderr.on("data", (chunk) => {
      outputBytes += chunk.length;
      if (outputBytes > MAX_OUTPUT_BYTES) {
        child.kill("SIGKILL");
        finish(new Error("client output exceeded the test limit"));
        return;
      }
      stderr += chunk.toString("utf8");
    });
    child.on("close", (code, signal) => {
      if (signal) return finish(new Error(`client stopped with ${signal}`));
      let response;
      try {
        response = parseClientOutput(stdout);
      } catch (error) {
        return finish(new Error(`${error.message}; stderr=${stderr.trim() || "<empty>"}`));
      }
      finish(null, { code, response, stderr });
    });
    child.stdin.end(JSON.stringify(request));
  });
}

async function apiOk(client, request, timeoutSeconds) {
  const result = await runClient(client, request, timeoutSeconds);
  if (result.code !== 0 || result.response.status !== "ok" || !result.response.result) {
    throw new Error(`API ${request.op} failed with ${result.response.code || `exit ${result.code}`}`);
  }
  return result.response.result;
}

async function snapshot(client, tabId, timeoutSeconds) {
  const result = await apiOk(client, { op: "form_snapshot", tab_id: tabId }, timeoutSeconds);
  if (result.kind !== "form_snapshot" || !result.snapshot) throw new Error("invalid form_snapshot result");
  return result.snapshot;
}

async function perform(client, tabId, documentId, actions, timeoutSeconds) {
  const result = await apiOk(client, {
    op: "form_perform",
    tab_id: tabId,
    document_id: documentId,
    actions
  }, timeoutSeconds);
  if (result.kind !== "form_perform" || !Array.isArray(result.results)) {
    throw new Error("invalid form_perform result");
  }
  return result;
}

function assertVerified(result, originalTargetId) {
  if (result.results.length !== 1) throw new Error("expected one action result");
  const [action] = result.results;
  if (action.target_id !== originalTargetId || action.status !== "verified") {
    throw new Error("action was not verified with its original target_id");
  }
}

function assertAllVerified(result, originalTargetIds) {
  assert.equal(result.results.length, originalTargetIds.length, "unexpected action result count");
  for (const [index, action] of result.results.entries()) {
    assert.equal(action.target_id, originalTargetIds[index], "action result changed its target_id");
    assert.equal(action.status, "verified", `action ${index} was not verified`);
    assert.equal(action.value_returned, false, `action ${index} returned a value`);
  }
}

function selectedOption(control, value) {
  return control.options?.find((option) => option.value === value);
}

function assertInputMatrixMetadata(snapshot) {
  const notes = uniqueControl(snapshot, "notes");
  assert.equal(notes.tag, "textarea");
  assert.equal(notes.kind, "text");
  assert.equal(notes.type, "textarea");
  assert.equal(notes.current_value, "");

  for (const [name, type] of [["email", "email"], ["telephone", "tel"], ["quantity", "number"], ["birth-date", "date"]]) {
    const control = uniqueControl(snapshot, name);
    assert.equal(control.tag, "input");
    assert.equal(control.kind, "text");
    assert.equal(control.type, type);
    assert.equal(typeof control.current_value, "string");
  }

  const password = uniqueControl(snapshot, "password");
  assert.equal(password.tag, "input");
  assert.equal(password.kind, "text");
  assert.equal(password.type, "password");
  assert.equal(Object.hasOwn(password, "current_value"), false, "password value leaked in snapshot");

  const country = uniqueControl(snapshot, "country");
  assert.equal(country.tag, "select");
  assert.equal(country.kind, "select");
  assert.deepEqual(country.options, [
    { value: "", label: "Choose a country", selected: true },
    { value: "jp", label: "Japan", selected: false },
    { value: "us", label: "United States", selected: false }
  ]);

  const terms = uniqueControl(snapshot, "terms");
  assert.equal(terms.tag, "input");
  assert.equal(terms.kind, "checkbox");
  assert.equal(terms.type, "checkbox");
  assert.equal(terms.checked, false);

  const emailRadio = matchingControl(snapshot,
    (control) => control.name === "contact-preference" && control.label === "Contact by email",
    "email contact-preference");
  const phoneRadio = matchingControl(snapshot,
    (control) => control.name === "contact-preference" && control.label === "Contact by phone",
    "phone contact-preference");
  assert.equal(emailRadio.kind, "radio");
  assert.equal(emailRadio.type, "radio");
  assert.equal(emailRadio.checked, false);
  assert.equal(phoneRadio.kind, "radio");
  assert.equal(phoneRadio.type, "radio");
  assert.equal(phoneRadio.checked, false);

  const year = uniqueControl(snapshot, "dob-year");
  const month = uniqueControl(snapshot, "dob-month");
  const day = uniqueControl(snapshot, "dob-day");
  for (const control of [year, month, day]) {
    assert.equal(control.tag, "select");
    assert.equal(control.kind, "select");
    assert.equal(Object.hasOwn(control, "current_value"), false, "select exposed current_value");
  }
  assert.deepEqual(year.options, [
    { value: "", label: "Choose year", selected: true },
    { value: "2003", label: "2003", selected: false },
    { value: "2004", label: "2004", selected: false }
  ]);
  assert.deepEqual(month.options, [
    { value: "", label: "Choose month", selected: true },
    { value: "01", label: "January", selected: false },
    { value: "02", label: "February", selected: false }
  ]);
  assert.equal(day.options.length, 32, "initial DOB day options should include days 1 through 31");
  assert.equal(day.options.at(-1).value, "31");
  assert.equal(uniqueControl(snapshot, "dob-change-count").current_value, "1");
}

function assertInputMatrixValues(snapshot) {
  assert.equal(uniqueControl(snapshot, "notes").current_value, MATRIX_NOTES);
  assert.equal(uniqueControl(snapshot, "email").current_value, MATRIX_EMAIL);
  assert.equal(uniqueControl(snapshot, "telephone").current_value, MATRIX_TELEPHONE);
  assert.equal(uniqueControl(snapshot, "quantity").current_value, MATRIX_QUANTITY);
  assert.equal(uniqueControl(snapshot, "birth-date").current_value, MATRIX_BIRTH_DATE);
  assert.equal(Object.hasOwn(uniqueControl(snapshot, "password"), "current_value"), false, "password value leaked after write");

  const country = uniqueControl(snapshot, "country");
  assert.equal(selectedOption(country, "jp")?.selected, true);
  assert.equal(selectedOption(country, "")?.selected, false);

  assert.equal(uniqueControl(snapshot, "terms").checked, true);
  assert.equal(matchingControl(snapshot,
    (control) => control.name === "contact-preference" && control.label === "Contact by email",
    "email contact-preference").checked, true);
  assert.equal(matchingControl(snapshot,
    (control) => control.name === "contact-preference" && control.label === "Contact by phone",
    "phone contact-preference").checked, false);
  assert.equal(uniqueControl(snapshot, "dob-change-count").current_value, "1");
}

async function waitForFixtureTab(client, origin, fixtureUrl, timeoutSeconds) {
  const deadline = Date.now() + timeoutSeconds * 1000;
  let lastProblem = "bridge not ready";
  while (Date.now() < deadline) {
    try {
      const result = await apiOk(client, { op: "tabs_list", origin }, 5);
      if (result.kind !== "tabs_list" || !Array.isArray(result.tabs)) {
        lastProblem = "invalid tabs_list response";
      } else {
        const matches = result.tabs.filter((tab) => tab.url === fixtureUrl);
        if (matches.length === 1) return matches[0];
        lastProblem = matches.length === 0
          ? "fixture tab is not open or this origin is not permitted"
          : "more than one fixture tab is open";
      }
    } catch (error) {
      lastProblem = error.message;
    }
    await delay(500);
  }
  throw new Error(`${lastProblem}. Open ${fixtureUrl} in Chrome, click Knapper Fill, and choose ALL for this origin.`);
}

async function waitForFixtureReady(client, tabId, runToken, timeoutSeconds) {
  const deadline = Date.now() + timeoutSeconds * 1000;
  let lastProblem = "fixture has not loaded the current run token";
  while (Date.now() < deadline) {
    try {
      const current = await snapshot(client, tabId, 5);
      if (fixtureTokenMatches(current, runToken)) return current;
      lastProblem = "fixture is still showing a previous run token";
    } catch (error) {
      lastProblem = error.message;
    }
    await delay(250);
  }
  throw new Error(`${lastProblem}. Keep the permitted fixture tab open until it reloads.`);
}

async function serveFixture(port) {
  const fixturePath = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/bridge-fixture.html");
  const runToken = randomUUID();
  const body = Buffer.from((await readFile(fixturePath, "utf8")).replaceAll("__KNAPPER_RUN_TOKEN__", runToken));
  const server = createServer((request, response) => {
    const requestUrl = new URL(request.url || "/", `http://127.0.0.1:${port}`);
    if (requestUrl.pathname === "/run-token") {
      response.writeHead(200, {
        "Content-Type": "text/plain; charset=utf-8",
        "Content-Length": Buffer.byteLength(runToken),
        "Cache-Control": "no-store",
        "X-Content-Type-Options": "nosniff"
      });
      response.end(runToken);
      return;
    }
    if (requestUrl.pathname === "/favicon.ico") {
      response.writeHead(204, { "Cache-Control": "no-store" });
      response.end();
      return;
    }
    if (requestUrl.pathname !== "/" && requestUrl.pathname !== "/bridge-fixture.html") {
      response.writeHead(404, { "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" });
      response.end("not found\n");
      return;
    }
    response.writeHead(200, {
      "Content-Type": "text/html; charset=utf-8",
      "Content-Length": body.length,
      "Cache-Control": "no-store",
      "Content-Security-Policy": "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; form-action 'none'; base-uri 'none'",
      "X-Content-Type-Options": "nosniff"
    });
    response.end(body);
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", resolve);
  });
  return { server, runToken };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const origin = `http://${options.host}:${options.port}`;
  const fixtureUrl = `${origin}/bridge-fixture.html`;
  const { server, runToken } = await serveFixture(options.port);
  console.log(`Fixture: ${fixtureUrl}`);
  console.log("Waiting for one permitted Chrome tab. Open the URL and choose Knapper Fill > ALL once.");

  try {
    const tab = await waitForFixtureTab(options.client, origin, fixtureUrl, options.timeoutSeconds);
    console.log(`PASS tab discovery (${tab.tab_id})`);

    let current = await waitForFixtureReady(options.client, tab.tab_id, runToken, options.timeoutSeconds);
    const resetTrigger = uniqueControl(current, "ambiguity-trigger");
    const reset = await perform(options.client, tab.tab_id, current.document_id, [{
      op: "set_value",
      target_id: resetTrigger.target_id,
      value: RESET_VALUE
    }], options.timeoutSeconds);
    assertVerified(reset, resetTrigger.target_id);
    await delay(50);

    const baseline = await snapshot(options.client, tab.tab_id, options.timeoutSeconds);
    assertInputMatrixMetadata(baseline);
    const matrixEmailRadio = matchingControl(baseline,
      (control) => control.name === "contact-preference" && control.label === "Contact by email",
      "email contact-preference");
    const matrixPhoneRadio = matchingControl(baseline,
      (control) => control.name === "contact-preference" && control.label === "Contact by phone",
      "phone contact-preference");
    const matrixActions = [
      { op: "set_value", target_id: uniqueControl(baseline, "notes").target_id, value: MATRIX_NOTES },
      { op: "set_value", target_id: uniqueControl(baseline, "email").target_id, value: MATRIX_EMAIL },
      { op: "set_value", target_id: uniqueControl(baseline, "telephone").target_id, value: MATRIX_TELEPHONE },
      { op: "set_value", target_id: uniqueControl(baseline, "quantity").target_id, value: MATRIX_QUANTITY },
      { op: "set_value", target_id: uniqueControl(baseline, "password").target_id, value: MATRIX_PASSWORD },
      { op: "set_value", target_id: uniqueControl(baseline, "birth-date").target_id, value: MATRIX_BIRTH_DATE },
      { op: "select_option", target_id: uniqueControl(baseline, "country").target_id, value: "jp" },
      { op: "set_checked", target_id: uniqueControl(baseline, "terms").target_id, checked: true },
      { op: "set_checked", target_id: matrixEmailRadio.target_id, checked: true },
      { op: "set_checked", target_id: matrixPhoneRadio.target_id, checked: false }
    ];
    const matrixResult = await perform(options.client, tab.tab_id, baseline.document_id, matrixActions, options.timeoutSeconds);
    assertAllVerified(matrixResult, matrixActions.map((action) => action.target_id));
    current = await snapshot(options.client, tab.tab_id, options.timeoutSeconds);
    assertInputMatrixValues(current);
    console.log("PASS live input matrix: textarea, email, tel, number, date, password privacy, select, checkbox, radio");

    const dobYear = uniqueControl(current, "dob-year");
    const yearResult = await perform(options.client, tab.tab_id, current.document_id, [{
      op: "select_option",
      target_id: dobYear.target_id,
      value: "2004"
    }], options.timeoutSeconds);
    assertVerified(yearResult, dobYear.target_id);
    await delay(50);
    current = await snapshot(options.client, tab.tab_id, options.timeoutSeconds);
    assert.equal(selectedOption(uniqueControl(current, "dob-year"), "2004")?.selected, true);
    assert.equal(uniqueControl(current, "dob-change-count").current_value, "2");

    const dobMonth = uniqueControl(current, "dob-month");
    const monthResult = await perform(options.client, tab.tab_id, current.document_id, [{
      op: "select_option",
      target_id: dobMonth.target_id,
      value: "02"
    }], options.timeoutSeconds);
    assertVerified(monthResult, dobMonth.target_id);
    await delay(50);
    current = await snapshot(options.client, tab.tab_id, options.timeoutSeconds);
    const leapDayOptions = uniqueControl(current, "dob-day").options;
    assert.equal(uniqueControl(current, "dob-change-count").current_value, "3");
    assert.equal(selectedOption(uniqueControl(current, "dob-month"), "02")?.selected, true);
    assert.equal(leapDayOptions.length, 30, "2004-02 should expose 29 days plus the placeholder");
    assert.equal(leapDayOptions.at(-1).value, "29");
    assert.equal(selectedOption(uniqueControl(current, "dob-day"), "29")?.selected, false);

    const dobDay = uniqueControl(current, "dob-day");
    const dayResult = await perform(options.client, tab.tab_id, current.document_id, [{
      op: "select_option",
      target_id: dobDay.target_id,
      value: "29"
    }], options.timeoutSeconds);
    assertVerified(dayResult, dobDay.target_id);
    current = await snapshot(options.client, tab.tab_id, options.timeoutSeconds);
    assert.equal(selectedOption(uniqueControl(current, "dob-day"), "29")?.selected, true);

    const nonLeapYear = uniqueControl(current, "dob-year");
    const nonLeapResult = await perform(options.client, tab.tab_id, current.document_id, [{
      op: "select_option",
      target_id: nonLeapYear.target_id,
      value: "2003"
    }], options.timeoutSeconds);
    assertVerified(nonLeapResult, nonLeapYear.target_id);
    await delay(50);
    current = await snapshot(options.client, tab.tab_id, options.timeoutSeconds);
    const nonLeapDay = uniqueControl(current, "dob-day");
    assert.equal(uniqueControl(current, "dob-change-count").current_value, "4");
    assert.equal(selectedOption(uniqueControl(current, "dob-year"), "2003")?.selected, true);
    assert.equal(selectedOption(uniqueControl(current, "dob-month"), "02")?.selected, true);
    assert.equal(nonLeapDay.options.length, 29, "2003-02 should expose 28 days plus the placeholder");
    assert.equal(nonLeapDay.options.some((option) => option.value === "29"), false);
    assert.equal(nonLeapDay.options[0].selected, true, "invalid leap day should be cleared on rerender");
    console.log("PASS event-driven DOB select rerender: 2004-02-29 -> 2003-02-28");

    const rerenderTrigger = uniqueControl(current, "rerender-trigger");
    const recoveryTarget = uniqueControl(current, "recovery-target");
    const triggerResult = await perform(options.client, tab.tab_id, current.document_id, [{
      op: "set_value",
      target_id: rerenderTrigger.target_id,
      value: RERENDER_VALUE
    }], options.timeoutSeconds);
    assertVerified(triggerResult, rerenderTrigger.target_id);
    await delay(50);

    const recoveryResult = await perform(options.client, tab.tab_id, current.document_id, [{
      op: "set_value",
      target_id: recoveryTarget.target_id,
      value: RECOVERED_VALUE
    }], options.timeoutSeconds);
    assertVerified(recoveryResult, recoveryTarget.target_id);
    current = await snapshot(options.client, tab.tab_id, options.timeoutSeconds);
    if (uniqueControl(current, "recovery-target").current_value !== RECOVERED_VALUE) {
      throw new Error("recovered value was not retained in the replacement control");
    }
    console.log("PASS unique same-document stale-target recovery");

    const ambiguityTrigger = uniqueControl(current, "ambiguity-trigger");
    const ambiguousTarget = uniqueControl(current, "ambiguous-target");
    const duplicateResult = await perform(options.client, tab.tab_id, current.document_id, [{
      op: "set_value",
      target_id: ambiguityTrigger.target_id,
      value: DUPLICATE_VALUE
    }], options.timeoutSeconds);
    assertVerified(duplicateResult, ambiguityTrigger.target_id);
    await delay(50);

    const rejected = await runClient(options.client, {
      op: "form_perform",
      tab_id: tab.tab_id,
      document_id: current.document_id,
      actions: [
        { op: "set_value", target_id: ambiguityTrigger.target_id, value: BLOCKED_TRIGGER_VALUE },
        { op: "set_value", target_id: ambiguousTarget.target_id, value: BLOCKED_VALUE }
      ]
    }, options.timeoutSeconds);
    if (rejected.code === 0 || rejected.response.status !== "error" || rejected.response.code !== "ambiguous_target") {
      throw new Error(`ambiguous remap returned ${rejected.response.code || `exit ${rejected.code}`}`);
    }
    current = await snapshot(options.client, tab.tab_id, options.timeoutSeconds);
    const duplicateControls = controlsFrom(current).filter((control) => control.name === "ambiguous-target");
    const untouchedTrigger = uniqueControl(current, "ambiguity-trigger");
    if (duplicateControls.length !== 2 ||
        duplicateControls.some((control) => control.current_value === BLOCKED_VALUE) ||
        untouchedTrigger.current_value === BLOCKED_TRIGGER_VALUE) {
      throw new Error("ambiguous remap partially changed the rejected batch");
    }
    console.log("PASS ambiguous remap rejection without partial write");

    const finalResetTrigger = uniqueControl(current, "ambiguity-trigger");
    const finalReset = await perform(options.client, tab.tab_id, current.document_id, [{
      op: "set_value",
      target_id: finalResetTrigger.target_id,
      value: RESET_VALUE
    }], options.timeoutSeconds);
    assertVerified(finalReset, finalResetTrigger.target_id);
    console.log(`PASS ${FIXTURE_TITLE} live E2E`);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

const invokedPath = process.argv[1] ? path.resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`FAIL ${error.message}`);
    process.exitCode = 1;
  });
}
