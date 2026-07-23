import { describe, expect, test } from "vitest";
import type { RunRecord, TargetKind } from "../api/backend";
import { comparableSeriesKey } from "./HistoryPage";

function makeRun(overrides: Partial<RunRecord> = {}): RunRecord {
  return {
    id: "10000000-0000-4000-8000-000000000001",
    target: {
      kind: "codex_cli",
      reportedModel: "default",
      reasoningEffort: "high",
      modelSource: "default_route",
      modelVerification: "unverified",
    },
    mode: "quick",
    suiteId: "cli-quick",
    suiteVersion: "1.0.0",
    status: "completed",
    startedAt: "2026-07-17T00:00:00Z",
    finishedAt: "2026-07-17T00:12:00Z",
    totalTasks: 2,
    completedTasks: 2,
    environment: {
      osFamily: "Windows",
      osVersion: "11",
      appVersion: "0.2.0",
      cliVersion: "codex 1.0.0",
      verifierRuntimeVersion: "node v22.0.0",
      suiteId: "cli-environment",
      suiteVersion: "1.0.0",
      suiteContentSha256: "a".repeat(64),
      scoringRuleVersion: "ability-v1",
      resumed: false,
    },
    score: {
      abilityScore: 75,
      passedTasks: 1,
      validTasks: 2,
      totalTasks: 2,
      categoryScores: { cli_coding: 75 },
    },
    ...overrides,
  };
}

describe("comparableSeriesKey", () => {
  test("is collision safe for values containing the old delimiter", () => {
    const left = makeRun({
      target: {
        kind: "codex_cli",
        reportedModel: "a\u001fb",
        reasoningEffort: "c",
        modelSource: "legacy_unknown",
        modelVerification: "legacy_unknown",
      },
    });
    const right = makeRun({
      target: {
        kind: "codex_cli",
        reportedModel: "a",
        reasoningEffort: "b\u001fc",
        modelSource: "legacy_unknown",
        modelVerification: "legacy_unknown",
      },
    });

    expect(comparableSeriesKey(left)).not.toBe(comparableSeriesKey(right));
    expect(() => JSON.parse(comparableSeriesKey(left))).not.toThrow();
  });

  test("separates every comparability field independently without normalization", () => {
    const base = makeRun();
    const variants: RunRecord[] = [
      makeRun({
        target: { ...base.target, kind: "claude_code" },
      }),
      makeRun({
        target: { ...base.target, reportedModel: " default" },
      }),
      makeRun({
        target: { ...base.target, reasoningEffort: "medium" },
      }),
      makeRun({ mode: "deep" }),
      makeRun({ suiteId: "cli-quick-next" }),
      makeRun({ suiteVersion: "1.0.1" }),
      makeRun({
        environment: { ...base.environment, suiteId: "environment-next" },
      }),
      makeRun({
        environment: { ...base.environment, suiteVersion: "1.0.1" },
      }),
      makeRun({
        environment: {
          ...base.environment,
          suiteContentSha256: "b".repeat(64),
        },
      }),
      makeRun({
        environment: {
          ...base.environment,
          scoringRuleVersion: "ability-v2",
        },
      }),
      makeRun({
        environment: { ...base.environment, osFamily: "Linux" },
      }),
      makeRun({
        environment: { ...base.environment, osVersion: "10" },
      }),
      makeRun({
        environment: { ...base.environment, appVersion: "0.2.1" },
      }),
      makeRun({
        environment: { ...base.environment, cliVersion: "codex 1.1.0" },
      }),
      makeRun({
        environment: {
          ...base.environment,
          verifierRuntimeVersion: "node v24.0.0",
        },
      }),
      makeRun({
        environment: { ...base.environment, resumed: true },
      }),
      makeRun({ totalTasks: 3 }),
    ];

    expect(
      new Set([base, ...variants].map(comparableSeriesKey)).size,
    ).toBe(variants.length + 1);
  });

  test("normalizes missing optional values deterministically", () => {
    const withNull = makeRun({
      target: { ...makeRun().target, reasoningEffort: null },
      environment: {
        ...makeRun().environment,
        cliVersion: null,
        verifierRuntimeVersion: null,
      },
    });
    const withMissing = makeRun({
      target: {
        kind: "codex_cli",
        reportedModel: "default",
        modelSource: "default_route",
        modelVerification: "unverified",
      },
      environment: {
        ...makeRun().environment,
        cliVersion: undefined,
        verifierRuntimeVersion: undefined,
      },
    });

    expect(comparableSeriesKey(withMissing)).toBe(
      comparableSeriesKey(withNull),
    );
  });

  test("does not split a series on observational status, score, or timestamps", () => {
    const base = makeRun();
    const observation = makeRun({
      id: "different-internal-id",
      status: "interrupted",
      startedAt: "invalid-time",
      finishedAt: null,
      completedTasks: 1,
      score: null,
    });

    expect(comparableSeriesKey(observation)).toBe(
      comparableSeriesKey(base),
    );
  });

  test("keeps all four target kinds in separate series", () => {
    const kinds: TargetKind[] = [
      "chat_gpt_client",
      "claude_client",
      "codex_cli",
      "claude_code",
    ];
    const keys = kinds.map((kind) =>
      comparableSeriesKey(
        makeRun({
          target: { ...makeRun().target, kind },
        }),
      ),
    );

    expect(new Set(keys).size).toBe(4);
  });
});
