import { describe, expect, test } from "vitest";
import {
  BATCH_RESPONSE_LIMITS,
  supportsBatchMode,
  type BatchFeatureLevel,
} from "./batch";

describe("batch capability policy", () => {
  const capabilities: BatchFeatureLevel[] = [
    "guided_quick_v1",
    "cli_standard_v1",
  ];

  test("exposes only guided quick and CLI quick/standard before reliable analysis", () => {
    expect(
      supportsBatchMode(capabilities, "guided_client", "quick_comparison"),
    ).toBe(true);
    expect(
      supportsBatchMode(capabilities, "automated_cli", "quick_comparison"),
    ).toBe(true);
    expect(
      supportsBatchMode(capabilities, "automated_cli", "standard"),
    ).toBe(true);
    expect(
      supportsBatchMode(capabilities, "guided_client", "standard"),
    ).toBe(false);
    expect(
      supportsBatchMode(capabilities, "automated_cli", "full"),
    ).toBe(false);
  });

  test("full remains unavailable until the explicit reliable capability exists", () => {
    expect(
      supportsBatchMode(
        [...capabilities, "reliable_full_v1"],
        "automated_cli",
        "full",
      ),
    ).toBe(true);
    expect(
      supportsBatchMode(
        [...capabilities, "reliable_full_v1"],
        "guided_client",
        "full",
      ),
    ).toBe(false);
  });

  test("publishes finite hard bounds for every response array", () => {
    expect(BATCH_RESPONSE_LIMITS.targets).toBe(5);
    expect(BATCH_RESPONSE_LIMITS.members).toBe(25);
    expect(BATCH_RESPONSE_LIMITS.taskBudgets).toBe(8);
    expect(BATCH_RESPONSE_LIMITS.batchList).toBe(256);
  });
});
