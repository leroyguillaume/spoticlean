import type { Track } from "../api/client";

const COLUMNS: ReadonlyArray<readonly [string, (track: Track) => string]> = [
  ["Titre", (track) => track.title],
  ["Artistes", (track) => track.artists.join(", ")],
  ["Album", (track) => track.album],
  ["Sortie", (track) => track.releaseDate ?? ""],
  ["Durée (ms)", (track) => String(track.durationMs)],
  ["Liké le", (track) => track.addedAt],
  ["ISRC", (track) => track.isrc ?? ""],
  ["ID Spotify", (track) => track.id],
  ["Lien", (track) => `https://open.spotify.com/track/${track.id}`],
];

function cell(value: string): string {
  return /[",\r\n]/.test(value) ? `"${value.replaceAll('"', '""')}"` : value;
}

/** RFC 4180, with a byte order mark so that spreadsheets read it as UTF-8. */
export function tracksCsv(tracks: readonly Track[]): string {
  const rows = [
    COLUMNS.map(([header]) => header),
    ...tracks.map((track) => COLUMNS.map(([, value]) => value(track))),
  ];
  return `﻿${rows.map((row) => row.map(cell).join(",")).join("\r\n")}\r\n`;
}

export function downloadCsv(filename: string, content: string): void {
  const url = URL.createObjectURL(new Blob([content], { type: "text/csv;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  // Firefox starts the download after `click()` returns, and fails it if the
  // URL is already revoked.
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
