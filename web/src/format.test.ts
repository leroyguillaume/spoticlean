import { describe, expect, it } from "vitest";
import {
  countTracks,
  formatDateTime,
  formatDay,
  formatDuration,
  formatReleaseDate,
} from "./format";

describe("formatReleaseDate", () => {
  it("keeps the precision Spotify gives", () => {
    expect(formatReleaseDate("1987")).toBe("1987");
    expect(formatReleaseDate("1987-08")).toBe("08/1987");
    expect(formatReleaseDate("1987-08-31")).toBe("31/08/1987");
  });

  it("shows a dash when the date is unknown", () => {
    expect(formatReleaseDate(null)).toBe("—");
    expect(formatReleaseDate(undefined)).toBe("—");
  });
});

describe("formatDuration", () => {
  it("rounds to the second", () => {
    expect(formatDuration(212_400)).toBe("3:32");
    expect(formatDuration(59_600)).toBe("1:00");
  });

  it("adds hours past an hour", () => {
    expect(formatDuration(3_725_000)).toBe("1:02:05");
  });
});

describe("counts and dates", () => {
  it("agrees the word with the count", () => {
    expect(countTracks(1)).toBe("1 titre");
    expect(countTracks(2233)).toMatch(/^2\s233 titres$/u);
  });

  it("formats days and times the French way", () => {
    expect(formatDay("2024-05-22T12:00:00Z")).toBe("22/05/2024");
    expect(formatDay("not a date")).toBe("—");
    expect(formatDateTime(new Date(2026, 9, 30, 18, 8))).toBe("30/10/2026 à 18:08");
  });
});
