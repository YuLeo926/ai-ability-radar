import { describe, expect, test } from "vitest";
import {
  BATCH_RESPONSE_LIMITS,
  regressionSignalLabel,
  supportsBatchMode,
  type BatchFeatureLevel,
} from "./batch";

describe("batch capability policy", () => {
  const capabilities: BatchFeatureLevel[] = [
    "guided_quick_v1",
    "cli_standard_v1",
    "reliable_full_v1",
  ];

  test("exposes guided quick, CLI standard, and snapshot-backed Full", () => {
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
    ).toBe(true);
  });

  test("full still requires the explicit reliable capability", () => {
    expect(
      supportsBatchMode(
        capabilities,
        "automated_cli",
        "full",
      ),
    ).toBe(true);
    expect(
      supportsBatchMode(
        capabilities,
        "guided_client",
        "full",
      ),
    ).toBe(false);
    expect(
      supportsBatchMode(
        ["guided_quick_v1", "cli_standard_v1"],
        "automated_cli",
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

  test("keeps the strongest wording behind the calibration gate", () => {
    expect(regressionSignalLabel("stable")).toBe("表现稳定");
    expect(regressionSignalLabel("watch")).toBe("值得复测");
    expect(regressionSignalLabel("likely_regression")).toBe("值得复测");
    expect(regressionSignalLabel("insufficient_data")).toBe("证据不足");
  });
});
