修复 `src/retrySchedule.mjs` 中的 `buildRetrySchedule(options)`。

要求：
1. `maxAttempts` 包含第一次立即执行，因此结果第一项始终是 `0`。
2. 后续基础延迟为 `baseDelayMs * 2^(retryIndex-1)`，并限制在 `maxDelayMs`。
3. `retryAfterMs` 可为每次重试提供最小延迟；实际延迟取基础延迟和对应值的较大者。
4. 返回累计时间点，而不是单次延迟。
5. 所有输入必须是非负整数，且 `maxAttempts>=1`、`baseDelayMs>=1`、`maxDelayMs>=baseDelayMs`；无效时抛出 `TypeError`。
6. 不得修改 `retryAfterMs`。
