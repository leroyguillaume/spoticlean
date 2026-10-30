import { describe, expect, it } from "vitest";
import { track } from "../test/fixtures";
import { defaultKeep } from "./keep";

const original = track({ releaseDate: "1994-03-01", addedAt: "2025-01-10T12:00:00Z" });
const reissue = track({ releaseDate: "2014-06-01", addedAt: "2022-07-01T12:00:00Z" });
const compilation = track({ releaseDate: "2004", addedAt: "2024-02-02T12:00:00Z" });
const tracks = [original, compilation, reissue];

describe("defaultKeep", () => {
  it("keeps the earliest release for the original album", () => {
    expect(defaultKeep(tracks, "original")).toBe(original.id);
  });

  it("keeps the first liked", () => {
    expect(defaultKeep(tracks, "first")).toBe(reissue.id);
  });

  it("keeps the latest release for the most recent version", () => {
    expect(defaultKeep(tracks, "recent")).toBe(reissue.id);
  });

  it("breaks a release tie with the like date", () => {
    const early = track({ releaseDate: "2001", addedAt: "2020-01-01T12:00:00Z" });
    const late = track({ releaseDate: "2001", addedAt: "2021-01-01T12:00:00Z" });
    expect(defaultKeep([late, early], "original")).toBe(early.id);
    expect(defaultKeep([early, late], "recent")).toBe(late.id);
  });

  it("never prefers a track without a release date", () => {
    const undated = track({ releaseDate: null, addedAt: "2019-01-01T12:00:00Z" });
    expect(defaultKeep([undated, reissue], "original")).toBe(reissue.id);
    expect(defaultKeep([undated, original], "recent")).toBe(original.id);
  });

  it("keeps nothing in an empty group", () => {
    expect(defaultKeep([], "original")).toBeNull();
  });

  describe("remaster rule", () => {
    const normal = track({ releaseDate: "1994-03-01", addedAt: "2020-01-01T12:00:00Z" });

    it("keeps the remaster over the normal version", () => {
      const remaster = track({ releaseDate: "2012-05-01", versionLabel: "2012 Remaster" });
      expect(defaultKeep([normal, remaster], "remaster")).toBe(remaster.id);
    });

    it("recognises a remaster from the album alone", () => {
      const remaster = track({ releaseDate: "2012-05-01", album: "Bad (Remastered)" });
      expect(defaultKeep([normal, remaster], "remaster")).toBe(remaster.id);
    });

    it("keeps the most recent of two remasters", () => {
      const older = track({ versionLabel: "Remastered 2009", releaseDate: "2009-01-01" });
      const newer = track({ versionLabel: "2021 Remaster", releaseDate: "2021-01-01" });
      expect(defaultKeep([newer, older], "remaster")).toBe(newer.id);
      expect(defaultKeep([older, newer], "remaster")).toBe(newer.id);
    });

    it("ranks a remaster without a year below one with a year", () => {
      const noYear = track({ versionLabel: "Remastered", releaseDate: "1990-01-01" });
      const withYear = track({ versionLabel: "2011 Remaster", releaseDate: "2011-01-01" });
      expect(defaultKeep([noYear, withYear], "remaster")).toBe(withYear.id);
    });

    it("breaks a tie between remasters with the original rule", () => {
      const early = track({ versionLabel: "2011 Remaster", releaseDate: "2011-01-01" });
      const late = track({ versionLabel: "2011 Remaster", releaseDate: "2015-01-01" });
      expect(defaultKeep([late, early], "remaster")).toBe(early.id);
    });

    it("falls back to the original rule without a remaster", () => {
      expect(defaultKeep(tracks, "remaster")).toBe(original.id);
    });
  });
});
