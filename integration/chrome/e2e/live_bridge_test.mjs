#!/usr/bin/env node

import { createServer } from "node:http";
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

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
    const rerenderTrigger = uniqueControl(baseline, "rerender-trigger");
    const recoveryTarget = uniqueControl(baseline, "recovery-target");
    const triggerResult = await perform(options.client, tab.tab_id, baseline.document_id, [{
      op: "set_value",
      target_id: rerenderTrigger.target_id,
      value: RERENDER_VALUE
    }], options.timeoutSeconds);
    assertVerified(triggerResult, rerenderTrigger.target_id);
    await delay(50);

    const recoveryResult = await perform(options.client, tab.tab_id, baseline.document_id, [{
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
