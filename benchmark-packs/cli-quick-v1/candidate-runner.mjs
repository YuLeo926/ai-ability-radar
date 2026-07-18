import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const trustedDeepEqual = assert.deepEqual.bind(assert);
const trustedThrows = assert.throws.bind(assert);
const trustedClone = structuredClone;
const trustedTypeError = TypeError;
const protocolWrite = process.stdout.write.bind(process.stdout);
const nonce = fs.readFileSync(0, "utf8");
const workspace = process.argv[2];
const taskId = process.argv[3];

if (!nonce || !workspace || !["dedupe-events", "retry-schedule"].includes(taskId)) {
  process.exitCode = 2;
} else {
  protocolWrite(`RUNNER_READY ${nonce}\n`);
  try {
    if (taskId === "dedupe-events") {
      await verifyDedupe(workspace);
    } else {
      await verifyRetry(workspace);
    }
    protocolWrite(`RUNNER_PASSED ${nonce}\n`);
    process.exitCode = 0;
  } catch {
    protocolWrite(`RUNNER_FAILED ${nonce}\n`);
    process.exitCode = 1;
  }
}

async function loadExport(workspaceRoot, fileName, exportName) {
  const moduleUrl = pathToFileURL(path.join(workspaceRoot, "src", fileName));
  moduleUrl.searchParams.set("run", String(Date.now()));
  const candidateModule = await import(moduleUrl.href);
  const candidateExport = candidateModule[exportName];
  if (typeof candidateExport !== "function") {
    throw new trustedTypeError(`missing export ${exportName}`);
  }
  return candidateExport;
}

async function verifyDedupe(workspaceRoot) {
  const dedupeEvents = await loadExport(
    workspaceRoot,
    "dedupeEvents.mjs",
    "dedupeEvents",
  );

  {
    const valid = {
      id: "valid",
      occurredAt: "2026-01-01T00:00:00Z",
      payload: { n: 1 },
    };
    const input = [null, undefined, 0, 7, "event", true, valid];
    trustedDeepEqual(dedupeEvents(input), [valid]);
  }

  {
    const valid = {
      id: "valid",
      occurredAt: "2026-01-01T00:00:00Z",
      payload: { n: 1 },
    };
    const input = [
      { occurredAt: "2026-01-01T00:00:00Z" },
      { id: 12, occurredAt: "2026-01-01T00:00:00Z" },
      { id: "", occurredAt: "2026-01-01T00:00:00Z" },
      valid,
    ];
    trustedDeepEqual(dedupeEvents(input), [valid]);
  }

  {
    const valid = {
      id: "valid",
      occurredAt: "2026-01-01T00:00:00Z",
      payload: { n: 1 },
    };
    const input = [
      { id: "missing-date" },
      { id: "invalid-date", occurredAt: "not-a-date" },
      valid,
    ];
    trustedDeepEqual(dedupeEvents(input), [valid]);
  }

  {
    const earlier = {
      id: "same",
      occurredAt: "2026-01-02T00:00:00Z",
      payload: { revision: 1 },
    };
    const laterInput = {
      id: "same",
      occurredAt: "2026-01-02T00:00:00Z",
      payload: { revision: 2 },
    };
    trustedDeepEqual(dedupeEvents([earlier, laterInput]), [laterInput]);
  }

  {
    const z = { id: "z", occurredAt: "2026-01-03T00:00:00Z" };
    const a = { id: "a", occurredAt: "2026-01-03T00:00:00Z" };
    const m = { id: "m", occurredAt: "2026-01-03T00:00:00Z" };
    trustedDeepEqual(dedupeEvents([z, a, m]), [a, m, z]);
  }

  {
    const late = { id: "late", occurredAt: "2026-01-04T00:00:00Z" };
    const early = { id: "early", occurredAt: "2026-01-01T00:00:00Z" };
    const middle = { id: "middle", occurredAt: "2026-01-02T00:00:00Z" };
    trustedDeepEqual(dedupeEvents([late, early, middle]), [
      early,
      middle,
      late,
    ]);
  }

  {
    const input = [
      {
        id: "b",
        occurredAt: "2026-01-02T00:00:00Z",
        payload: { nested: { n: 1 } },
      },
      {
        id: "a",
        occurredAt: "2026-01-01T00:00:00Z",
        payload: { nested: { n: 2 } },
      },
    ];
    const snapshot = trustedClone(input);
    dedupeEvents(input);
    trustedDeepEqual(input, snapshot);
  }
}

async function verifyRetry(workspaceRoot) {
  const buildRetrySchedule = await loadExport(
    workspaceRoot,
    "retrySchedule.mjs",
    "buildRetrySchedule",
  );

  trustedDeepEqual(
    buildRetrySchedule({
      maxAttempts: 4,
      baseDelayMs: 4,
      maxDelayMs: 5,
    }),
    [0, 4, 9, 14],
  );

  trustedDeepEqual(
    buildRetrySchedule({
      maxAttempts: 4,
      baseDelayMs: 2,
      maxDelayMs: 8,
      retryAfterMs: [9, 0, 20],
    }),
    [0, 9, 13, 33],
  );

  trustedDeepEqual(
    buildRetrySchedule({
      maxAttempts: 4,
      baseDelayMs: 3,
      maxDelayMs: 20,
    }),
    [0, 3, 9, 21],
  );

  trustedDeepEqual(
    buildRetrySchedule({
      maxAttempts: 1,
      baseDelayMs: 10,
      maxDelayMs: 10,
    }),
    [0],
  );

  {
    const retryAfterMs = [0, 5, 1];
    const snapshot = trustedClone(retryAfterMs);
    buildRetrySchedule({
      maxAttempts: 4,
      baseDelayMs: 2,
      maxDelayMs: 8,
      retryAfterMs,
    });
    trustedDeepEqual(retryAfterMs, snapshot);
  }

  const valid = {
    maxAttempts: 2,
    baseDelayMs: 2,
    maxDelayMs: 4,
    retryAfterMs: [0],
  };
  for (const maxAttempts of [0, -1, 1.5, "2", null, undefined]) {
    assertTypeError(buildRetrySchedule, { ...valid, maxAttempts });
  }
  for (const baseDelayMs of [0, -1, 1.5, "2", null, undefined]) {
    assertTypeError(buildRetrySchedule, { ...valid, baseDelayMs });
  }
  for (const maxDelayMs of [-1, 0, 1, 1.5, "4", null, undefined]) {
    assertTypeError(buildRetrySchedule, { ...valid, maxDelayMs });
  }
  for (const retryAfterMs of [
    "not-an-array",
    null,
    [-1],
    [1.5],
    ["1"],
    [null],
    [undefined],
    [0, -1],
    [0, 1.5],
    [0, "1"],
    [0, null],
    [0, undefined],
  ]) {
    assertTypeError(buildRetrySchedule, { ...valid, retryAfterMs });
  }
}

function assertTypeError(candidate, options) {
  trustedThrows(() => candidate(options), trustedTypeError);
}
