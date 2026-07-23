import type { ClientSelectionCandidate } from "../api/backend";

export function clientSelectionCandidateKey(
  candidate: ClientSelectionCandidate,
): string {
  return JSON.stringify([
    candidate.model ?? null,
    candidate.reasoningEffort ?? null,
    candidate.surface,
    candidate.source,
    candidate.confidence,
  ]);
}

export function distinctClientSelectionCandidates(
  candidates: ClientSelectionCandidate[],
): ClientSelectionCandidate[] {
  const seen = new Set<string>();
  return candidates.filter((candidate) => {
    const key = clientSelectionCandidateKey(candidate);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}
