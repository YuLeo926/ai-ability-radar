import { describe, expect, test } from "vitest";
import type {
  FailureKind,
  RunDetail,
  RunRecord,
  TaskResult,
} from "./backend";
import {
  isSafeRunDetail,
  isSafeRunRecord,
  isSafeRunRecordList,
  scoreableResultScore,
} from "./runtimeValidation";

const runId = "validation-run";

function makeTask(
  index: number,
  overrides: Partial<TaskResult> = {},
): TaskResult {
  return {
    runId,
    taskId: `task-${index}`,
    category: "instruction_following",
    outcome: "passed",
    score: 100,
    failureKind: null,
    durationMs: 1_000,
    answerRelPath: null,
    ...overrides,
  };
}

function canonicalTasks(): TaskResult[] {
  return [
    makeTask(1),
    makeTask(2, {
      outcome: "failed",
      score: 0,
      failureKind: "wrong_answer",
    }),
    makeTask(3, {
      category: "logic",
      outcome: "failed",
      score: 50,
      failureKind: null,
    }),
    makeTask(4, {
      category: "code_review",
      outcome: "invalid",
      score: 0,
      failureKind: "network",
    }),
  ];
}

function makeRun(overrides: Partial<RunRecord> = {}): RunRecord {
  return {
    id: runId,
    target: {
      kind: "chat_gpt_client",
      reportedModel: "GPT-X",
      reasoningEffort: "high",
    },
    mode: "quick",
    suiteId: "client-quick",
    suiteVersion: "1.0.0",
    status: "completed",
    startedAt: "2026-07-17T00:00:00Z",
    finishedAt: "2026-07-17T00:12:00Z",
    totalTasks: 4,
    completedTasks: 4,
    environment: {
      osFamily: "Windows",
      osVersion: "11",
      appVersion: "0.2.0",
      cliVersion: null,
      verifierRuntimeVersion: "embedded-verifier 1.0.0",
      suiteId: "client-quick",
      suiteVersion: "1.0.0",
      suiteContentSha256: "e".repeat(64),
      scoringRuleVersion: "ability-v1",
      resumed: false,
    },
    score: {
      abilityScore: 50,
      passedTasks: 1,
      validTasks: 3,
      totalTasks: 4,
      categoryScores: {
        instruction_following: 50,
        logic: 50,
      },
    },
    ...overrides,
  };
}

function makeDetail(
  runOverrides: Partial<RunRecord> = {},
  taskResults = canonicalTasks(),
): RunDetail {
  return {
    run: makeRun(runOverrides),
    taskResults,
  };
}

describe("scoreableResultScore", () => {
  test("matches the core scoring predicate exactly", () => {
    expect(scoreableResultScore(makeTask(1))).toBe(100);
    expect(
      scoreableResultScore(makeTask(1, { outcome: "passed", score: 99 })),
    ).toBeNull();
    expect(
      scoreableResultScore(
        makeTask(1, {
          outcome: "passed",
          score: 100,
          failureKind: "network",
        }),
      ),
    ).toBeNull();
    expect(
      scoreableResultScore(
        makeTask(1, { outcome: "failed", score: 25, failureKind: null }),
      ),
    ).toBe(25);
    expect(
      scoreableResultScore(
        makeTask(1, {
          outcome: "failed",
          score: 0,
          failureKind: "agent_budget_exceeded",
        }),
      ),
    ).toBe(0);
    expect(
      scoreableResultScore(
        makeTask(1, {
          outcome: "failed",
          score: 0,
          failureKind: "wrong_answer",
        }),
      ),
    ).toBe(0);
    expect(
      scoreableResultScore(
        makeTask(1, {
          outcome: "failed",
          score: 100,
          failureKind: "wrong_answer",
        }),
      ),
    ).toBeNull();
    expect(
      scoreableResultScore(
        makeTask(1, { outcome: "invalid", score: 0 }),
      ),
    ).toBeNull();
    expect(
      scoreableResultScore(
        makeTask(1, { outcome: "cancelled", score: 0 }),
      ),
    ).toBeNull();
  });

  test("excludes every infrastructure failure kind", () => {
    const infrastructureFailures: FailureKind[] = [
      "cli_missing",
      "runtime_missing",
      "auth_expired",
      "quota_exhausted",
      "network",
      "user_cancelled",
      "app_interrupted",
      "infrastructure_timeout",
      "verifier_error",
    ];

    for (const failureKind of infrastructureFailures) {
      expect(
        scoreableResultScore(
          makeTask(1, { outcome: "failed", score: 0, failureKind }),
        ),
      ).toBeNull();
    }
  });
});

