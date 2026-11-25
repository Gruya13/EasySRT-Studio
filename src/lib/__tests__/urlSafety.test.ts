// @ts-ignore Node built-in test runner without @types/node
import { describe, it } from "node:test";
// @ts-ignore Node built-in strict assert without @types/node
import assert from "node:assert/strict";

import { toSafeExternalHttpsUrl } from "../urlSafety.ts";

describe("toSafeExternalHttpsUrl", () => {
  it("accepts and normalizes valid external https:// URLs", () => {
    assert.equal(
      toSafeExternalHttpsUrl("https://github.com/Gruya13/EasySRT-Studio/issues/42"),
      "https://github.com/Gruya13/EasySRT-Studio/issues/42",
    );
    assert.equal(
      toSafeExternalHttpsUrl("  HTTPS://GitHub.com/Gruya13/EasySRT-Studio/releases?tag=v0.4.0#notes  "),
      "https://github.com/Gruya13/EasySRT-Studio/releases?tag=v0.4.0#notes",
    );
    assert.equal(
      toSafeExternalHttpsUrl("https://example.com"),
      "https://example.com/",
    );
    assert.equal(
      toSafeExternalHttpsUrl("https://example.com:8443/path/@handle/info"),
      "https://example.com:8443/path/@handle/info",
    );
  });

  it("rejects non-HTTPS schemes including javascript:, data:, file:, http:, and blob:", () => {
    const blocked = [
      "javascript:alert(1)",
      "JAVASCRIPT:alert(document.domain)",
      "  javascript:void(0)  ",
      "data:text/html,<script>alert(1)</script>",
      "file:///etc/passwd",
      "file:///C:/Windows/System32/cmd.exe",
      "http://github.com/Gruya13/EasySRT-Studio/releases",
      "http://127.0.0.1:8080/admin",
      "vbscript:msgbox(1)",
      "blob:https://example.com/1234",
      "mailto:user@example.com",
      "ftp://files.example.com/pub",
      "asset://localhost/file.mp3",
      "tauri://localhost/index.html",
    ];

    for (const candidate of blocked) {
      assert.equal(
        toSafeExternalHttpsUrl(candidate),
        null,
        `Expected unsafe URL to be rejected: ${candidate}`,
      );
    }
  });

  it("rejects URLs containing embedded userinfo / credentials", () => {
    const credentialed = [
      "https://user:pass@github.com/Gruya13/EasySRT-Studio",
      "https://token@github.com/Gruya13/EasySRT-Studio",
      "https://:secret@example.com/release",
      "https://@example.com/release",
      "https://:@example.com/release",
    ];

    for (const candidate of credentialed) {
      assert.equal(
        toSafeExternalHttpsUrl(candidate),
        null,
        `Expected credentialed URL to be rejected: ${candidate}`,
      );
    }
  });

  it("rejects nullish, empty, relative, control-character, and malformed strings", () => {
    const malformed: Array<string | null | undefined> = [
      null,
      undefined,
      "",
      "   ",
      "not a url",
      "/relative/path",
      "//github.com/protocol-relative",
      "https:///missing-host",
      "https://\\evil.com",
      "https://github.com/path\nwith-newline",
      "https://github.com/path\twith-tab",
      "https:github.com/no-slashes",
    ];

    for (const candidate of malformed) {
      assert.equal(
        toSafeExternalHttpsUrl(candidate),
        null,
        `Expected malformed input to be rejected: ${String(candidate)}`,
      );
    }
  });
});
