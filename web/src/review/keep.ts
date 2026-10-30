import { z } from "zod";
import type { Track } from "../api/client";

export const KEEP_RULES = ["original", "first", "recent"] as const;

export const zKeepRule = z.enum(KEEP_RULES);

export type KeepRule = z.infer<typeof zKeepRule>;

export const KEEP_RULE_LABELS: Record<KeepRule, string> = {
  original: "Album original (sortie la plus ancienne)",
  first: "Premier liké",
  recent: "Version la plus récente",
};

/** Negative when `a` comes first. A track with no release date never wins. */
function compareReleases(a: Track, b: Track): number {
  if (!a.releaseDate || !b.releaseDate) {
    return a.releaseDate ? -1 : b.releaseDate ? 1 : 0;
  }
  return a.releaseDate.localeCompare(b.releaseDate);
}

function compareLikes(a: Track, b: Track): number {
  return Date.parse(a.addedAt) - Date.parse(b.addedAt);
}

/** Whether `candidate` should be kept over `best` under `rule`. */
function beats(rule: KeepRule, candidate: Track, best: Track): boolean {
  switch (rule) {
    case "first":
      return compareLikes(candidate, best) < 0;
    case "original": {
      const release = compareReleases(candidate, best);
      return release < 0 || (release === 0 && compareLikes(candidate, best) < 0);
    }
    case "recent": {
      if (!candidate.releaseDate) {
        return false;
      }
      const release = best.releaseDate ? compareReleases(best, candidate) : -1;
      return release < 0 || (release === 0 && compareLikes(candidate, best) > 0);
    }
  }
}

/** The ID of the track the rule keeps; ties go to the first track. */
export function defaultKeep(tracks: readonly Track[], rule: KeepRule): string | null {
  let best: Track | undefined;
  for (const track of tracks) {
    if (best === undefined || beats(rule, track, best)) {
      best = track;
    }
  }
  return best?.id ?? null;
}
