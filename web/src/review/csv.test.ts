import { describe, expect, it } from "vitest";
import { track } from "../test/fixtures";
import { tracksCsv } from "./csv";

describe("tracksCsv", () => {
  it("writes a header and one quoted row per track", () => {
    const csv = tracksCsv([
      track({
        id: "abcdefghijklmnopqrstuv",
        title: 'Le "grand" soir, part 1',
        artists: ["A", "B"],
        album: "Album",
        releaseDate: "2001",
        durationMs: 1000,
        addedAt: "2023-03-14T12:00:00Z",
        isrc: null,
      }),
    ]);
    expect(csv.startsWith("﻿Titre,Artistes,Album,Sortie,")).toBe(true);
    expect(csv.split("\r\n")[1]).toBe(
      '"Le ""grand"" soir, part 1","A, B",Album,2001,1000,2023-03-14T12:00:00Z,,abcdefghijklmnopqrstuv,https://open.spotify.com/track/abcdefghijklmnopqrstuv',
    );
  });
});
