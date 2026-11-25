import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";
import {
  checkDocs,
  checkLanguageCoModification,
  countH2Headings,
  extractLocalLinks,
} from "../docs-check.mjs";

function createValidDocsTree(rootDir) {
  mkdirSync(path.join(rootDir, "docs", "user", "en"), { recursive: true });
  mkdirSync(path.join(rootDir, "docs", "user", "sr"), { recursive: true });
  mkdirSync(path.join(rootDir, "docs", "dev"), { recursive: true });

  writeFileSync(path.join(rootDir, "LICENSE"), "Apache 2.0\n");

  writeFileSync(
    path.join(rootDir, "README.md"),
    [
      "**English** | [Srpski](README.sr.md)",
      "",
      "[![License](https://img.shields.io/badge/license-Apache-green)](LICENSE)",
      "",
      "See [User Guide](docs/user/en/guide.md) and [Architecture](docs/dev/ARCHITECTURE.md).",
      "",
    ].join("\n"),
  );

  writeFileSync(
    path.join(rootDir, "README.sr.md"),
    [
      "[English](README.md) | **Srpski**",
      "",
      "[![Licenca](https://img.shields.io/badge/license-Apache-green)](LICENSE)",
      "",
      "Pogledajte [Korisnički vodič](docs/user/sr/guide.md) i [Arhitekturu](docs/dev/ARCHITECTURE.md).",
      "",
    ].join("\n"),
  );

  writeFileSync(
    path.join(rootDir, "CHANGELOG.md"),
    "# Changelog\n\n## [0.4.0] - 2026-11-25\n\n### Added\n- New cinema editor.\n",
  );

  writeFileSync(
    path.join(rootDir, "AGENTS.md"),
    "# Rules\n\nSee [Release Workflow](docs/dev/RELEASE_WORKFLOW.md).\n",
  );

  writeFileSync(
    path.join(rootDir, "docs", "dev", "ARCHITECTURE.md"),
    "# Architecture\n\n## Overview\nSystem details.\n",
  );

  writeFileSync(
    path.join(rootDir, "docs", "dev", "RELEASE_WORKFLOW.md"),
    "# Release Workflow\n\n## Staging to Main\nDetails.\n",
  );

  writeFileSync(
    path.join(rootDir, "docs", "user", "en", "guide.md"),
    [
      "**English** | [Srpski](../sr/guide.md)",
      "",
      "# User Guide",
      "",
      "## Getting Started",
      "Install from [README](../../../README.md).",
      "",
      "## Shortcuts",
      "```markdown",
      "## Ignored heading inside fenced code block",
      "[Ignored Fake Link](nonexistent/code-example.md)",
      "```",
      "Inline example `[fake](missing.md)` is also ignored.",
      "",
    ].join("\n"),
  );

  writeFileSync(
    path.join(rootDir, "docs", "user", "sr", "guide.md"),
    [
      "[English](../en/guide.md) | **Srpski**",
      "",
      "# Korisnički vodič",
      "",
      "## Početak rada",
      "Instalirajte sa [README](../../../README.sr.md).",
      "",
      "## Prečice",
      "Opis prečica.",
      "",
    ].join("\n"),
  );
}

describe("markdown parsing helpers", () => {
  it("counts H2 headings while ignoring fenced code blocks", () => {
    const md = [
      "# Title",
      "## Real Section 1",
      "```bash",
      "## Not a section",
      "```",
      "## Real Section 2",
      "### Subsection",
    ].join("\n");
    assert.equal(countH2Headings(md), 2);
  });

  it("extracts nested badge targets, HTML links, and ignores external/code links", () => {
    const md = [
      "[![License](https://img.shields.io/badge/Apache-green)](LICENSE)",
      "[Guide](docs/user/en/guide.md#shortcuts)",
      '<p><a href="docs/dev/ARCHITECTURE.md">Arch</a></p>',
      "`[InlineCode](should-ignore.md)`",
      "[External](https://github.com/Gruya13/EasySRT-Studio)",
      "[AnchorOnly](#top)",
    ].join("\n");
    const targets = extractLocalLinks(md).map((t) => t.targetPath);
    assert.deepEqual(targets, [
      "LICENSE",
      "docs/user/en/guide.md",
      "docs/dev/ARCHITECTURE.md",
    ]);
  });
});

describe("checkDocs end-to-end rules", () => {
  it("passes on a valid 3-layer repo and enforces --strict-main on docs/planning", () => {
    const tmp = mkdtempSync(path.join(os.tmpdir(), "easysrt-docs-check-"));
    try {
      createValidDocsTree(tmp);

      // Clean tree passes both staging and strict-main
      assert.equal(checkDocs(tmp, { strictMain: false }).ok, true);
      assert.equal(checkDocs(tmp, { strictMain: true }).ok, true);

      // Adding an active sprint plan passes on staging, but fails on --strict-main
      mkdirSync(path.join(tmp, "docs", "planning"), { recursive: true });
      writeFileSync(
        path.join(tmp, "docs", "planning", "V0.5.0_PLAN.md"),
        "# Sprint Plan\n\nSee [Architecture](../dev/ARCHITECTURE.md).\n",
      );

      const stagingRes = checkDocs(tmp, { strictMain: false });
      assert.equal(stagingRes.ok, true);
      assert.equal(stagingRes.planningFileCount, 1);

      const mainRes = checkDocs(tmp, { strictMain: true });
      assert.equal(mainRes.ok, false);
      assert.ok(
        mainRes.errors.some((e) => e.includes("Strict main gate failed")),
      );
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  });

  it("catches broken links, section parity mismatches, and planning leaks", () => {
    const tmp = mkdtempSync(path.join(os.tmpdir(), "easysrt-docs-err-"));
    try {
      createValidDocsTree(tmp);

      // Add extra ## section only in EN guide + a broken link + planning leak in README
      writeFileSync(
        path.join(tmp, "docs", "user", "en", "guide.md"),
        [
          "**English** | [Srpski](../sr/guide.md)",
          "## Section 1",
          "## Section 2",
          "## Extra Unmatched Section",
          "Broken [link](does-not-exist.md).",
        ].join("\n"),
      );
      writeFileSync(
        path.join(tmp, "CHANGELOG.md"),
        "# Changelog\n\nSee docs/planning/V0.5.0_PLAN.md for details.\n",
      );

      const res = checkDocs(tmp);
      assert.equal(res.ok, false);
      assert.ok(res.errors.some((e) => e.includes("Section count mismatch")));
      assert.ok(res.errors.some((e) => e.includes("Broken relative link")));
      assert.ok(
        res.errors.some((e) =>
          e.includes('must not reference internal "docs/planning"'),
        ),
      );
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  });

  it("detects single-language drift in co-modification checks", () => {
    assert.deepEqual(
      checkLanguageCoModification(["README.md", "README.sr.md"]),
      [],
    );
    assert.equal(checkLanguageCoModification(["README.md"]).length, 1);
    assert.equal(
      checkLanguageCoModification(["docs/user/en/guide.md"]).length,
      1,
    );
    assert.deepEqual(
      checkLanguageCoModification([
        "docs/user/en/guide.md",
        "docs/user/sr/guide.md",
        "docs/dev/ARCHITECTURE.md",
      ]),
      [],
    );
  });
});
