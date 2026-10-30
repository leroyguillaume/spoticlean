import { useId } from "react";
import type { Track } from "../api/client";
import { formatDay, formatDuration, formatReleaseDate } from "../format";
import type { Badge, ReviewGroup } from "../review/model";
import styles from "./GroupCard.module.css";
import ui from "./ui.module.css";

const BADGES: Record<Badge, { label: string; className: string | undefined }> = {
  certain: { label: "Certain", className: ui.certain },
  probable: { label: "Probable", className: ui.probable },
  variant: { label: "Variante", className: ui.variant },
  manual: { label: "Manuel", className: ui.manual },
};

function toggleLabel(group: ReviewGroup): string {
  if (group.dismissed) {
    return "Restaurer";
  }
  if (group.promoted) {
    return "Annuler";
  }
  return group.confidence === "variant" ? "C'est un doublon" : "Pas un doublon";
}

interface TrackCellsProps {
  track: Track;
  differs: ReviewGroup["differs"];
}

function TrackCells({ track, differs }: TrackCellsProps) {
  const emphasis = (differ: boolean) => (differ ? styles.differs : styles.same);
  const version = track.versionLabel ?? "Original";
  return (
    <>
      <span className={track.versionLabel ? styles.version : styles.original} title={version}>
        {version}
      </span>
      <span className={`${styles.album} ${emphasis(differs.album)}`} title={track.album}>
        {track.album}
      </span>
      <span className={`${styles.value} ${emphasis(differs.releaseDate)}`}>
        {formatReleaseDate(track.releaseDate)}
      </span>
      <span className={`${styles.value} ${emphasis(differs.duration)}`}>
        {formatDuration(track.durationMs)}
      </span>
      <span className={`${styles.value} ${styles.same}`}>{formatDay(track.addedAt)}</span>
    </>
  );
}

function ColumnHeaders() {
  return (
    <div className={styles.header} aria-hidden="true">
      <span />
      <span>Version</span>
      <span>Album</span>
      <span>Sortie</span>
      <span>Durée</span>
      <span>Liké le</span>
    </div>
  );
}

export interface GroupCardProps {
  group: ReviewGroup;
  onToggle(group: ReviewGroup): void;
  onKeep(group: ReviewGroup, trackId: string): void;
}

export function GroupCard({ group, onToggle, onKeep }: GroupCardProps) {
  const badge = BADGES[group.badge];
  const titleId = useId();
  const toggle = toggleLabel(group);

  return (
    <article aria-labelledby={titleId} className={group.dismissed ? styles.dismissed : styles.card}>
      <div className={styles.top}>
        <div className={styles.heading}>
          <div className={styles.tags}>
            <span className={badge.className}>{badge.label}</span>
            <span className={styles.reason}>{group.reason}</span>
          </div>
          <h2 id={titleId} className={styles.title}>
            {group.title} <span className={styles.artist}>— {group.artist}</span>
          </h2>
        </div>
        <button
          type="button"
          className={styles.toggle}
          onClick={() => onToggle(group)}
          aria-label={`${toggle} : ${group.title}`}
        >
          {toggle}
        </button>
      </div>
      <div className={styles.scroller}>
        {group.isDuplicate ? (
          <div className={styles.table} role="radiogroup" aria-labelledby={titleId}>
            <ColumnHeaders />
            {group.tracks.map((track) => {
              const kept = track.id === group.keepId;
              return (
                <label key={track.id} className={kept ? styles.kept : styles.selectable}>
                  <span className={styles.action}>
                    <input
                      type="radio"
                      className={styles.radio}
                      name={titleId}
                      checked={kept}
                      onChange={() => onKeep(group, track.id)}
                      aria-label={`Garder ${track.versionLabel ?? "Original"}, ${track.album}, sortie ${formatReleaseDate(track.releaseDate)}`}
                    />
                    {kept ? "Garder" : "Retirer"}
                  </span>
                  <TrackCells track={track} differs={group.differs} />
                </label>
              );
            })}
          </div>
        ) : (
          <div className={styles.table}>
            <ColumnHeaders />
            {group.tracks.map((track) => (
              <div key={track.id} className={styles.row}>
                <span />
                <TrackCells track={track} differs={group.differs} />
              </div>
            ))}
          </div>
        )}
      </div>
    </article>
  );
}
