import { describe, expect, it, vi } from "vitest";
import { FakeEventSource, problem, stubFetch } from "../test/fakes";
import { getSession, ProblemError, problemCode, removeTracks, startScan } from "./client";

const handlers = () => ({
  onProgress: vi.fn(),
  onResult: vi.fn(),
  onProblem: vi.fn(),
  onDisconnect: vi.fn(),
});

const withCode = (code: string) => (error: unknown) =>
  error instanceof ProblemError && problemCode(error.problem) === code;

describe("api client", () => {
  it("turns a failure into its problem", async () => {
    stubFetch([
      {
        method: "GET",
        url: "/api/v1/session",
        status: 502,
        body: problem(502, "spotifyUnavailable"),
      },
    ]);
    await expect(getSession()).rejects.toSatisfy(withCode("spotifyUnavailable"));
  });

  it("rejects a body that does not match the API", async () => {
    stubFetch([{ method: "POST", url: "/api/v1/removals", status: 200, body: { removed: "all" } }]);
    await expect(removeTracks(["x"])).rejects.toBeInstanceOf(ProblemError);
  });

  it("reports an unreachable server as a problem", async () => {
    stubFetch([]);
    await expect(getSession()).rejects.toSatisfy(withCode("unreachable"));
  });

  it("stops the scan on an event it cannot read", () => {
    const scan = handlers();
    startScan(scan);
    const source = FakeEventSource.latest();
    source.emit("progress", { fetched: "many" });
    expect(source.closed).toBe(true);
    expect(scan.onProblem).toHaveBeenCalledOnce();
    source.drop();
    expect(scan.onDisconnect).not.toHaveBeenCalled();
  });

  it("closes the stream once stopped", () => {
    const stop = startScan(handlers());
    stop();
    expect(FakeEventSource.latest().closed).toBe(true);
  });
});
