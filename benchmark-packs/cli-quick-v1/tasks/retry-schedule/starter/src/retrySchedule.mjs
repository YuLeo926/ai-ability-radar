export function buildRetrySchedule({
  maxAttempts,
  baseDelayMs,
  maxDelayMs,
  retryAfterMs = [],
}) {
  const result = [];
  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    result.push(baseDelayMs * 2 ** attempt);
  }
  return result;
}
