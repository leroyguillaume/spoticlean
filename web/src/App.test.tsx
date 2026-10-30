import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { App, takeAuthError } from "./App";
import { FakeEventSource, problem, stubFetch } from "./test/fakes";
import { group, scanResult, track } from "./test/fixtures";

const SESSION = { method: "GET", url: "/api/v1/session" };
const loggedIn = { ...SESSION, status: 200, body: { scopes: ["user-library-read"] } };
const loggedOut = { ...SESSION, status: 401, body: problem(401, "unauthenticated") };

describe("startup", () => {
  it("shows the login screen without a session", async () => {
    stubFetch([loggedOut]);
    render(<App authError={null} />);
    const login = await screen.findByRole("link", { name: "Se connecter avec Spotify" });
    expect(login).toHaveAttribute("href", "/auth/login");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("explains a failed login", async () => {
    stubFetch([loggedOut]);
    render(<App authError="accessDenied" />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Tu as refusé l'accès à Spotify");
  });

  it("removes the login error from the address bar", () => {
    window.history.replaceState(null, "", "/?authError=invalidState#top");
    expect(takeAuthError()).toBe("invalidState");
    expect(`${window.location.pathname}${window.location.search}${window.location.hash}`).toBe(
      "/#top",
    );
    expect(takeAuthError()).toBeNull();
  });
});

describe("scan", () => {
  it("shows the progress, then the review", async () => {
    stubFetch([loggedIn]);
    render(<App authError={null} />);
    await screen.findByRole("heading", { name: "Analyse de ta bibliothèque" });
    const source = FakeEventSource.latest();
    expect(source.url).toBe("/api/v1/scan");

    act(() => source.emit("progress", { fetched: 50, total: 200 }));
    const bar = screen.getByRole("progressbar", { name: "Titres likés récupérés" });
    expect(bar).toHaveAttribute("aria-valuenow", "50");
    expect(bar).toHaveAttribute("aria-valuemax", "200");
    expect(screen.getByText("50 / 200")).toBeInTheDocument();

    const tracks = [track({ releaseDate: "1990" }), track({ releaseDate: "2010" })];
    act(() => source.emit("result", scanResult([group({ tracks })], 200)));
    expect(source.closed).toBe(true);
    expect(
      await screen.findByRole("heading", { level: 1, name: "1 titre en double dans tes likes" }),
    ).toBeInTheDocument();
  });

  it("offers a retry after an error event", async () => {
    stubFetch([loggedIn]);
    render(<App authError={null} />);
    await screen.findByRole("heading", { name: "Analyse de ta bibliothèque" });
    const failed = FakeEventSource.latest();
    act(() => failed.emit("error", problem(503, "spotifyRateLimited", "retry in 3600 s")));
    expect(failed.closed).toBe(true);
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("Spotify limite les requêtes");
    expect(alert).toHaveTextContent("retry in 3600 s");

    await userEvent.click(screen.getByRole("button", { name: "Réessayer" }));
    expect(FakeEventSource.instances).toHaveLength(2);
    expect(FakeEventSource.latest().closed).toBe(false);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("returns to the login screen when the stream is refused for want of a session", async () => {
    stubFetch([loggedIn, loggedOut]);
    render(<App authError={null} />);
    await screen.findByRole("heading", { name: "Analyse de ta bibliothèque" });
    act(() => FakeEventSource.latest().drop());
    expect(await screen.findByRole("alert")).toHaveTextContent("Ta session a expiré");
    expect(screen.getByRole("link", { name: "Se connecter avec Spotify" })).toBeInTheDocument();
  });

  it("reports a dropped stream while the session is still valid", async () => {
    stubFetch([loggedIn, loggedIn]);
    render(<App authError={null} />);
    await screen.findByRole("heading", { name: "Analyse de ta bibliothèque" });
    act(() => FakeEventSource.latest().drop());
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "La connexion au serveur a été coupée",
    );
  });
});

describe("logout", () => {
  it("deletes the session and returns to the login screen", async () => {
    const calls = stubFetch([loggedIn, { method: "DELETE", url: "/api/v1/session", status: 204 }]);
    render(<App authError={null} />);
    const header = await screen.findByRole("banner");
    await userEvent.click(within(header).getByRole("button", { name: "Se déconnecter" }));
    expect(
      await screen.findByRole("link", { name: "Se connecter avec Spotify" }),
    ).toBeInTheDocument();
    expect(calls.map((call) => call.method)).toEqual(["GET", "DELETE"]);
    expect(FakeEventSource.latest().closed).toBe(true);
  });
});
