import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Group, ScanResult } from "../api/client";
import { STORAGE_KEY } from "../review/choices";
import { problem, stubFetch } from "../test/fakes";
import { group, scanResult, track } from "../test/fixtures";
import { ReviewScreen } from "./ReviewScreen";

const original = track({
  title: "Marée haute",
  album: "Grand Large",
  releaseDate: "2001",
  addedAt: "2024-05-01T12:00:00Z",
});
const remaster = track({
  title: "Marée haute - Remastered 2021",
  album: "Grand Large (Remastered)",
  releaseDate: "2021-02-05",
  versionLabel: "Remastered 2021",
  addedAt: "2022-01-01T12:00:00Z",
});
const certain = group({ key: "tide", confidence: "certain", tracks: [original, remaster] });

const dawnA = track({
  title: "Aube",
  artists: ["Lune Rousse"],
  album: "Nuits",
  releaseDate: "2010-01-01",
});
const dawnB = track({
  title: "Aube",
  artists: ["Lune Rousse"],
  album: "Best Of",
  releaseDate: "2015-01-01",
  durationMs: 217_000,
});
const probable = group({
  key: "dawn",
  confidence: "probable",
  durationGapMs: 3_000,
  tracks: [dawnA, dawnB],
});

const studio = track({ title: "Rivière", artists: ["Les Galets"], album: "Courants" });
const live = track({
  title: "Rivière - Live",
  artists: ["Les Galets"],
  album: "En concert",
  versionLabel: "Live",
});
const variant = group({ key: "river", confidence: "variant", tracks: [studio, live] });

function renderReview(groups: Group[] = [certain, probable, variant], result?: ScanResult) {
  const onUnauthenticated = vi.fn();
  const view = render(
    <ReviewScreen
      result={result ?? scanResult(groups, 4321)}
      scannedAt={new Date(2026, 9, 30, 18, 8)}
      onRescan={vi.fn()}
      onLogout={vi.fn()}
      onUnauthenticated={onUnauthenticated}
    />,
  );
  return { ...view, onUnauthenticated };
}

const card = (title: string) => {
  const heading = screen.getByRole("heading", { level: 2, name: new RegExp(`^${title} —`) });
  const article = heading.closest("article");
  if (article === null) {
    throw new Error(`no card for ${title}`);
  }
  return within(article);
};

const bar = () => within(screen.getByRole("region", { name: "Retrait" }));

