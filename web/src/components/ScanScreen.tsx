import { useEffect, useEffectEvent, useState } from "react";
import {
  getSession,
  localProblem,
  type Problem,
  ProblemError,
  problemCode,
  type ScanProgress,
  type ScanResult,
  startScan,
} from "../api/client";
import { formatNumber } from "../format";
import { problemMessage } from "../messages";
import { Header } from "./Header";
import styles from "./ScanScreen.module.css";
import ui from "./ui.module.css";

const UNAUTHENTICATED = new Set(["unauthenticated", "sessionExpired"]);

type StepState = "done" | "active" | "pending";

function StepIcon({ state }: { state: StepState }) {
  switch (state) {
    case "done":
      return (
        <svg
          width="20"
          height="20"
          viewBox="0 0 20 20"
          fill="none"
          stroke="var(--color-accent)"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <circle cx="10" cy="10" r="8" />
          <path d="M6.5 10.5l2.3 2.2 4.7-5" />
        </svg>
      );
    case "active":
      return (
        <svg
          width="20"
          height="20"
          viewBox="0 0 20 20"
          fill="none"
          stroke="var(--color-accent)"
          strokeWidth="2"
          strokeLinecap="round"
          className={styles.spinner}
          aria-hidden="true"
        >
          <circle cx="10" cy="10" r="8" stroke="var(--color-accent-line)" />
          <path d="M10 2a8 8 0 0 1 8 8" />
        </svg>
      );
    case "pending":
      return (
        <svg
          width="20"
          height="20"
          viewBox="0 0 20 20"
          fill="none"
          stroke="var(--color-faint)"
          strokeWidth="2"
          aria-hidden="true"
        >
          <circle cx="10" cy="10" r="8" />
        </svg>
      );
  }
}

const STEP_STATUS: Record<StepState, string> = {
  done: "terminé",
  active: "en cours",
  pending: "à venir",
};

function Step({ state, label }: { state: StepState; label: string }) {
  return (
    <li className={styles[state]}>
      <StepIcon state={state} />
      <span>{label}</span>
      <span className="visually-hidden">({STEP_STATUS[state]})</span>
    </li>
  );
}

/**
 * The server only reports the fetch: the grouping that follows is shown as
 * the next step until the result arrives.
 */
function steps(progress: ScanProgress | null, failed: boolean): StepState[] {
  const current = failed ? "pending" : "active";
  if (progress === null) {
    return [current, "pending", "pending", "pending"];
  }
  if (progress.fetched < progress.total) {
    return ["done", current, "pending", "pending"];
  }
  return ["done", "done", current, "pending"];
}

const STEP_LABELS = [
  "Connexion à Spotify",
  "Récupération des titres likés",
  "Regroupement par ISRC",
  "Comparaison des titres, artistes et durées",
];

export interface ScanScreenProps {
  onResult(result: ScanResult): void;
  onUnauthenticated(): void;
  onLogout(): void;
}

export function ScanScreen({ onResult, onUnauthenticated, onLogout }: ScanScreenProps) {
  const [attempt, setAttempt] = useState(0);
  return (
    <>
      <title>Analyse en cours · spoticlean</title>
      <Header onLogout={onLogout} />
      <main className={styles.main}>
        <ScanPanel
          key={attempt}
          onResult={onResult}
          onUnauthenticated={onUnauthenticated}
          onRetry={() => setAttempt((n) => n + 1)}
        />
      </main>
    </>
  );
}

interface ScanPanelProps {
  onResult(result: ScanResult): void;
  onUnauthenticated(): void;
  onRetry(): void;
}

function ScanPanel({ onResult, onUnauthenticated, onRetry }: ScanPanelProps) {
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);

  const finish = useEffectEvent((result: ScanResult) => onResult(result));
  const fail = useEffectEvent((failure: Problem) => {
    if (UNAUTHENTICATED.has(problemCode(failure))) {
      onUnauthenticated();
    } else {
      setProblem(failure);
    }
  });
  const disconnected = useEffectEvent(async () => {
    try {
      if ((await getSession()) === null) {
        onUnauthenticated();
        return;
      }
      setProblem(localProblem("disconnected", "the scan stream closed before its result"));
    } catch (error) {
      setProblem(
        error instanceof ProblemError ? error.problem : localProblem("internal", String(error)),
      );
    }
  });

  useEffect(() => {
    let active = true;
    const stop = startScan({
      onProgress: setProgress,
      onResult: finish,
      onProblem: fail,
      onDisconnect: () => {
        if (active) {
          void disconnected();
        }
      },
    });
    return () => {
      active = false;
      stop();
    };
  }, []);

  const states = steps(progress, problem !== null);
  const total = progress?.total ?? null;
  const fetched = progress?.fetched ?? 0;
  const percent = total === null ? 0 : total === 0 ? 100 : Math.round((fetched / total) * 100);

  return (
    <section aria-labelledby="scan-title" className={styles.panel}>
      <div className={styles.heading}>
        <h1 id="scan-title" className={styles.title}>
          Analyse de ta bibliothèque
        </h1>
        <p className={styles.lead}>Quelques secondes, même pour une grosse collection.</p>
      </div>
      <div className={styles.progress}>
        <div className={styles.counter}>
          <span className={styles.label}>Titres likés récupérés</span>
          <span className={styles.count}>
            {formatNumber(fetched)} / {total === null ? "…" : formatNumber(total)}
          </span>
        </div>
        <div
          role="progressbar"
          aria-label="Titres likés récupérés"
          aria-valuemin={0}
          aria-valuemax={total ?? undefined}
          aria-valuenow={total === null ? undefined : fetched}
          className={styles.bar}
        >
          <div className={styles.fill} style={{ width: `${percent}%` }} />
        </div>
      </div>
      <ol className={styles.steps}>
        {STEP_LABELS.map((label, index) => (
          <Step key={label} label={label} state={states[index] ?? "pending"} />
        ))}
      </ol>
      {problem && (
        <div role="alert" className={ui.alert}>
          <span>{problemMessage(problem)}</span>
          <span className={ui.detail}>{problem.detail}</span>
        </div>
      )}
      {problem && (
        <button type="button" className={styles.retry} onClick={onRetry}>
          Réessayer
        </button>
      )}
    </section>
  );
}
