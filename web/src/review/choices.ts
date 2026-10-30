import { z } from "zod";
import { logger } from "../logger";
import { zKeepRule } from "./keep";

/**
 * What the user decided, by group key. A key whose group is gone from the
 * scan is kept but has no effect.
 */
const zChoices = z.object({
  rule: zKeepRule.catch("original"),
  dismissed: z.array(z.string()).catch([]),
  promoted: z.array(z.string()).catch([]),
  keep: z.record(z.string(), z.string()).catch({}),
});

export type Choices = z.infer<typeof zChoices>;

export const STORAGE_KEY = "spoticlean.choices";

export const DEFAULT_CHOICES: Choices = { rule: "original", dismissed: [], promoted: [], keep: {} };

export function loadChoices(): Choices {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (stored === null) {
      return DEFAULT_CHOICES;
    }
    const parsed = zChoices.safeParse(JSON.parse(stored));
    if (parsed.success) {
      return parsed.data;
    }
    logger.warn("choices.invalid", { issues: parsed.error.issues.length });
  } catch (error) {
    logger.warn("choices.unreadable", { error: String(error) });
  }
  return DEFAULT_CHOICES;
}

export function saveChoices(choices: Choices): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(choices));
  } catch (error) {
    logger.warn("choices.unwritable", { error: String(error) });
  }
}

const toggled = (keys: readonly string[], key: string, on: boolean): string[] =>
  on ? [...new Set([...keys, key])] : keys.filter((other) => other !== key);

export function setDismissed(choices: Choices, key: string, dismissed: boolean): Choices {
  return { ...choices, dismissed: toggled(choices.dismissed, key, dismissed) };
}

export function setPromoted(choices: Choices, key: string, promoted: boolean): Choices {
  return { ...choices, promoted: toggled(choices.promoted, key, promoted) };
}

export function setKeep(choices: Choices, key: string, trackId: string): Choices {
  return { ...choices, keep: { ...choices.keep, [key]: trackId } };
}

/** A new rule drops the per-group picks, which were made against the old one. */
export function setRule(choices: Choices, rule: Choices["rule"]): Choices {
  return { ...choices, rule, keep: {} };
}
