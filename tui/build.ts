import solidPlugin from "@opentui/solid/bun-plugin";
import { copyFile, mkdir } from "node:fs/promises";

const result = await Bun.build({
  entrypoints: ["./src/index.tsx"],
  target: "bun",
  outdir: "./dist",
  plugins: [solidPlugin],
  // OpenTUI 0.5 loads the native library with a static import per OS/arch.
  // Only the host package is installed. Leave every platform package external
  // so the bundle resolves it from node_modules at runtime.
  external: [
    "@opentui/core-darwin-x64",
    "@opentui/core-darwin-arm64",
    "@opentui/core-linux-x64",
    "@opentui/core-linux-x64-musl",
    "@opentui/core-linux-arm64",
    "@opentui/core-linux-arm64-musl",
    "@opentui/core-win32-x64",
    "@opentui/core-win32-arm64",
  ],
});

if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}

await mkdir("./dist", { recursive: true });
await copyFile("./node_modules/@jsquash/webp/codec/dec/webp_dec.wasm", "./dist/webp_dec.wasm");
