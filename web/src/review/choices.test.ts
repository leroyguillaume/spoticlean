import { describe, expect, it, vi } from "vitest";
import {
  DEFAULT_CHOICES,
  loadChoices,
  STORAGE_KEY,
  saveChoices,
  setDismissed,
  setKeep,
  setPromoted,
  setRule,
} from "./choices";

describe("persisted choices", () => {
  it("start from the defaults when nothing is stored", () => {
    expect(loadChoices()).toEqual(DEFAULT_CHOICES);
  });

  it("survive a round trip", () => {
    const choices = setKeep(
      setPromoted(setDismissed(setRule(DEFAULT_CHOICES, "first"), "a", true), "b", true),
      "c",
      "track-c",
    );
    saveChoices(choices);
    expect(loadChoices()).toEqual({
      rule: "first",
      dismissed: ["a"],
      promoted: ["b"],
      keep: { c: "track-c" },
    });
  });

  it("fall back to the defaults on corrupt storage", () => {
    window.localStorage.setItem(STORAGE_KEY, "{not json");
    expect(loadChoices()).toEqual(DEFAULT_CHOICES);
    window.localStorage.setItem(STORAGE_KEY, "42");
    expect(loadChoices()).toEqual(DEFAULT_CHOICES);
  });

  it("keep the valid parts of a damaged record", () => {
    window.localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ rule: "loudest", dismissed: ["a"], promoted: "b", keep: { c: 3 } }),
    );
    expect(loadChoices()).toEqual({ rule: "remaster", dismissed: ["a"], promoted: [], keep: {} });
  });

  it("default to the remaster rule when the stored rule is invalid or absent", () => {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ dismissed: [] }));
    expect(loadChoices().rule).toBe("remaster");
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ rule: "loudest" }));
    expect(loadChoices().rule).toBe("remaster");
  });

  it("preserve a stored existing rule", () => {
    for (const rule of ["original", "first", "recent"] as const) {
      saveChoices(setRule(DEFAULT_CHOICES, rule));
      expect(loadChoices().rule).toBe(rule);
    }
  });

  it("work without storage", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("denied", "SecurityError");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("full", "QuotaExceededError");
    });
    expect(loadChoices()).toEqual(DEFAULT_CHOICES);
    expect(() => saveChoices(DEFAULT_CHOICES)).not.toThrow();
  });

  it("drop the per-group picks when the rule changes", () => {
    const picked = setKeep(DEFAULT_CHOICES, "a", "track-a");
    expect(setRule(picked, "recent").keep).toEqual({});
  });

  it("toggle a key once", () => {
    const twice = setDismissed(setDismissed(DEFAULT_CHOICES, "a", true), "a", true);
    expect(twice.dismissed).toEqual(["a"]);
    expect(setDismissed(twice, "a", false).dismissed).toEqual([]);
  });
});
