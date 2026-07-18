修复 `src/dedupeEvents.mjs` 中的 `dedupeEvents(events)`。

要求：
1. 忽略不是对象、缺少非空字符串 `id`、或 `occurredAt` 无法被 `Date.parse` 解析的条目。
2. 每个 `id` 只保留时间最新的事件；时间相同则保留输入中靠后的事件。
3. 结果按 `occurredAt` 升序排列；时间相同按 `id` 升序。
4. 不得修改输入数组或输入对象。
5. 保持导出函数签名不变。
