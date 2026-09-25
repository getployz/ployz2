import "@tanstack/react-start/server-only";
import { Data, Effect, Schema } from "effect";

/**
 * The Hosted DNS HTTP client: every call Cloud makes to Hosted DNS lives in this file.
 * `POST /domains` mints a name and a token shown once, authorized by the optional mint key;
 * every other call uses the per-name bearer token.
 */

export class HostedDnsError extends Data.TaggedError("HostedDnsError")<{
  readonly operation: string;
  readonly status?: number;
  readonly cause?: unknown;
}> {}

const REQUEST_TIMEOUT_MS = 10_000;

const request = (operation: string, url: string, init: { token: string | undefined; body?: unknown }) =>
  Effect.tryPromise({
    try: async (signal) => {
      const headers = new Headers({ "content-type": "application/json" });
      if (init.token !== undefined) headers.set("authorization", `Bearer ${init.token}`);
      const response = await fetch(url, {
        method: "POST",
        signal: AbortSignal.any([signal, AbortSignal.timeout(REQUEST_TIMEOUT_MS)]),
        headers,
        body: JSON.stringify(init.body ?? {}),
      });
      if (!response.ok) throw new HostedDnsError({ operation, status: response.status });
      const text = await response.text();
      const body: unknown = text === "" ? null : JSON.parse(text);
      return body;
    },
    catch: (cause) => cause instanceof HostedDnsError ? cause : new HostedDnsError({ operation, cause }),
  });

const domainsUrl = (endpoint: string, ...path: string[]) =>
  [endpoint.replace(/\/+$/u, ""), "domains", ...path.map(encodeURIComponent)].join("/");

const Reservation = Schema.Struct({ name: Schema.NonEmptyString, token: Schema.NonEmptyString });

/** Mints a name; Hosted DNS suffixes or replaces `preferred` as it sees fit and returns what it granted. */
export const reserveHostedDomain = Effect.fn("HostedDns.reserve")(function* (input: {
  readonly endpoint: string;
  readonly preferred: string;
  readonly mintKey: string | undefined;
}) {
  const body = yield* request("reserve", domainsUrl(input.endpoint), { token: input.mintKey, body: { preferred: input.preferred } });
  return yield* Schema.decodeUnknownEffect(Reservation)(body).pipe(
    Effect.mapError((cause) => new HostedDnsError({ operation: "reserve", cause })),
  );
});

/** Retires the name forever. */
export const releaseHostedDomain = Effect.fn("HostedDns.release")(function* (input: {
  readonly endpoint: string;
  readonly name: string;
  readonly token: string;
}) {
  yield* request("release", domainsUrl(input.endpoint, input.name, "release"), { token: input.token });
});
