import { z } from "zod";
import type { Track } from "../api/client";

export const KEEP_RULES = ["remaster", "original", "first", "recent"] as const;

export const zKeepRule = z.enum(KEEP_RULES);

export type KeepRule = z.infer<typeof zKeepRule>;

export const KEEP_RULE_LABELS: Record<KeepRule, string> = {
  remaster: "Remaster en priorité",
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
    case "remaster": {
      const years = remasterYear(candidate) - remasterYear(best);
      return years > 0 || (years === 0 && beats("original", candidate, best));
    }
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

const REMASTER = /remaster/i;
const YEAR = /\b(?:19|20)\d{2}\b/;

/** The year of a track's remaster: 0 when the year is unknown, -1 when it is not a remaster. */
function remasterYear(track: Track): number {
  const marked = [track.versionLabel, track.album].filter(
    (text): text is string => typeof text === "string" && REMASTER.test(text),
  );
  if (marked.length === 0) {
    return -1;
  }
  const years = marked.flatMap((text) => text.match(YEAR) ?? []).map(Number);
  return Math.max(0, ...years);
}

/** The ID of the track the rule keeps; ties go to the first track. */
export function defaultKeep(tracks: readonly Track[], rule: KeepRule): string | null {
  const effective =
    rule === "remaster" && tracks.every((t) => remasterYear(t) < 0) ? "original" : rule;
  let best: Track | undefined;
  for (const track of tracks) {
    if (best === undefined || beats(effective, track, best)) {
      best = track;
    }
  }
  return best?.id ?? null;
}
