import { createServer } from "node:http";
import type { AddressInfo } from "node:net";

export type HostedDnsRequest = { method: string; path: string; authorization: string | null; body: unknown };

/**
 * An in-process Hosted DNS: grants `<preferred>.ployz.test`, answers a lease renewal with 200 and every other
 * call with 204. `failWith` fails every call; `gone` answers calls for a name with a status (404 reaped,
 * 410 retired) until the name is granted again.
 */
export async function startFakeHostedDns() {
  const requests: HostedDnsRequest[] = [];
  const state = { failWith: null as number | null, gone: new Map<string, number>() };
  const server = createServer((request, response) => {
    let text = "";
    request.on("data", (chunk: Buffer) => { text += chunk.toString("utf8"); });
    request.on("end", () => {
      const body = text === "" ? null : JSON.parse(text) as { preferred?: string };
      const path = request.url ?? "";
      requests.push({ method: request.method ?? "", path, authorization: request.headers.authorization ?? null, body });
      const goneStatus = state.gone.get(decodeURIComponent(path.split("/")[2] ?? ""));
      if (state.failWith !== null) {
        response.writeHead(state.failWith).end();
      } else if (request.method === "POST" && path === "/domains") {
        const name = `${body?.preferred}.ployz.test`;
        state.gone.delete(name);
        response.writeHead(201, { "content-type": "application/json" })
          .end(JSON.stringify({ name, token: `token-${requests.length}` }));
      } else if (goneStatus !== undefined) {
        response.writeHead(goneStatus).end();
      } else if (path.endsWith("/lease")) {
        response.writeHead(200, { "content-type": "application/json" })
          .end(JSON.stringify({ name: path.split("/")[2], lease_expires_at: new Date(Date.now() + 7 * 86_400_000).toISOString() }));
      } else {
        response.writeHead(204).end();
      }
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  return {
    url: `http://127.0.0.1:${port}/`,
    requests,
    state,
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
}

export type FakeHostedDns = Awaited<ReturnType<typeof startFakeHostedDns>>;