describe("ReviewScreen", () => {
  it("summarises the scan", () => {
    renderReview();
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
      "2 titres en double dans tes likes",
    );
    expect(screen.getByText(/titres likés analysés le 30\/10\/2026 à 18:08/)).toHaveTextContent(
      /^4\s321/u,
    );
    expect(card("Marée haute").getByText("Même ISRC")).toBeInTheDocument();
    expect(card("Aube").getByText("Même titre et artiste · 3 s d'écart")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /^Rivière/ })).not.toBeInTheDocument();
  });

  it("filters by tab", async () => {
    renderReview();
    await userEvent.click(screen.getByRole("button", { name: "Variantes 1" }));
    expect(screen.getByRole("button", { name: "Variantes 1" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(card("Rivière").getByText("Version live")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /^Marée haute/ })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Ignorés 0" }));
    expect(screen.getByText("Rien dans cette catégorie.")).toBeInTheDocument();
  });

  it("shows versions, dates at their precision, and emphasises what differs", () => {
    renderReview([certain]);
    const tide = card("Marée haute");
    expect(tide.getByText("Original")).toBeInTheDocument();
    expect(tide.getByText("Remastered 2021")).toBeInTheDocument();
    expect(tide.getByText("2001")).toBeInTheDocument();
    expect(tide.getByText("05/02/2021")).toBeInTheDocument();
    expect(tide.getByText("Grand Large").className).toMatch(/differs/);
    expect(tide.getAllByText("3:34")[0]?.className).toMatch(/same/);
  });

  it("keeps the track the user picks, and remembers it", async () => {
    const { unmount } = renderReview([certain]);
    const radios = card("Marée haute").getAllByRole("radio");
    expect(radios[1]).toBeChecked();
    await userEvent.click(card("Marée haute").getByText("Grand Large", { selector: "span" }));
    expect(card("Marée haute").getAllByRole("radio")[0]).toBeChecked();
    expect(JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}").keep).toEqual({
      tide: original.id,
    });
    unmount();
    renderReview([certain]);
    expect(card("Marée haute").getAllByRole("radio")[0]).toBeChecked();
  });

  it("applies the keep rule chosen", async () => {
    renderReview([certain]);
    await userEvent.selectOptions(screen.getByLabelText("Titre à garder par défaut"), "original");
    expect(card("Marée haute").getAllByRole("radio")[0]).toBeChecked();
    expect(JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}").rule).toBe("original");
  });

  it("dismisses and promotes groups", async () => {
    renderReview();
    await userEvent.click(card("Aube").getByRole("button", { name: "Pas un doublon : Aube" }));
    expect(screen.queryByRole("heading", { name: /^Aube/ })).not.toBeInTheDocument();
    expect(bar().getByText("1 titre")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /^Variantes/ }));
    await userEvent.click(
      card("Rivière").getByRole("button", { name: "C'est un doublon : Rivière" }),
    );
    await userEvent.click(screen.getByRole("button", { name: /^Doublons/ }));
    expect(card("Rivière").getByText("Manuel")).toBeInTheDocument();
    expect(bar().getByText("2 titres")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /^Ignorés/ }));
    await userEvent.click(card("Aube").getByRole("button", { name: "Restaurer : Aube" }));
    const stored = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}");
    expect(stored.dismissed).toEqual([]);
    expect(stored.promoted).toEqual(["river"]);
  });

  it("removes the marked tracks after confirmation", async () => {
    const calls = stubFetch([
      {
        method: "POST",
        url: "/api/v1/removals",
        status: 200,
        body: { removed: [remaster.id, dawnB.id], failed: [] },
      },
    ]);
    renderReview();
    await userEvent.click(bar().getByRole("button", { name: "Retirer 2 titres" }));
    expect(bar().getByText("Retirer 2 titres de tes likes ?")).toBeInTheDocument();
    await userEvent.click(bar().getByRole("button", { name: "Confirmer le retrait" }));

    expect(await bar().findByText("2 titres retirés de tes likes.")).toBeInTheDocument();
    expect(calls[0]?.body).toEqual({ trackIds: [original.id, dawnB.id] });
    expect(screen.queryByRole("heading", { name: /^Marée haute/ })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Aucun doublon à retirer");
    await userEvent.click(bar().getByRole("button", { name: "Fermer" }));
    expect(bar().getByRole("button", { name: "Retirer 0 titre" })).toBeDisabled();
  });

  it("can be cancelled before anything is sent", async () => {
    const calls = stubFetch([]);
    renderReview();
    await userEvent.click(bar().getByRole("button", { name: "Retirer 2 titres" }));
    await userEvent.click(bar().getByRole("button", { name: "Annuler" }));
    expect(bar().getByRole("button", { name: "Retirer 2 titres" })).toBeEnabled();
    expect(calls).toEqual([]);
  });

  it("reports a partial removal and keeps the failed tracks", async () => {
    stubFetch([
      {
        method: "POST",
        url: "/api/v1/removals",
        status: 207,
        body: {
          removed: [remaster.id],
          failed: [{ trackIds: [dawnB.id], reason: "spotifyRateLimited", detail: "slow down" }],
        },
      },
    ]);
    renderReview();
    await userEvent.click(bar().getByRole("button", { name: "Retirer 2 titres" }));
    await userEvent.click(bar().getByRole("button", { name: "Confirmer le retrait" }));

    expect(
      await bar().findByText(/1 titre retiré, 1 titre toujours dans tes likes/),
    ).toBeInTheDocument();
    expect(bar().getByRole("listitem")).toHaveTextContent(
      "Aube — Lune Rousse : Spotify limite les requêtes",
    );
    expect(screen.queryByRole("heading", { name: /^Marée haute/ })).not.toBeInTheDocument();
    expect(card("Aube").getAllByRole("radio")[1]).not.toBeChecked();

    await userEvent.click(bar().getByRole("button", { name: "Fermer" }));
    expect(bar().getByRole("button", { name: "Retirer 1 titre" })).toBeEnabled();
  });

  it("reports a removal that failed entirely", async () => {
    stubFetch([
      {
        method: "POST",
        url: "/api/v1/removals",
        status: 502,
        body: problem(502, "spotifyUnavailable", "gave up"),
      },
    ]);
    renderReview();
    await userEvent.click(bar().getByRole("button", { name: "Retirer 2 titres" }));
    await userEvent.click(bar().getByRole("button", { name: "Confirmer le retrait" }));
    const alert = await bar().findByRole("alert");
    expect(alert).toHaveTextContent(
      "Aucun titre n'a été retiré. Spotify ne répond pas correctement.",
    );
    expect(alert).toHaveTextContent("gave up");
    expect(card("Marée haute")).toBeTruthy();
  });

  it("sends the user back to the login screen on a 401", async () => {
    stubFetch([
      {
        method: "POST",
        url: "/api/v1/removals",
        status: 401,
        body: problem(401, "sessionExpired"),
      },
    ]);
    const { onUnauthenticated } = renderReview();
    await userEvent.click(bar().getByRole("button", { name: "Retirer 2 titres" }));
    await userEvent.click(bar().getByRole("button", { name: "Confirmer le retrait" }));
    await vi.waitFor(() => expect(onUnauthenticated).toHaveBeenCalledOnce());
  });

  it("exports the marked tracks", async () => {
    const createObjectURL = vi.fn((_: Blob) => "blob:list");
    Object.assign(URL, { createObjectURL, revokeObjectURL: vi.fn() });
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    renderReview();
    await userEvent.click(bar().getByRole("button", { name: "Exporter la liste" }));
    expect(click).toHaveBeenCalledOnce();
    const csv = await createObjectURL.mock.calls[0]?.[0].text();
    expect(csv?.split("\r\n").filter(Boolean)).toHaveLength(3);
    expect(csv).toContain(original.id);
    expect(csv).toContain(dawnB.id);
  });
});
