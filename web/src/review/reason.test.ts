import { describe, expect, it } from "vitest";
import { group, track } from "../test/fixtures";
import { groupReason } from "./reason";

describe("groupReason", () => {
  it("names the ISRC for certain groups", () => {
    expect(groupReason(group({ confidence: "certain", tracks: [track(), track()] }))).toBe(
      "Même ISRC",
    );
  });

  it("gives the duration gap of probable groups", () => {
    const tracks = [track(), track()];
    expect(groupReason(group({ confidence: "probable", durationGapMs: 3_600, tracks }))).toBe(
      "Même titre et artiste · 4 s d'écart",
    );
    expect(groupReason(group({ confidence: "probable", durationGapMs: 300, tracks }))).toBe(
      "Même titre et artiste · même durée",
    );
  });

  it("describes variants from their version labels", () => {
    const variant = (...labels: (string | null)[]) =>
      groupReason(
        group({
          confidence: "variant",
          tracks: labels.map((versionLabel) => track({ versionLabel })),
        }),
      );
    expect(variant(null, "Live at the Olympia")).toBe("Version live");
    expect(variant(null, "Acoustic")).toBe("Version acoustique");
    expect(variant(null, "International Mix", "Radio Edit")).toBe("Versions mix et edit");
    expect(variant("2011 Remaster", "Live · 2011 Remaster")).toBe("Version live");
    expect(variant(null, "Midnight Version")).toBe("Version « Midnight Version »");
    expect(variant(null, null)).toBe("Artistes crédités différents");
  });
});
