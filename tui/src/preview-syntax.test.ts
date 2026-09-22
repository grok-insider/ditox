import { describe, expect, test } from "bun:test";
import { classifyPreview, codeFiletype } from "./preview-syntax";

describe("preview syntax", () => {
  test("leaves ordinary clipboard text plain", () => {
    expect(classifyPreview("https://x.com/status/1")).toBe("plain");
    expect(classifyPreview("just a sentence")).toBe("plain");
  });

  test("treats markdown structure as markdown and source as code", () => {
    expect(classifyPreview("# Title\n\n- item")).toBe("markdown");
    expect(classifyPreview("```ts\nconst n = 1\n```")).toBe("markdown");
    expect(classifyPreview("function greet() {\n  return 1\n}")).toBe("code");
    expect(codeFiletype("function greet() {\n  return 1\n}")).toBe("javascript");
    expect(codeFiletype("```rs\nfn main() {}\n```")).toBe("rs");
  });
});
