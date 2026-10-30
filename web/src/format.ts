const numberFormat = new Intl.NumberFormat("fr-FR");
const dateFormat = new Intl.DateTimeFormat("fr-FR", {
  day: "2-digit",
  month: "2-digit",
  year: "numeric",
});
const timeFormat = new Intl.DateTimeFormat("fr-FR", { hour: "2-digit", minute: "2-digit" });

export function formatNumber(value: number): string {
  return numberFormat.format(value);
}

/** "1 titre", "2 233 titres". */
export function countTracks(count: number): string {
  return `${formatNumber(count)} ${agree(count, "titre", "titres")}`;
}

/**
 * A release date at the precision Spotify gives it: `1987`, `08/1987` or
 * `31/08/1987`.
 */
export function formatReleaseDate(releaseDate: string | null | undefined): string {
  if (!releaseDate) {
    return "—";
  }
  const [year, month, day] = releaseDate.split("-");
  if (day !== undefined && month !== undefined) {
    return `${day}/${month}/${year}`;
  }
  if (month !== undefined) {
    return `${month}/${year}`;
  }
  return releaseDate;
}

/** An RFC 3339 instant as a local date, `22/05/2024`. */
export function formatDay(instant: string | Date): string {
  const date = typeof instant === "string" ? new Date(instant) : instant;
  return Number.isNaN(date.getTime()) ? "—" : dateFormat.format(date);
}

/** `30/10/2026 à 18:08`. */
export function formatDateTime(date: Date): string {
  return `${dateFormat.format(date)} à ${timeFormat.format(date)}`;
}

/** `3:32`, or `1:02:05` past an hour. */
export function formatDuration(durationMs: number): string {
  const totalSeconds = Math.round(durationMs / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = String(totalSeconds % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}`
    : `${minutes}:${seconds}`;
}

/** The singular or the plural form, as French agrees it with `count`. */
export function agree(count: number, singular: string, plural: string): string {
  return count > 1 ? plural : singular;
}
