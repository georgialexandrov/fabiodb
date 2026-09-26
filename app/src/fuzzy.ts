/**
 * Scores `text` against `query`: every query character must appear in order.
 * Higher is better; `null` means no match. Rewards consecutive runs and
 * matches at word starts, so "tr" ranks "track" above "playlist_track".
 */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.toLowerCase().replace(/\s+/g, "");
  if (!q) return 0;
  const t = text.toLowerCase();
  let score = 0;
  let ti = 0;
  let run = 0;
  for (const ch of q) {
    const found = t.indexOf(ch, ti);
    if (found === -1) return null;
    run = found === ti ? run + 1 : 1;
    const wordStart = found === 0 || /[\s._\-/:]/.test(t[found - 1]);
    score += 1 + run * 2 + (wordStart ? 5 : 0) - Math.min(found - ti, 10) * 0.1;
    ti = found + 1;
  }
  // Shorter texts win ties: "track" over "track_archive".
  return score - t.length * 0.01;
}

/** Items matching `query`, best first; all of them, in order, when it's empty. */
export function fuzzyFilter<T>(items: T[], query: string, text: (item: T) => string): T[] {
  if (!query.trim()) return items;
  return items
    .map((item) => ({ item, score: fuzzyScore(query, text(item)) }))
    .filter((x): x is { item: T; score: number } => x.score !== null)
    .sort((a, b) => b.score - a.score)
    .map((x) => x.item);
}