describe("isSafeRunRecord", () => {
  test("accepts a structurally and arithmetically valid record", () => {
    const record = makeRun();
    expect(isSafeRunRecord(record)).toBe(true);
    expect(isSafeRunRecordList([record])).toBe(true);
  });

  test("requires nonempty legal category scores and their one-decimal equal-weight mean", () => {
    const baseScore = makeRun().score!;
    const invalidScores = [
      { ...baseScore, categoryScores: {} },
      {
        ...baseScore,
        categoryScores: { ...baseScore.categoryScores, unknown: 50 },
      },
      {
        ...baseScore,
        categoryScores: { instruction_following: -1 },
      },
      {
        ...baseScore,
        categoryScores: { instruction_following: 50.01 },
        abilityScore: 50.01,
      },
      { ...baseScore, abilityScore: 49.9 },
    ];

    for (const score of invalidScores) {
      expect(isSafeRunRecord(makeRun({ score } as Partial<RunRecord>))).toBe(
        false,
      );
    }
  });

  test("requires legal counts and score total equality with the run", () => {
    const baseScore = makeRun().score!;
    expect(
      isSafeRunRecord(
        makeRun({ score: { ...baseScore, totalTasks: 3 } }),
      ),
    ).toBe(false);
    expect(
      isSafeRunRecord(
        makeRun({ score: { ...baseScore, passedTasks: 4 } }),
      ),
    ).toBe(false);
    expect(
      isSafeRunRecord(makeRun({ completedTasks: 5 })),
    ).toBe(false);
    expect(
      isSafeRunRecord(
        makeRun({
          completedTasks: 2,
          score: { ...baseScore, validTasks: 3 },
        }),
      ),
    ).toBe(false);
    expect(
      isSafeRunRecord(
        makeRun({
          score: {
            abilityScore: 50,
            passedTasks: 1,
            validTasks: 1,
            totalTasks: 4,
            categoryScores: {
              instruction_following: 50,
              logic: 50,
            },
          },
        }),
      ),
    ).toBe(false);
  });

  test("allows a score only for completed runs", () => {
    for (const status of [
      "created",
      "running",
      "cancelled",
      "interrupted",
    ] as const) {
      expect(isSafeRunRecord(makeRun({ status }))).toBe(false);
      expect(isSafeRunRecord(makeRun({ status, score: null }))).toBe(true);
    }
  });

  test("requires completed records to account for every planned task", () => {
    const partialScore = {
      abilityScore: 50,
      passedTasks: 1,
      validTasks: 3,
      totalTasks: 4,
      categoryScores: {
        instruction_following: 50,
        logic: 50,
      },
    };

    expect(
      isSafeRunRecord(
        makeRun({ completedTasks: 3, score: partialScore }),
      ),
    ).toBe(false);
    expect(
      isSafeRunRecord(makeRun({ completedTasks: 3, score: null })),
    ).toBe(false);

    for (const status of ["running", "cancelled", "interrupted"] as const) {
      expect(
        isSafeRunRecord(
          makeRun({ status, completedTasks: 3, score: null }),
        ),
      ).toBe(true);
    }
  });
});

