import type { FailedRemoval, Problem, Track } from "../api/client";
import { agree, countTracks } from "../format";
import { FAILURE_REASONS, problemMessage } from "../messages";
import styles from "./RemovalBar.module.css";
import ui from "./ui.module.css";

export type RemovalStep =
  | { kind: "idle" }
  | { kind: "confirm" }
  | { kind: "removing"; count: number }
  | { kind: "done"; removed: number }
  | { kind: "partial"; removed: number; failed: FailedRemoval[] }
  | { kind: "failed"; problem: Problem };

export interface RemovalBarProps {
  step: RemovalStep;
  count: number;
  track(id: string): Track | undefined;
  onExport(): void;
  onAsk(): void;
  onConfirm(): void;
  onClose(): void;
}

function Failures({ failed, track }: Pick<RemovalBarProps, "track"> & { failed: FailedRemoval[] }) {
  return (
    <ul className={styles.failures}>
      {failed.flatMap((batch) =>
        batch.trackIds.map((id) => {
          const found = track(id);
          return (
            <li key={id}>
              {found ? `${found.title} — ${found.artists.join(", ")}` : id} :{" "}
              {FAILURE_REASONS[batch.reason]}
            </li>
          );
        }),
      )}
    </ul>
  );
}

function Content({ step, count, track, onExport, onAsk, onConfirm, onClose }: RemovalBarProps) {
  switch (step.kind) {
    case "idle":
      return (
        <>
          <p className={styles.status}>
            <strong>{countTracks(count)}</strong>{" "}
            <span className={styles.muted}>
              {agree(count, "sera retiré", "seront retirés")} de tes titres likés.
            </span>
          </p>
          <div className={styles.actions}>
            <button type="button" className={ui.pill} onClick={onExport} disabled={count === 0}>
              Exporter la liste
            </button>
            <button type="button" className={ui.primary} onClick={onAsk} disabled={count === 0}>
              Retirer {countTracks(count)}
            </button>
          </div>
        </>
      );
    case "confirm":
      return (
        <>
          <p className={styles.status}>
            <strong>Retirer {countTracks(count)} de tes likes ?</strong>{" "}
            <span className={styles.muted}>Les titres gardés ne bougent pas.</span>
          </p>
          <div className={styles.actions}>
            <button type="button" className={ui.pill} onClick={onClose}>
              Annuler
            </button>
            <button type="button" className={ui.danger} onClick={onConfirm}>
              Confirmer le retrait
            </button>
          </div>
        </>
      );
    case "removing":
      return (
        <>
          <p className={styles.status}>
            <strong>Retrait de {countTracks(step.count)} en cours…</strong>
          </p>
          <div className={styles.actions}>
            <button type="button" className={ui.danger} disabled>
              Confirmer le retrait
            </button>
          </div>
        </>
      );
    case "done":
      return (
        <>
          <p className={styles.status}>
            <strong>
              {countTracks(step.removed)} {agree(step.removed, "retiré", "retirés")} de tes likes.
            </strong>
          </p>
          <button type="button" className={ui.pill} onClick={onClose}>
            Fermer
          </button>
        </>
      );
    case "partial": {
      const failed = step.failed.reduce((n, batch) => n + batch.trackIds.length, 0);
      return (
        <>
          <p className={styles.status}>
            <strong>
              {countTracks(step.removed)} {agree(step.removed, "retiré", "retirés")},{" "}
              {countTracks(failed)} toujours dans tes likes.
            </strong>{" "}
            <span className={styles.muted}>
              {agree(failed, "Il reste sélectionné", "Ils restent sélectionnés")} : tu peux
              réessayer.
            </span>
          </p>
          <button type="button" className={ui.pill} onClick={onClose}>
            Fermer
          </button>
          <Failures failed={step.failed} track={track} />
        </>
      );
    }
    case "failed":
      return (
        <>
          <div role="alert" className={ui.alert}>
            <strong>Aucun titre n'a été retiré. {problemMessage(step.problem)}</strong>
            <span className={ui.detail}>{step.problem.detail}</span>
          </div>
          <div className={styles.actions}>
            <button type="button" className={ui.pill} onClick={onClose}>
              Fermer
            </button>
            <button type="button" className={ui.primary} onClick={onConfirm}>
              Réessayer
            </button>
          </div>
        </>
      );
  }
}

export function RemovalBar(props: RemovalBarProps) {
  return (
    <section aria-label="Retrait" className={styles.bar}>
      <div className={styles.inner} aria-live="polite">
        <Content {...props} />
      </div>
    </section>
  );
}
