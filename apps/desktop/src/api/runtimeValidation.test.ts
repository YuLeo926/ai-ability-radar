import { describe, expect, test } from "vitest";
import type {
  FailureKind,
  RunDetail,
  RunRecord,
  TaskResult,
} from "./backend";
import {
  isSafeBatchEstimate,
  isSafeBatchRecord,
  isSafeBatchRecordList,
  isSafeBatchRetryEstimate,
  isSafeClientSelectionDetection,
  isSafeNextGuidedMember,
  isSafeRunDetail,
  isSafeRunRecord,
  isSafeRunRecordList,
  isSafeScanExecutionAuthorization,
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
      modelSource: "windows_accessibility",
      modelVerification: "user_confirmed",
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

function selectionCandidate(overrides: Record<string, unknown> = {}) {
  return {
    model: "GPT-5.6",
    reasoningEffort: "max",
    surface: "codex_desktop",
    source: "windows_accessibility",
    confidence: "visible_selector",
    ...overrides,
  };
}

describe("isSafeClientSelectionDetection", () => {
  test("accepts the exact reviewed statuses, candidates, and optional values", () => {
    expect(
      isSafeClientSelectionDetection({
        status: "detected",
        candidates: [selectionCandidate()],
      }),
    ).toBe(true);
    expect(
      isSafeClientSelectionDetection({
        status: "multiple",
        candidates: [
          selectionCandidate({ reasoningEffort: null }),
          selectionCandidate({
            model: null,
            reasoningEffort: "high",
            surface: "chatgpt",
            confidence: "best_effort",
          }),
        ],
      }),
    ).toBe(true);

    for (const status of [
      "not_running",
      "not_exposed",
      "unsupported",
      "timed_out",
      "failed",
    ]) {
      expect(
        isSafeClientSelectionDetection({ status, candidates: [] }),
      ).toBe(true);
    }
  });

  test("rejects extra top-level and candidate identity or raw-data fields", () => {
    for (const extra of [
      { windowTitle: "private conversation" },
      { processPath: "private-process-path" },
      { rawControls: ["private text"] },
    ]) {
      expect(
        isSafeClientSelectionDetection({
          status: "detected",
          candidates: [selectionCandidate()],
          ...extra,
        }),
      ).toBe(false);
      expect(
        isSafeClientSelectionDetection({
          status: "detected",
          candidates: [selectionCandidate(extra)],
        }),
      ).toBe(false);
    }
  });

  test("rejects unknown status, surface, source, and confidence enums", () => {
    expect(
      isSafeClientSelectionDetection({
        status: "success",
        candidates: [],
      }),
    ).toBe(false);
    for (const candidate of [
      selectionCandidate({ surface: "browser" }),
      selectionCandidate({ source: "window_title" }),
      selectionCandidate({ confidence: "guessed" }),
    ]) {
      expect(
        isSafeClientSelectionDetection({
          status: "detected",
          candidates: [candidate],
        }),
      ).toBe(false);
    }
  });

  test("enforces display-safe scalar lengths and non-empty candidate values", () => {
    expect(
      isSafeClientSelectionDetection({
        status: "detected",
        candidates: [
          selectionCandidate({
            model: "模".repeat(120),
            reasoningEffort: "想".repeat(40),
          }),
        ],
      }),
    ).toBe(true);
    expect(
      isSafeClientSelectionDetection({
        status: "detected",
        candidates: [
          selectionCandidate({
            model: "😀".repeat(120),
            reasoningEffort: null,
          }),
        ],
      }),
    ).toBe(true);

    for (const candidate of [
      selectionCandidate({ model: "模".repeat(121) }),
      selectionCandidate({ reasoningEffort: "想".repeat(41) }),
      selectionCandidate({ model: " GPT-5.6" }),
      selectionCandidate({ model: "GPT\u0000-5.6" }),
      selectionCandidate({ model: "GPT\u202e-5.6" }),
      selectionCandidate({ model: "GPT-\ud800" }),
      selectionCandidate({ reasoningEffort: "high\u200b" }),
      selectionCandidate({ model: "", reasoningEffort: "high" }),
      selectionCandidate({ model: null, reasoningEffort: " " }),
      selectionCandidate({ model: null, reasoningEffort: null }),
    ]) {
      expect(
        isSafeClientSelectionDetection({
          status: "detected",
          candidates: [candidate],
        }),
      ).toBe(false);
    }
  });

  test("enforces the 24-candidate cap and exact status cardinality", () => {
    const candidates = Array.from({ length: 24 }, (_, index) =>
      selectionCandidate({ model: `GPT-${index}` }),
    );
    expect(
      isSafeClientSelectionDetection({
        status: "multiple",
        candidates,
      }),
    ).toBe(true);
    expect(
      isSafeClientSelectionDetection({
        status: "multiple",
        candidates: [...candidates, selectionCandidate({ model: "GPT-25" })],
      }),
    ).toBe(false);
    expect(
      isSafeClientSelectionDetection({
        status: "detected",
        candidates: [],
      }),
    ).toBe(false);
    expect(
      isSafeClientSelectionDetection({
        status: "detected",
        candidates: candidates.slice(0, 2),
      }),
    ).toBe(false);
    expect(
      isSafeClientSelectionDetection({
        status: "multiple",
        candidates: candidates.slice(0, 1),
      }),
    ).toBe(false);
    expect(
      isSafeClientSelectionDetection({
        status: "not_running",
        candidates: candidates.slice(0, 1),
      }),
    ).toBe(false);
  });

  test("requires multiple to contain at least two distinct normalized candidates", () => {
    const duplicate = selectionCandidate();
    const {
      reasoningEffort,
      surface,
      source,
      confidence,
    } = selectionCandidate();
    const modelAbsent = { reasoningEffort, surface, source, confidence };
    const modelNull = { ...modelAbsent, model: null };
    const { model } = selectionCandidate();
    const effortAbsent = { model, surface, source, confidence };
    const effortNull = { ...effortAbsent, reasoningEffort: null };

    for (const candidates of [
      [duplicate, { ...duplicate }],
      [modelAbsent, modelNull],
      [effortAbsent, effortNull],
    ]) {
      expect(
        isSafeClientSelectionDetection({
          status: "multiple",
          candidates,
        }),
      ).toBe(false);
    }
  });
});

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

  test("run validation rejects unknown model provenance", () => {
    const run = makeRun();
    run.target.modelSource = "answer_inference" as never;
    expect(isSafeRunRecord(run)).toBe(false);
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
                modelSource: "legacy_unknown",
                modelVerification: "legacy_unknown",
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

const batchTarget = {
  target: {
    kind: "chat_gpt_client",
    reportedModel: "GPT-5.6",
    reasoningEffort: "high",
    modelSource: "manual",
    modelVerification: "user_confirmed",
  },
  routeIdentity: {
    kind: "chat_gpt_client",
    modelOrRoute: "gpt-5.6",
    reasoningEffort: "high",
    executionSurface: "guided_client",
    isDefaultRoute: false,
  },
  executionAdapterIdentity: {
    executionSurface: "guided_client",
    providerFamily: "openai",
    launchKind: "guided_client",
    publicVersion: null,
    adapterContractVersion: "guided-client-v1",
  },
} as const;

function makeBatchEstimate() {
  return {
    plan: {
      suiteId: "client-quick",
      suiteVersion: "1.0.0",
      suiteContentSha256: "a".repeat(64),
      scoringRuleVersion: "ability-v1",
      mode: "quick_comparison",
      seed: 17,
      status: "created",
      schedulePolicyVersion: 1,
      taskSessionPolicyVersion: 1,
      sessionIsolationPolicy: "user_attested_fresh_conversation_per_task",
      targets: [
        batchTarget,
        {
          target: {
            ...batchTarget.target,
            kind: "claude_client",
            reportedModel: "Claude Sonnet 4.5",
          },
          routeIdentity: {
            ...batchTarget.routeIdentity,
            kind: "claude_client",
            modelOrRoute: "claude sonnet 4.5",
          },
          executionAdapterIdentity: {
            ...batchTarget.executionAdapterIdentity,
            providerFamily: "anthropic",
          },
        },
      ],
      sealedTaskBudgets: [
        { maxTurns: 1, timeBudgetSecs: 100 },
        { maxTurns: 1, timeBudgetSecs: 100 },
      ],
      costEstimate: {
        policyVersion: 1,
        executionSurface: "guided_client",
        mode: "quick_comparison",
        targetCount: 2,
        repetitionsPerTarget: 1,
        tasksPerMemberRun: 2,
        plannedMemberRuns: 2,
        taskLaunches: 4,
        guidedInteractions: 4,
        maxProviderTurns: 4,
        summedTaskBudgetSecs: 400,
        expectedElapsedSecsMin: 1_200,
        expectedElapsedSecsMax: 1_800,
        providerExecutionCeilingSecs: 1_000,
        authorizationWallClockSecs: 14_400,
        issuedAt: "2026-07-30T02:00:00Z",
        initialAcknowledgementExpiresAt: "2026-07-30T02:15:00Z",
        tokenQuotaAmount: null,
        automaticRetryBudget: 0,
      },
      acknowledgementHash: "b".repeat(64),
    },
    capabilities: ["guided_quick_v1", "cli_standard_v1"],
  };
}

function makeBatchRecord() {
  const estimate = makeBatchEstimate();
  return {
    id: "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
    plan: estimate.plan,
    status: "created",
    cancelRequested: false,
    plannedMemberCount: 2,
    terminalMemberCount: 0,
    createdAt: "2026-07-30T02:00:01Z",
    updatedAt: "2026-07-30T02:00:01Z",
    members: [
      {
        ordinal: 0,
        targetPosition: 1,
        repetitionIndex: 0,
        runId: null,
        status: "planned",
        failureKind: null,
        attemptNumber: 0,
        updatedAt: "2026-07-30T02:00:01Z",
      },
      {
        ordinal: 1,
        targetPosition: 0,
        repetitionIndex: 0,
        runId: null,
        status: "planned",
        failureKind: null,
        attemptNumber: 0,
        updatedAt: "2026-07-30T02:00:01Z",
      },
    ],
  };
}

function makeAuthorization() {
  return {
    batchId: "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
    memberOrdinal: null,
    attemptNumber: 1,
    maxTaskLaunches: 4,
    maxProviderTurns: 4,
    maxTaskBudgetSecs: 400,
    maxGuidedInteractions: 4,
    acknowledgementHash: "b".repeat(64),
    allowedFailureKind: null,
    expiresAt: "2026-07-30T06:00:02Z",
    createdAt: "2026-07-30T02:00:02Z",
  };
}

describe("strict batch runtime validation", () => {
  test("accepts exact estimates, records, authorizations, retries, and guided decisions", () => {
    const estimate = makeBatchEstimate();
    const record = makeBatchRecord();
    const authorization = makeAuthorization();
    expect(isSafeBatchEstimate(estimate)).toBe(true);
    expect(isSafeBatchRecord(record)).toBe(true);
    expect(isSafeBatchRecordList([record])).toBe(true);
    expect(isSafeScanExecutionAuthorization(authorization)).toBe(true);
    expect(
      isSafeBatchRetryEstimate({
        authorization: {
          ...authorization,
          memberOrdinal: 0,
          maxTaskLaunches: 2,
          maxProviderTurns: 2,
          maxTaskBudgetSecs: 200,
          maxGuidedInteractions: 2,
          allowedFailureKind: "network",
        },
      }),
    ).toBe(true);
    expect(
      isSafeNextGuidedMember({
        decision: "runnable",
        member: record.members[0],
        target: record.plan.targets[1],
      }),
    ).toBe(true);
    expect(
      isSafeNextGuidedMember({
        decision: "exhausted",
        member: null,
        target: null,
      }),
    ).toBe(true);
  });

  test("fails closed for unknown nested fields, enum strings, non-finite values, and incoherence", () => {
    const estimate = makeBatchEstimate();
    expect(
      isSafeBatchEstimate({
        ...estimate,
        plan: {
          ...estimate.plan,
          targets: [
            {
              ...estimate.plan.targets[0],
              executionAdapterIdentity: {
                ...estimate.plan.targets[0].executionAdapterIdentity,
                program: "C:/private/codex.exe",
              },
            },
            estimate.plan.targets[1],
          ],
        },
      }),
    ).toBe(false);
    expect(
      isSafeBatchEstimate({
        ...estimate,
        plan: {
          ...estimate.plan,
          targets: [
            {
              ...estimate.plan.targets[0],
              target: {
                ...estimate.plan.targets[0].target,
                reportedModel: "C:/private/model",
              },
              routeIdentity: {
                ...estimate.plan.targets[0].routeIdentity,
                modelOrRoute: "c:/private/model",
              },
            },
            estimate.plan.targets[1],
          ],
        },
      }),
    ).toBe(false);
    expect(
      isSafeBatchEstimate({
        ...estimate,
        plan: { ...estimate.plan, mode: "turbo" },
      }),
    ).toBe(false);
    expect(
      isSafeBatchEstimate({
        ...estimate,
        plan: {
          ...estimate.plan,
          costEstimate: {
            ...estimate.plan.costEstimate,
            expectedElapsedSecsMax: Number.POSITIVE_INFINITY,
          },
        },
      }),
    ).toBe(false);
    expect(
      isSafeBatchRecord({
        ...makeBatchRecord(),
        terminalMemberCount: 1,
      }),
    ).toBe(false);
    expect(
      isSafeBatchRecord({
        ...makeBatchRecord(),
        plannedMemberCount: 1,
        members: makeBatchRecord().members.slice(0, 1),
      }),
    ).toBe(false);
    expect(
      isSafeNextGuidedMember({
        decision: "exhausted",
        member: makeBatchRecord().members[0],
        target: batchTarget,
      }),
    ).toBe(false);
    expect(
      isSafeScanExecutionAuthorization({
        ...makeAuthorization(),
        expiresAt: "2026-08-02T02:00:03Z",
      }),
    ).toBe(false);
    expect(
      isSafeScanExecutionAuthorization({
        ...makeAuthorization(),
        maxTaskLaunches: 51,
      }),
    ).toBe(false);
    expect(
      isSafeScanExecutionAuthorization({
        ...makeAuthorization(),
        maxGuidedInteractions: 3,
      }),
    ).toBe(false);
    expect(
      isSafeBatchEstimate({
        ...estimate,
        plan: {
          ...estimate.plan,
          sealedTaskBudgets: [
            { maxTurns: 17, timeBudgetSecs: 100 },
            { maxTurns: 1, timeBudgetSecs: 100 },
          ],
          costEstimate: {
            ...estimate.plan.costEstimate,
            maxProviderTurns: 36,
          },
        },
      }),
    ).toBe(false);
  });

  test("enforces hard bounds on nested and top-level response arrays", () => {
    const estimate = makeBatchEstimate();
    expect(
      isSafeBatchEstimate({
        ...estimate,
        plan: {
          ...estimate.plan,
          targets: Array.from({ length: 6 }, () => estimate.plan.targets[0]),
        },
      }),
    ).toBe(false);
    expect(
      isSafeBatchEstimate({
        ...estimate,
        plan: {
          ...estimate.plan,
          sealedTaskBudgets: Array.from(
            { length: 9 },
            () => estimate.plan.sealedTaskBudgets[0],
          ),
        },
      }),
    ).toBe(false);
    expect(
      isSafeBatchRecordList(
        Array.from({ length: 257 }, () => makeBatchRecord()),
      ),
    ).toBe(false);
    expect(
      isSafeBatchRecord({
        ...makeBatchRecord(),
        plannedMemberCount: 26,
        members: Array.from({ length: 26 }, (_, ordinal) => ({
          ...makeBatchRecord().members[0],
          ordinal,
        })),
      }),
    ).toBe(false);
  });
});
