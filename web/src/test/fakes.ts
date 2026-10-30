import { vi } from "vitest";

/** An `EventSource` the test drives by hand. */
export class FakeEventSource extends EventTarget {
  static instances: FakeEventSource[] = [];

  readonly url: string;
  closed = false;

  constructor(url: string) {
    super();
    this.url = url;
    FakeEventSource.instances.push(this);
  }

  close(): void {
    this.closed = true;
  }

  emit(event: string, data: unknown): void {
    this.dispatchEvent(new MessageEvent(event, { data: JSON.stringify(data) }));
  }

  /** What the browser dispatches when the connection fails or the server answers 401. */
  drop(): void {
    this.dispatchEvent(new Event("error"));
  }

  static latest(): FakeEventSource {
    const source = FakeEventSource.instances.at(-1);
    if (source === undefined) {
      throw new Error("no EventSource was opened");
    }
    return source;
  }
}

export interface FakeRoute {
  method: string;
  url: string;
  status: number;
  body?: unknown;
}

export interface FetchCall {
  method: string;
  url: string;
  body: unknown;
}

/**
 * Stubs `fetch` with fixed answers, each used once in order for its method
 * and URL, and records the calls.
 */
export function stubFetch(routes: FakeRoute[]): FetchCall[] {
  const pending = [...routes];
  const calls: FetchCall[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const method = init?.method ?? "GET";
      const url = String(input);
      const body = typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
      calls.push({ method, url, body });
      const index = pending.findIndex((route) => route.method === method && route.url === url);
      const route = pending.splice(index, 1)[0];
      if (index === -1 || route === undefined) {
        throw new TypeError(`no answer for ${method} ${url}`);
      }
      const problem = route.status >= 400;
      return new Response(
        route.body === undefined || route.status === 204 ? null : JSON.stringify(route.body),
        {
          status: route.status,
          headers: { "Content-Type": problem ? "application/problem+json" : "application/json" },
        },
      );
    }),
  );
  return calls;
}

export function problem(status: number, code: string, detail = code) {
  return { type: `urn:spoticlean:problem:${code}`, title: code, status, detail };
}
