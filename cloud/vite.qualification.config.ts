import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const outDir = process.env["PLOYZ_QUALIFICATION_DRIVER_OUT"];
if (!outDir) throw new Error("PLOYZ_QUALIFICATION_DRIVER_OUT is required");

/** Opt-in plain-Node build for the live qualification driver. */
export default defineConfig({
  resolve: {
    alias: {
      "#": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  build: {
    ssr: "src/qualification/tailcat-883.live.ts",
    outDir,
    rollupOptions: {
      external: ["@ployz/sdk"],
    },
  },
});
