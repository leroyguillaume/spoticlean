import { describe, expect, it } from "vitest";
import { group, track } from "../test/fixtures";
import { DEFAULT_CHOICES, setDismissed, setKeep, setPromoted } from "./choices";
import { reviewGroups, TAB_FILTERS, TABS, tracksToRemove } from "./model";

const first = track({ releaseDate: "1994", versionLabel: null, album: "Premier Soir" });
const remaster = track({
  title: "Marée haute - Remastered 2014",
  releaseDate: "2014-06-01",
  versionLabel: "Remastered 2014",
  album: "Premier Soir (Remastered)",
});
const certain = group({ key: "certain", confidence: "certain", tracks: [first, remaster] });
const probable = group({
  key: "probable",
  confidence: "probable",
  tracks: [track({ album: "Same" }), track({ album: "Same", durationMs: 215_000 })],
});
const live = track({ versionLabel: "Live", title: "Marée haute - Live" });
const variant = group({ key: "variant", confidence: "variant", tracks: [first, live] });
const groups = [certain, probable, variant];

const counts = (choices = DEFAULT_CHOICES) => {
  const reviewed = reviewGroups(groups, choices, new Set());
  return Object.fromEntries(TABS.map((tab) => [tab, reviewed.filter(TAB_FILTERS[tab]).length]));
};

describe("reviewGroups", () => {
  it("applies the default rule", () => {
    const [reviewed] = reviewGroups([certain], DEFAULT_CHOICES, new Set());
    expect(reviewed?.keepId).toBe(first.id);
    expect(reviewed?.title).toBe("Marée haute");
    expect(tracksToRemove(reviewGroups([certain], DEFAULT_CHOICES, new Set()))).toEqual([remaster]);
  });

  it("lets a pick override the rule", () => {
    const choices = setKeep(DEFAULT_CHOICES, "certain", remaster.id);
    expect(tracksToRemove(reviewGroups([certain], choices, new Set()))).toEqual([first]);
  });

  it("ignores a pick naming a track that is gone", () => {
    const choices = setKeep(DEFAULT_CHOICES, "certain", "gone");
    expect(reviewGroups([certain], choices, new Set())[0]?.keepId).toBe(first.id);
  });

  it("ignores choices about groups the scan did not find", () => {
    const choices = setDismissed(setPromoted(DEFAULT_CHOICES, "elsewhere", true), "nowhere", true);
    expect(counts(choices)).toEqual(counts());
  });

  it("files groups under their tabs", () => {
    expect(counts()).toEqual({ duplicates: 2, certain: 1, probable: 1, variant: 1, ignored: 0 });
  });

  it("moves a dismissed group to the ignored tab", () => {
    const choices = setDismissed(DEFAULT_CHOICES, "probable", true);
    expect(counts(choices)).toEqual({
      duplicates: 1,
      certain: 1,
      probable: 0,
      variant: 1,
      ignored: 1,
    });
    const reviewed = reviewGroups(groups, choices, new Set());
    expect(tracksToRemove(reviewed)).toEqual([remaster]);
  });

  it("turns a promoted variant into a manual duplicate", () => {
    const choices = setPromoted(DEFAULT_CHOICES, "variant", true);
    expect(counts(choices)).toEqual({
      duplicates: 3,
      certain: 1,
      probable: 1,
      variant: 0,
      ignored: 0,
    });
    const promoted = reviewGroups(groups, choices, new Set()).find((g) => g.key === "variant");
    expect(promoted?.badge).toBe("manual");
    expect(promoted?.keepId).not.toBeNull();
  });

  it("marks variants for nothing until promoted", () => {
    const [reviewed] = reviewGroups([variant], DEFAULT_CHOICES, new Set());
    expect(reviewed?.isDuplicate).toBe(false);
    expect(reviewed?.keepId).toBeNull();
    expect(tracksToRemove(reviewGroups([variant], DEFAULT_CHOICES, new Set()))).toEqual([]);
  });

  it("lists a track marked twice once", () => {
    const choices = setKeep(setPromoted(DEFAULT_CHOICES, "variant", true), "variant", live.id);
    const marked = tracksToRemove(reviewGroups([certain, variant], choices, new Set()));
    expect(marked.map((t) => t.id)).toEqual([remaster.id, first.id]);
  });

  it("leaves out removed tracks, and groups left alone", () => {
    const reviewed = reviewGroups(groups, DEFAULT_CHOICES, new Set([remaster.id]));
    expect(reviewed.map((g) => g.key)).toEqual(["probable", "variant"]);
  });

  it("emphasises the columns that differ", () => {
    const [cert] = reviewGroups([certain], DEFAULT_CHOICES, new Set());
    expect(cert?.differs).toEqual({ album: true, releaseDate: true, duration: false });
    const [prob] = reviewGroups([probable], DEFAULT_CHOICES, new Set());
    expect(prob?.differs).toEqual({ album: false, releaseDate: false, duration: true });
  });
});
