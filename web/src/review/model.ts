import type { Group, Track } from "../api/client";
import { formatDuration, formatReleaseDate } from "../format";
import type { Choices } from "./choices";
import { defaultKeep } from "./keep";
import { groupReason } from "./reason";

export type Badge = "certain" | "probable" | "variant" | "manual";

export interface ReviewGroup {
  key: string;
  confidence: Group["confidence"];
  title: string;
  artist: string;
  reason: string;
  badge: Badge;
  tracks: Track[];
  dismissed: boolean;
  promoted: boolean;
  /** Whether the group's tracks are removed, all but `keepId`. */
  isDuplicate: boolean;
  keepId: string | null;
  differs: { album: boolean; releaseDate: boolean; duration: boolean };
}

export const TABS = ["duplicates", "certain", "probable", "variant", "ignored"] as const;

export type Tab = (typeof TABS)[number];

export const TAB_LABELS: Record<Tab, string> = {
  duplicates: "Doublons",
  certain: "Certains",
  probable: "Probables",
  variant: "Variantes",
  ignored: "Ignorés",
};

export const TAB_FILTERS: Record<Tab, (group: ReviewGroup) => boolean> = {
  duplicates: (group) => group.isDuplicate,
  certain: (group) => group.confidence === "certain" && !group.dismissed,
  probable: (group) => group.confidence === "probable" && !group.dismissed,
  variant: (group) => group.confidence === "variant" && !group.promoted,
  ignored: (group) => group.dismissed,
};

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** The title without its version label, in the ` - x`, ` (x)` or ` [x]` form. */
function withoutVersion(track: Track): string {
  if (track.versionLabel == null) {
    return track.title;
  }
  const label = escapeRegExp(track.versionLabel.trim());
  const suffix = new RegExp(
    `\\s+(?:-\\s+${label}|\\(\\s*${label}\\s*\\)|\\[\\s*${label}\\s*\\])\\s*$`,
    "i",
  );
  return track.title.replace(suffix, "").trim();
}

/** The base title of a bare track when there is one, of the shortest track otherwise. */
function groupTitle(tracks: readonly Track[]): string {
  const bare = tracks.filter((track) => track.versionLabel == null);
  const candidates = bare.length > 0 ? bare : tracks;
  const chosen = candidates.reduce<Track | undefined>(
    (shortest, track) =>
      shortest && shortest.title.length <= track.title.length ? shortest : track,
    undefined,
  );
  return chosen ? withoutVersion(chosen) : "";
}

function differs(tracks: readonly Track[], value: (track: Track) => string): boolean {
  return new Set(tracks.map(value)).size > 1;
}

/**
 * The scan's groups as the user sees them: their choices applied, and the
 * tracks already removed left out. A group left with a single track is gone.
 */
export function reviewGroups(
  groups: readonly Group[],
  choices: Choices,
  removed: ReadonlySet<string>,
): ReviewGroup[] {
  const dismissed = new Set(choices.dismissed);
  const promoted = new Set(choices.promoted);
  return groups.flatMap((group): ReviewGroup[] => {
    const tracks = group.tracks.filter((track) => !removed.has(track.id));
    if (tracks.length < 2) {
      return [];
    }
    const isVariant = group.confidence === "variant";
    const isDismissed = !isVariant && dismissed.has(group.key);
    const isPromoted = isVariant && promoted.has(group.key);
    const isDuplicate = (!isVariant || isPromoted) && !isDismissed;
    const picked = choices.keep[group.key];
    const keepId = !isDuplicate
      ? null
      : picked !== undefined && tracks.some((track) => track.id === picked)
        ? picked
        : defaultKeep(tracks, choices.rule);
    return [
      {
        key: group.key,
        confidence: group.confidence,
        title: groupTitle(tracks),
        artist: tracks[0]?.artists.join(", ") ?? "",
        reason: groupReason({ ...group, tracks }),
        badge: isPromoted ? "manual" : group.confidence,
        tracks,
        dismissed: isDismissed,
        promoted: isPromoted,
        isDuplicate,
        keepId,
        differs: {
          album: differs(tracks, (track) => track.album),
          releaseDate: differs(tracks, (track) => formatReleaseDate(track.releaseDate)),
          duration: differs(tracks, (track) => formatDuration(track.durationMs)),
        },
      },
    ];
  });
}

/**
 * The tracks marked for removal: every duplicate but the one kept, once
 * each, since a track can sit in a duplicate group and a variant group.
 */
export function tracksToRemove(groups: readonly ReviewGroup[]): Track[] {
  const marked = new Map<string, Track>();
  for (const group of groups) {
    if (group.isDuplicate) {
      for (const track of group.tracks) {
        if (track.id !== group.keepId) {
          marked.set(track.id, track);
        }
      }
    }
  }
  return [...marked.values()];
}
