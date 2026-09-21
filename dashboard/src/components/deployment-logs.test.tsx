import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import type { DeploymentProgress } from "#/modules/deployments/deployment-progress";
import { BuildLogs } from "./deployment-logs";

it("renders retained output safely, filters services and keeps truncation visible", () => {
  const event = (id: number, serviceId: string, output: string, outputTruncated = false) => ({ id, progress: {
    completed: 0, total: 0, rows: [], compensation: [], outcome: null,
    preparation: { phase: "build", serviceId, machineId: "builder", machineName: null, message: null, output, outputTruncated },
  } satisfies DeploymentProgress });
  const html = renderToStaticMarkup(createElement(BuildLogs, { hasBuild: true, serviceId: "web", events: [event(1, "web", "<script>alert(1)</script>\nBuild failed", true), event(2, "other", "Other Service output")] }));
  expect(html).toContain("&lt;script&gt;");
  expect(html).toContain("Build failed");
  expect(html).toContain("Build output truncated");
  expect(html).not.toContain("Other Service output");
  expect(html).not.toContain("<script>");
});
it("distinguishes absent Git output from the image-only path", () => {
  const render = (hasBuild: boolean) => renderToStaticMarkup(createElement(BuildLogs, { events: [], hasBuild }));
  expect(render(true)).toContain("No retained build output");
  expect(render(false)).toContain("uses prebuilt images");
});
