import type { z } from "zod";
import { logger } from "../logger";
import {
  zProblemResponse,
  zRemovalResponse,
  zScanProgressResponse,
  zScanResponse,
  zSessionResponse,
} from "./generated/zod.gen";

export type Problem = z.infer<typeof zProblemResponse>;
export type Session = z.infer<typeof zSessionResponse>;
export type ScanProgress = z.infer<typeof zScanProgressResponse>;
export type ScanResult = z.infer<typeof zScanResponse>;
export type Group = ScanResult["groups"][number];
export type Track = Group["tracks"][number];
export type Confidence = Group["confidence"];
export type RemovalResponse = z.infer<typeof zRemovalResponse>;
export type FailedRemoval = RemovalResponse["failed"][number];
export type RemovalFailureReason = FailedRemoval["reason"];

export const LOGIN_URL = "/auth/login";
export const SCAN_URL = "/api/v1/scan";
const SESSION_URL = "/api/v1/session";
const REMOVALS_URL = "/api/v1/removals";

const PROBLEM_PREFIX = "urn:spoticlean:problem:";

/** The session is missing or expired: the user has to log in again. */
export class UnauthenticatedError extends Error {
  constructor() {
    super("unauthenticated");
    this.name = "UnauthenticatedError";
  }
}

/** A failed call, as the problem the server answered or one standing for it. */
export class ProblemError extends Error {
  readonly problem: Problem;

  constructor(problem: Problem) {
    super(problem.detail);
    this.name = "ProblemError";
    this.problem = problem;
  }
}

export function problemCode(problem: Problem): string {
  return problem.type.startsWith(PROBLEM_PREFIX)
    ? problem.type.slice(PROBLEM_PREFIX.length)
    : problem.type;
}

export function localProblem(code: string, detail: string, status = 0): Problem {
  return { type: `${PROBLEM_PREFIX}${code}`, title: code, status, detail };
}

async function send(url: string, init: RequestInit = {}): Promise<Response> {
  const method = init.method ?? "GET";
  let response: Response;
  try {
    response = await fetch(url, { credentials: "same-origin", ...init });
  } catch (error) {
    logger.warn("api.unreachable", { method, url, error: String(error) });
    throw new ProblemError(localProblem("unreachable", String(error)));
  }
  logger.debug("api.response", { method, url, status: response.status });
  if (response.status === 401) {
    throw new UnauthenticatedError();
  }
  return response;
}

async function json(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch {
    return undefined;
  }
}

async function failure(response: Response): Promise<ProblemError> {
  const parsed = zProblemResponse.safeParse(await json(response));
  return new ProblemError(
    parsed.success
      ? parsed.data
      : localProblem("unexpectedResponse", `HTTP ${response.status}`, response.status),
  );
}

function parse<T>(schema: z.ZodType<T>, value: unknown, url: string): T {
  const parsed = schema.safeParse(value);
  if (!parsed.success) {
    logger.error("api.invalid_body", { url, issues: parsed.error.issues.length });
    throw new ProblemError(localProblem("unexpectedResponse", parsed.error.message));
  }
  return parsed.data;
}

/** The current session, or `null` when the user is not logged in. */
export async function getSession(): Promise<Session | null> {
  try {
    const response = await send(SESSION_URL);
    if (!response.ok) {
      throw await failure(response);
    }
    return parse(zSessionResponse, await json(response), SESSION_URL);
  } catch (error) {
    if (error instanceof UnauthenticatedError) {
      return null;
    }
    throw error;
  }
}

export async function logout(): Promise<void> {
  const response = await send(SESSION_URL, { method: "DELETE" });
  if (!response.ok) {
    throw await failure(response);
  }
}

export type RemovalOutcome =
  | { kind: "complete"; removed: string[] }
  | { kind: "partial"; removed: string[]; failed: FailedRemoval[] };

export async function removeTracks(trackIds: readonly string[]): Promise<RemovalOutcome> {
  const response = await send(REMOVALS_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ trackIds }),
  });
  if (response.status !== 200 && response.status !== 207) {
    throw await failure(response);
  }
  const body = parse(zRemovalResponse, await json(response), REMOVALS_URL);
  return response.status === 200
    ? { kind: "complete", removed: body.removed }
    : { kind: "partial", removed: body.removed, failed: body.failed };
}

export interface ScanHandlers {
  onProgress(progress: ScanProgress): void;
  onResult(result: ScanResult): void;
  onProblem(problem: Problem): void;
  /** The stream closed before its last event, e.g. on a 401 or a dropped connection. */
  onDisconnect(): void;
}

/** Starts a scan; the returned function stops it. */
export function startScan(handlers: ScanHandlers): () => void {
  const source = new EventSource(SCAN_URL);
  let open = true;
  // An EventSource reconnects whenever its stream ends, which would start a
  // new scan: it is closed as soon as the last event arrives.
  const close = () => {
    open = false;
    source.close();
  };
  const payload = (event: MessageEvent<unknown>): unknown => {
    try {
      return typeof event.data === "string" ? JSON.parse(event.data) : undefined;
    } catch {
      return undefined;
    }
  };
  const invalid = (name: string) => {
    logger.error("scan.invalid_event", { event: name });
    close();
    handlers.onProblem(localProblem("unexpectedResponse", `invalid ${name} event`));
  };

  source.addEventListener("progress", (event) => {
    const parsed = zScanProgressResponse.safeParse(payload(event));
    if (parsed.success) {
      handlers.onProgress(parsed.data);
    } else {
      invalid("progress");
    }
  });
  source.addEventListener("result", (event) => {
    const parsed = zScanResponse.safeParse(payload(event));
    if (!parsed.success) {
      invalid("result");
      return;
    }
    close();
    logger.info("scan.done", { scanned: parsed.data.scanned, groups: parsed.data.groups.length });
    handlers.onResult(parsed.data);
  });
  // The server's `error` event and the browser's own connection error share
  // this name; only the server's carries data.
  source.addEventListener("error", (event) => {
    if (!open) {
      return;
    }
    if (event instanceof MessageEvent) {
      const parsed = zProblemResponse.safeParse(payload(event));
      close();
      logger.warn("scan.failed", { type: parsed.success ? parsed.data.type : "invalid" });
      handlers.onProblem(
        parsed.success ? parsed.data : localProblem("unexpectedResponse", "invalid error event"),
      );
      return;
    }
    close();
    logger.warn("scan.disconnected");
    handlers.onDisconnect();
  });
  return close;
}
