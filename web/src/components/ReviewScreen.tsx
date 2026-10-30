import { useEffect, useId, useMemo, useState } from "react";
import {
  localProblem,
  ProblemError,
  removeTracks,
  type ScanResult,
  type Track,
  UnauthenticatedError,
} from "../api/client";
import { countTracks, formatDateTime } from "../format";
import { logger } from "../logger";
import {
  type Choices,
  loadChoices,
  saveChoices,
  setDismissed,
  setKeep,
  setPromoted,
  setRule,
} from "../review/choices";
import { downloadCsv, tracksCsv } from "../review/csv";
import { KEEP_RULE_LABELS, KEEP_RULES, zKeepRule } from "../review/keep";
import {
  type ReviewGroup,
  reviewGroups,
  TAB_FILTERS,
  TAB_LABELS,
  TABS,
  type Tab,
  tracksToRemove,
} from "../review/model";
import { GroupCard } from "./GroupCard";
import { Header } from "./Header";
import { RemovalBar, type RemovalStep } from "./RemovalBar";
import styles from "./ReviewScreen.module.css";
import ui from "./ui.module.css";

export interface ReviewScreenProps {
  result: ScanResult;
  scannedAt: Date;
  onRescan(): void;
  onLogout(): void;
  onUnauthenticated(): void;
}

export function ReviewScreen({
  result,
  scannedAt,
  onRescan,
  onLogout,
  onUnauthenticated,
}: ReviewScreenProps) {
  const ruleId = useId();
  const [choices, setChoices] = useState<Choices>(loadChoices);
  const [tab, setTab] = useState<Tab>("duplicates");
  const [removed, setRemoved] = useState<ReadonlySet<string>>(() => new Set());
  const [step, setStep] = useState<RemovalStep>({ kind: "idle" });

  useEffect(() => saveChoices(choices), [choices]);

  const groups = useMemo(
    () => reviewGroups(result.groups, choices, removed),
    [result, choices, removed],
  );
  const marked = useMemo(() => tracksToRemove(groups), [groups]);
  const tracks = useMemo(
    () => new Map(result.groups.flatMap((group) => group.tracks.map((t) => [t.id, t] as const))),
    [result],
  );
  const count = (tab: Tab) => groups.filter(TAB_FILTERS[tab]).length;
  const visible = groups.filter(TAB_FILTERS[tab]);

  const choose = (change: (choices: Choices) => Choices) => {
    if (step.kind !== "removing") {
      setChoices(change);
      setStep({ kind: "idle" });
    }
  };
  const toggle = (group: ReviewGroup) =>
    choose((current) =>
      group.confidence === "variant"
        ? setPromoted(current, group.key, !group.promoted)
        : setDismissed(current, group.key, !group.dismissed),
    );
  const keep = (group: ReviewGroup, trackId: string) =>
    choose((current) => setKeep(current, group.key, trackId));

  const exportList = () => {
    const day = new Date().toISOString().slice(0, 10);
    downloadCsv(`spoticlean-a-retirer-${day}.csv`, tracksCsv(marked));
  };

  const remove = async () => {
    const ids = marked.map((track) => track.id);
    setStep({ kind: "removing", count: ids.length });
    try {
      const outcome = await removeTracks(ids);
      setRemoved((previous) => new Set([...previous, ...outcome.removed]));
      setStep(
        outcome.kind === "complete"
          ? { kind: "done", removed: outcome.removed.length }
          : { kind: "partial", removed: outcome.removed.length, failed: outcome.failed },
      );
      logger.info("removal.done", { kind: outcome.kind, removed: outcome.removed.length });
    } catch (error) {
      if (error instanceof UnauthenticatedError) {
        onUnauthenticated();
        return;
      }
      setStep({
        kind: "failed",
        problem:
          error instanceof ProblemError ? error.problem : localProblem("internal", String(error)),
      });
    }
  };

  return (
    <>
      <title>Revue des doublons · spoticlean</title>
      <Header elevated onLogout={onLogout} />
      <main className={styles.main}>
        <section className={styles.intro}>
          <h1 className={styles.title}>
            {marked.length > 0
              ? `${countTracks(marked.length)} en double dans tes likes`
              : "Aucun doublon à retirer"}
          </h1>
          <p className={styles.lead}>
            {countTracks(result.scanned)} likés analysés le {formatDateTime(scannedAt)} ·{" "}
            <button type="button" className={ui.link} onClick={onRescan}>
              Relancer l'analyse
            </button>
          </p>
        </section>

        <dl aria-label="Résumé" className={styles.summary}>
          <div className={`${styles.stat} ${styles.certain}`}>
            <dt>Groupes certains</dt>
            <dd>{count("certain")}</dd>
          </div>
          <div className={`${styles.stat} ${styles.probable}`}>
            <dt>Groupes probables</dt>
            <dd>{count("probable")}</dd>
          </div>
          <div className={`${styles.stat} ${styles.variant}`}>
            <dt>Variantes (pas des doublons)</dt>
            <dd>{count("variant")}</dd>
          </div>
          <div className={styles.total}>
            <dt>Titres à retirer</dt>
            <dd>{marked.length}</dd>
          </div>
        </dl>

        <section aria-label="Filtres" className={styles.filters}>
          <div className={styles.tabs}>
            {TABS.map((key) => (
              <button
                key={key}
                type="button"
                className={styles.tab}
                aria-pressed={tab === key}
                onClick={() => setTab(key)}
              >
                {TAB_LABELS[key]} <span className={styles.tabCount}>{count(key)}</span>
              </button>
            ))}
          </div>
          <div className={styles.rule}>
            <label htmlFor={ruleId}>Titre à garder par défaut</label>
            <select
              id={ruleId}
              value={choices.rule}
              onChange={(event) => {
                const rule = zKeepRule.safeParse(event.target.value);
                if (rule.success) {
                  choose((current) => setRule(current, rule.data));
                }
              }}
            >
              {KEEP_RULES.map((rule) => (
                <option key={rule} value={rule}>
                  {KEEP_RULE_LABELS[rule]}
                </option>
              ))}
            </select>
          </div>
        </section>

        <section aria-label="Groupes" className={styles.groups}>
          {visible.length === 0 && <p className={styles.empty}>Rien dans cette catégorie.</p>}
          {visible.map((group) => (
            <GroupCard key={group.key} group={group} onToggle={toggle} onKeep={keep} />
          ))}
        </section>
      </main>
      <RemovalBar
        step={step}
        count={marked.length}
        track={(id): Track | undefined => tracks.get(id)}
        onExport={exportList}
        onAsk={() => setStep({ kind: "confirm" })}
        onConfirm={() => void remove()}
        onClose={() => setStep({ kind: "idle" })}
      />
    </>
  );
}
