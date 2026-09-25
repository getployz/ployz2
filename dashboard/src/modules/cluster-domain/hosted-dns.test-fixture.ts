import { createServer } from "node:http";
import type { AddressInfo } from "node:net";

export type HostedDnsRequest = { path: string; authorization: string | null; body: unknown };

/** An in-process Hosted DNS: grants `<preferred>.ployz.test`, answers releases, or fails every call with `failWith`. */
export async function startFakeHostedDns() {
  const requests: HostedDnsRequest[] = [];
  const state = { failWith: null as number | null };
  const server = createServer((request, response) => {
    let text = "";
    request.on("data", (chunk: Buffer) => { text += chunk.toString("utf8"); });
    request.on("end", () => {
      const body = text === "" ? null : JSON.parse(text) as { preferred?: string };
      requests.push({ path: request.url ?? "", authorization: request.headers.authorization ?? null, body });
      if (state.failWith !== null) {
        response.writeHead(state.failWith).end();
      } else if (request.url === "/domains") {
        response.writeHead(201, { "content-type": "application/json" })
          .end(JSON.stringify({ name: `${body?.preferred}.ployz.test`, token: `token-${requests.length}` }));
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
