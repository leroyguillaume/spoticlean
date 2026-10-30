import type { Group, ScanResult, Track } from "../api/client";

let next = 0;

/** A Spotify-shaped track ID, unique within the test run. */
export function trackId(): string {
  next += 1;
  return `trk${String(next).padStart(19, "0")}`;
}

export function track(overrides: Partial<Track> = {}): Track {
  return {
    id: trackId(),
    title: "Marée haute",
    artists: ["Les Falaises"],
    album: "Grand Large",
    releaseDate: "2001-04-12",
    durationMs: 214_000,
    addedAt: "2023-03-14T12:00:00Z",
    isrc: "FRZZZ0100001",
    versionLabel: null,
    ...overrides,
  };
}

export function group(overrides: Partial<Group> & Pick<Group, "tracks">): Group {
  return {
    key: `key-${trackId()}`,
    confidence: "certain",
    durationGapMs: 0,
    ...overrides,
  };
}

export function scanResult(groups: Group[], scanned = 1234): ScanResult {
  return { scanned, groups };
}
