import type { Group } from "../api/client";

const VERSION_KINDS: ReadonlyArray<readonly [RegExp, string]> = [
  [/\blive\b/i, "live"],
  [/\bacoustic\b|\bunplugged\b/i, "acoustique"],
  [/\bremix\b/i, "remix"],
  [/\bmix\b/i, "mix"],
  [/\bdemo\b/i, "démo"],
  [/\binstrumental\b/i, "instrumentale"],
  [/\bedit\b/i, "edit"],
  [/re-?record|\w's version\b/i, "réenregistrée"],
];

/** Suffixes naming a reissue of the same recording, which tell nothing about the variant. */
const REISSUE =
  /remaster|deluxe|edition|anniversary|expanded|\bmono\b|\bstereo\b|(single|album) version|^\d{4}$|^from\b/i;

/** The French words for a version label's parts, `Live · 2011 Remaster` giving `live`. */
function versionKinds(label: string): string[] {
  return label
    .split(" · ")
    .filter((part) => !REISSUE.test(part))
    .map((part) => VERSION_KINDS.find(([pattern]) => pattern.test(part))?.[1] ?? `« ${part} »`);
}

function joinFrench(items: readonly string[]): string {
  return items.length <= 1
    ? (items[0] ?? "")
    : `${items.slice(0, -1).join(", ")} et ${items[items.length - 1]}`;
}

function variantReason(group: Group): string {
  const kinds = [
    ...new Set(
      group.tracks.flatMap((track) => (track.versionLabel ? versionKinds(track.versionLabel) : [])),
    ),
  ];
  if (kinds.length === 0) {
    return "Artistes crédités différents";
  }
  return kinds.length === 1 ? `Version ${kinds[0]}` : `Versions ${joinFrench(kinds)}`;
}

/** Why the tracks were grouped, in a few words. */
export function groupReason(group: Group): string {
  switch (group.confidence) {
    case "certain":
      return "Même ISRC";
    case "probable": {
      const seconds = Math.round(group.durationGapMs / 1000);
      return `Même titre et artiste · ${seconds === 0 ? "même durée" : `${seconds} s d'écart`}`;
    }
    case "variant":
      return variantReason(group);
  }
}
