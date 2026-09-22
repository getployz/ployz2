import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { BuildLogs, formatDuration, splitStepName, type BuildOutputRow, type BuildStepRow } from "./deployment-logs";

const deploymentId = "8f79e99b-cd08-4e9c-af96-f3fed313acc5";
const at = (seconds: number) => new Date(Date.UTC(2026, 8, 22, 21, 9, seconds));
const step = (id: number, name: string, extra: Partial<BuildStepRow> = {}): BuildStepRow => ({
  id, deploymentId, key: `sha256:${id}`, name, startedAt: at(id), completedAt: null, cached: false, error: null, createdAt: at(id), updatedAt: at(id), ...extra,
});
const line = (id: number, stepId: number, text: string, stderr = false): BuildOutputRow => ({ id, deploymentId, stepId, stderr, text, createdAt: at(id) });

it("renders each step as a row and keeps output collapsed unless the step is running or failed", () => {
  const steps = [
    step(1, "[internal] load .dockerignore", { completedAt: at(1), cached: true }),
    step(2, "[sdk 4/6] COPY core/ core/", { completedAt: at(3) }),
    step(3, "[sdk 5/6] RUN bash core/scripts/build-cloud-sdk.sh"),
    step(4, "[node 1/3] RUN npm ci", { completedAt: at(5), error: "process \"/bin/sh -c npm ci\" did not complete successfully: exit code: 1" }),
  ];
  const output = [line(1, 2, "<script>alert(1)</script>\n"), line(2, 3, "Compiling ployz-core\n"), line(3, 4, "npm ERR! missing script\n", true)];
  const html = renderToStaticMarkup(createElement(BuildLogs, { hasBuild: true, steps, output, now: at(9).getTime() }));
  expect(html.match(/<li>/g)).toHaveLength(4);
  expect(html).toContain("cached");
  expect(html).toContain("&lt;script&gt;");
  expect(html).not.toContain("<script>");
  expect(html.match(/<details>/g)).toHaveLength(1);
  expect(html.match(/<details open="">/g)).toHaveLength(2);
  expect(html).toContain("Compiling ployz-core");
  expect(html).toContain("exit code: 1");
  expect(html).toContain("21:09:03");
  expect(html).toContain(">2.0s<");
  expect(html).toContain(">6.0s<");
});

it("distinguishes absent Git output from the image-only path", () => {
  const render = (hasBuild: boolean) => renderToStaticMarkup(createElement(BuildLogs, { steps: [], output: [], hasBuild }));
  expect(render(true)).toContain("No retained build output");
  expect(render(false)).toContain("uses prebuilt images");
});

it("splits BuildKit step names into stage and instruction", () => {
  expect(splitStepName("[sdk 4/6] COPY core/ core/")).toEqual({ stage: "sdk", title: "COPY core/ core/" });
  expect(splitStepName("[internal] load build definition from Dockerfile")).toEqual({ stage: "internal", title: "load build definition from Dockerfile" });
  expect(splitStepName("[2/3] RUN echo hi")).toEqual({ stage: null, title: "RUN echo hi" });
  expect(splitStepName("Uploading source")).toEqual({ stage: null, title: "Uploading source" });
  expect([97, 1_234, 142_500].map(formatDuration)).toEqual(["97ms", "1.2s", "2m 22s"]);
});