describe("isSafeRunDetail", () => {
  test("accepts evidence whose stored summary exactly recomputes", () => {
    expect(isSafeRunDetail(makeDetail())).toBe(true);
  });

  test("requires task ownership, unique task ids, and evidence count consistency", () => {
    expect(
      isSafeRunDetail(
        makeDetail({}, [
          ...canonicalTasks().slice(0, 3),
          makeTask(4, { runId: "different-run" }),
        ]),
      ),
    ).toBe(false);
    expect(
      isSafeRunDetail(
        makeDetail({}, [
          ...canonicalTasks().slice(0, 3),
          makeTask(4, { taskId: "task-3" }),
        ]),
      ),
    ).toBe(false);
    expect(
      isSafeRunDetail(makeDetail({ completedTasks: 3 })),
    ).toBe(false);
  });

  test("requires bounded task scores and nonnegative safe-integer durations", () => {
    for (const score of [-1, 101, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(
        isSafeRunDetail(
          makeDetail({}, [
            makeTask(1, { score }),
            ...canonicalTasks().slice(1),
          ]),
        ),
      ).toBe(false);
    }
    for (const durationMs of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
      expect(
        isSafeRunDetail(
          makeDetail({}, [
            makeTask(1, { durationMs }),
            ...canonicalTasks().slice(1),
          ]),
        ),
      ).toBe(false);
    }
  });

  test("rejects every stored summary field when it differs from recomputed evidence", () => {
    const score = makeRun().score!;
    const mismatches = [
      { ...score, passedTasks: 2 },
      { ...score, validTasks: 2 },
      { ...score, abilityScore: 50.1 },
      {
        ...score,
        categoryScores: {
          ...score.categoryScores,
          instruction_following: 50.1,
        },
      },
      {
        ...score,
        categoryScores: {
          ...score.categoryScores,
          code_review: 0,
        },
      },
    ];

    for (const mismatch of mismatches) {
      expect(isSafeRunDetail(makeDetail({ score: mismatch }))).toBe(false);
    }
    expect(isSafeRunDetail(makeDetail({ score: null }))).toBe(false);
  });

  test("rejects scored and no-score completed details with partial evidence", () => {
    expect(
      isSafeRunDetail(
        makeDetail(
          {
            completedTasks: 1,
            score: {
              abilityScore: 100,
              passedTasks: 1,
              validTasks: 1,
              totalTasks: 4,
              categoryScores: { instruction_following: 100 },
            },
          },
          [makeTask(1)],
        ),
      ),
    ).toBe(false);
    expect(
      isSafeRunDetail(
        makeDetail(
          {
            completedTasks: 1,
            score: null,
          },
          [
            makeTask(1, {
              outcome: "failed",
              score: null,
              failureKind: "network",
            }),
          ],
        ),
      ),
    ).toBe(false);
  });

  test("accepts completed no-score evidence only when no task is scoreable", () => {
    const excluded = [
      makeTask(1, {
        outcome: "failed",
        score: 0,
        failureKind: "network",
      }),
      makeTask(2, {
        category: "logic",
        outcome: "invalid",
        score: null,
        failureKind: "verifier_error",
      }),
    ];
    expect(
      isSafeRunDetail(
        makeDetail(
          { totalTasks: 2, completedTasks: 2, score: null },
          excluded,
        ),
      ),
    ).toBe(true);
    expect(
      isSafeRunDetail(
        makeDetail(
          {
            totalTasks: 2,
            completedTasks: 2,
            score: {
              abilityScore: 0,
              passedTasks: 0,
              validTasks: 1,
              totalTasks: 2,
              categoryScores: { instruction_following: 0 },
            },
          },
          excluded,
        ),
      ),
    ).toBe(false);
  });

  test("accepts only coherent completed task evidence semantics", () => {
    const scoreFor = (task: TaskResult): RunRecord["score"] => ({
      abilityScore: task.score!,
      passedTasks: task.outcome === "passed" ? 1 : 0,
      validTasks: 1,
      totalTasks: 1,
      categoryScores: { instruction_following: task.score! },
    });
    const completedDetail = (
      task: TaskResult,
      score: RunRecord["score"],
    ): RunDetail =>
      makeDetail(
        {
          totalTasks: 1,
          completedTasks: 1,
          score,
        },
        [task],
      );

    const validScoreable = [
      makeTask(1),
      makeTask(1, {
        outcome: "failed",
        score: 25,
        failureKind: null,
      }),
      makeTask(1, {
        outcome: "failed",
        score: 0,
        failureKind: "wrong_answer",
      }),
      makeTask(1, {
        outcome: "failed",
        score: 99,
        failureKind: "agent_budget_exceeded",
      }),
    ];
    for (const task of validScoreable) {
      expect(isSafeRunDetail(completedDetail(task, scoreFor(task)))).toBe(
        true,
      );
    }

    const infrastructureFailures: FailureKind[] = [
      "cli_missing",
      "runtime_missing",
      "auth_expired",
      "quota_exhausted",
      "network",
      "user_cancelled",
      "app_interrupted",
      "infrastructure_timeout",
      "verifier_error",
    ];
    for (const failureKind of infrastructureFailures) {
      const infrastructureFailure = makeTask(1, {
        outcome: "failed",
        score: 0,
        failureKind,
      });
      expect(
        isSafeRunDetail(completedDetail(infrastructureFailure, null)),
      ).toBe(true);
    }

    const invalidEvidence = [
      makeTask(1, { outcome: "passed", score: 99 }),
      makeTask(1, { outcome: "passed", score: null }),
      makeTask(1, {
        outcome: "passed",
        score: 100,
        failureKind: "wrong_answer",
      }),
      makeTask(1, {
        outcome: "passed",
        score: 100,
        failureKind: "network",
      }),
      makeTask(1, {
        outcome: "failed",
        score: null,
        failureKind: null,
      }),
      makeTask(1, {
        outcome: "failed",
        score: 100,
        failureKind: null,
      }),
      makeTask(1, {
        outcome: "failed",
        score: undefined,
        failureKind: "wrong_answer",
      }),
      makeTask(1, {
        outcome: "failed",
        score: null,
        failureKind: "agent_budget_exceeded",
      }),
      makeTask(1, {
        outcome: "failed",
        score: 100,
        failureKind: "wrong_answer",
      }),
      makeTask(1, {
        outcome: "failed",
        score: 100,
        failureKind: "agent_budget_exceeded",
      }),
      ...(["wrong_answer", "agent_budget_exceeded"] as const).flatMap(
        (failureKind) =>
          (["passed", "invalid", "cancelled"] as const).map((outcome) =>
            makeTask(1, {
              outcome,
              score: 0,
              failureKind,
            }),
          ),
      ),
    ];
    for (const task of invalidEvidence) {
      expect(isSafeRunDetail(completedDetail(task, null))).toBe(false);
    }
  });

  test("keeps legal running, cancelled, and interrupted partial records displayable", () => {
    for (const status of ["running", "cancelled", "interrupted"] as const) {
      const partialTask = makeTask(1, {
        outcome: "invalid",
        score: undefined,
        failureKind: undefined,
        answerRelPath: undefined,
      });
      expect(
        isSafeRunDetail(
          makeDetail(
            {
              status,
              completedTasks: 1,
              finishedAt: undefined,
              score: undefined,
              target: {
                kind: "chat_gpt_client",
                reportedModel: "default",
                reasoningEffort: undefined,
              },
              environment: {
                ...makeRun().environment,
                cliVersion: undefined,
                verifierRuntimeVersion: undefined,
              },
            },
            [partialTask],
          ),
        ),
      ).toBe(true);
    }
  });
});
