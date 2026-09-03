import test from "node:test";
import assert from "node:assert/strict";

import {
  controlsFrom,
  parseArgs,
  parseClientOutput,
  uniqueControl
} from "./live_bridge_test.mjs";

test("runner options are bounded and dependency free", () => {
  assert.deepEqual(parseArgs(["--host", "127.0.0.1", "--port", "49123", "--timeout", "30", "--client", "/tmp/client"]), {
    host: "127.0.0.1",
    port: 49123,
    timeoutSeconds: 30,
    client: "/tmp/client"
  });
  assert.throws(() => parseArgs(["--port", "80"]), /--port/);
  assert.throws(() => parseArgs(["--host", "example.test"]), /--host/);
  assert.throws(() => parseArgs(["--timeout", "601"]), /--timeout/);
  assert.throws(() => parseArgs(["--unknown"]), /unknown/);
});

test("client output must be exactly one JSON response", () => {
  assert.equal(parseClientOutput('{"status":"ok"}\n').status, "ok");
  assert.throws(() => parseClientOutput("not json\n"), /valid JSON/);
  assert.throws(() => parseClientOutput('{"status":"ok"}\n{"status":"ok"}\n'), /2 JSON lines/);
});

test("semantic control lookup refuses missing and ambiguous targets", () => {
  const snapshot = {
    forms: [{
      controls: [
        { name: "unique", target_id: "one" },
        { name: "duplicate", target_id: "two" },
        { name: "duplicate", target_id: "three" }
      ]
    }]
  };
  assert.equal(controlsFrom(snapshot).length, 3);
  assert.equal(uniqueControl(snapshot, "unique").target_id, "one");
  assert.throws(() => uniqueControl(snapshot, "missing"), /found 0/);
  assert.throws(() => uniqueControl(snapshot, "duplicate"), /found 2/);
});
