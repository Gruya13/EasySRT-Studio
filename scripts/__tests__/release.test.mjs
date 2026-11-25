import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";
import {
  buildChangelogSection,
  bumpSemver,
  checkVersions,
  extractReleaseNotes,
  parseConventionalCommit,
  parseSemver,
  syncVersions,
  upsertChangelog,
} from "../release.mjs";

describe("semver utilities", () => {
  it("parses standard and prerelease versions", () => {
    assert.deepEqual(parseSemver("v0.3.2"), {
      major: 0,
      minor: 3,
      patch: 2,
      prerelease: null,
      raw: "0.3.2",
    });
    assert.deepEqual(parseSemver("1.2.3-rc.1"), {
      major: 1,
      minor: 2,
      patch: 3,
      prerelease: "rc.1",
      raw: "1.2.3-rc.1",
    });
  });

  it("bumps patch, minor, major, and accepts explicit targets", () => {
    assert.equal(bumpSemver("0.3.2", "patch"), "0.3.3");
    assert.equal(bumpSemver("0.3.2", "minor"), "0.4.0");
    assert.equal(bumpSemver("0.3.2", "major"), "1.0.0");
    assert.equal(bumpSemver("0.3.2", "v0.3.5"), "0.3.5");
    assert.equal(bumpSemver("0.3.2", "0.3.3-rc.1"), "0.3.3-rc.1");
    assert.throws(() => bumpSemver("0.3.2", "not-a-version"));
  });
});

describe("conventional commit parsing and changelog generation", () => {
  it("classifies commits and skips merge/release commits", () => {
    assert.equal(
      parseConventionalCommit({ hash: "aaaa111", subject: "Merge pull request #12 from staging" }),
      null,
    );
    assert.equal(
      parseConventionalCommit({ hash: "bbbb222", subject: "chore(release): prepare v0.3.3" }),
      null,
    );
    assert.equal(
      parseConventionalCommit({ hash: "cccc333", subject: "docs(planning): add v0.5.0 sprint plan" }),
      null,
    );
    assert.equal(
      parseConventionalCommit({ hash: "dddd444", subject: "docs(dev): update architecture diagram" }),
      null,
    );
    assert.equal(
      parseConventionalCommit({ hash: "eeee555", subject: "test(segmenter): add comma split unit tests" }),
      null,
    );
    assert.equal(
      parseConventionalCommit({ hash: "ffff666", subject: "ci(actions): pin checkout sha" }),
      null,
    );

    const sec = parseConventionalCommit({
      hash: "0d579fa",
      subject: "security(backend): harden storage vault",
    });
    assert.equal(sec?.section, "security");

    const feat = parseConventionalCommit({
      hash: "1234567890",
      subject: "feat(segmenter): split long sentences after comma",
    });
    assert.equal(feat?.section, "added");
    assert.equal(feat?.scope, "segmenter");
    assert.equal(feat?.shortHash, "1234567");

    const breaking = parseConventionalCommit({
      hash: "9999999",
      subject: "refactor(api)!: change checkpoint format",
    });
    assert.equal(breaking?.section, "breaking");

    const bodyBreaking = parseConventionalCommit({
      hash: "8888888",
      subject: "fix: adjust worker protocol",
      body: "Details\n\nBREAKING CHANGE: payload field renamed",
    });
    assert.equal(bodyBreaking?.section, "breaking");

    const plain = parseConventionalCommit({
      hash: "7777777",
      subject: "Improve Serbian translation defaults",
    });
    assert.equal(plain?.section, "other");
  });

  it("builds grouped Keep-a-Changelog section and handles empty commit lists", () => {
    const md = buildChangelogSection({
      version: "0.3.3",
      date: "2026-11-25",
      commits: [
        { hash: "1111111", subject: "fix(vad): avoid dropping first word" },
        { hash: "2222222", subject: "feat(diarizer): detect speaker turns" },
        { hash: "3333333", subject: "perf: speed up waveform decode" },
      ],
    });

    assert.match(md, /^## \[0\.3\.3\] - 2026-11-25/m);
    assert.ok(md.indexOf("### Added") < md.indexOf("### Fixed"));
    assert.ok(md.indexOf("### Fixed") < md.indexOf("### Performance"));
    assert.match(md, /- \*\*diarizer:\*\* detect speaker turns \(`2222222`\)/);

    const emptyMd = buildChangelogSection({
      version: "0.3.3",
      date: "2026-11-25",
      commits: [{ hash: "4444444", subject: "Merge branch 'main' into staging" }],
    });
    assert.match(emptyMd, /Maintenance and release updates/);
  });

  it("upserts changelog sections idempotently and extracts release notes", () => {
    const initial = [
      "# Changelog",
      "",
      "## [Unreleased]",
      "",
      "## [0.3.2] - 2026-11-24",
      "",
      "### Added",
      "- Initial public release.",
      "",
      "[Unreleased]: https://github.com/Gruya13/EasySRT-Studio/compare/v0.3.2...HEAD",
      "[0.3.2]: https://github.com/Gruya13/EasySRT-Studio/releases/tag/v0.3.2",
      "",
    ].join("\n");

    const sec033 = [
      "## [0.3.3] - 2026-11-26",
      "",
      "### Added",
      "- Comma-first splitting (`abcdef1`)",
    ].join("\n");

    const once = upsertChangelog(initial, {
      version: "0.3.3",
      prevTag: "v0.3.2",
      sectionMarkdown: sec033,
    });
    assert.match(once, /\[Unreleased\]: .*\/compare\/v0\.3\.3\.\.\.HEAD/);
    assert.match(once, /\[0\.3\.3\]: .*\/compare\/v0\.3\.2\.\.\.v0\.3\.3/);
    assert.match(once, /## \[0\.3\.2\] - 2026-11-24/);

    // Re-running for the same version replaces cleanly without duplicating
    const updatedSec033 = [
      "## [0.3.3] - 2026-11-26",
      "",
      "### Added",
      "- Comma-first splitting (`abcdef1`)",
      "- Speaker change detection (`fedcba9`)",
    ].join("\n");
    const twice = upsertChangelog(once, {
      version: "0.3.3",
      prevTag: "v0.3.2",
      sectionMarkdown: updatedSec033,
    });
    assert.equal((twice.match(/^## \[0\.3\.3\]/gm) ?? []).length, 1);
    assert.match(twice, /Speaker change detection/);
    assert.match(twice, /Initial public release/);

    const notes033 = extractReleaseNotes(twice, "v0.3.3");
    assert.equal(
      notes033,
      "### Added\n- Comma-first splitting (`abcdef1`)\n- Speaker change detection (`fedcba9`)",
    );

    const notes032 = extractReleaseNotes(twice, "0.3.2");
    assert.equal(notes032, "### Added\n- Initial public release.");
    assert.equal(extractReleaseNotes(twice, "9.9.9"), null);
  });
});

describe("manifest version check & sync", () => {
  it("syncs all manifests without modifying dependency versions", () => {
    const tmp = mkdtempSync(path.join(os.tmpdir(), "easysrt-release-test-"));
    try {
      mkdirSync(path.join(tmp, "src-tauri"), { recursive: true });
      mkdirSync(path.join(tmp, "worker"), { recursive: true });

      writeFileSync(
        path.join(tmp, "package.json"),
        JSON.stringify({ name: "easysrt-studio", version: "0.3.2" }, null, 2),
      );
      writeFileSync(
        path.join(tmp, "package-lock.json"),
        JSON.stringify(
          {
            name: "easysrt-studio",
            version: "0.3.2",
            packages: {
              "": { name: "easysrt-studio", version: "0.3.2" },
              "node_modules/vite": { version: "8.0.0" },
            },
          },
          null,
          2,
        ),
      );
      writeFileSync(
        path.join(tmp, "src-tauri", "tauri.conf.json"),
        JSON.stringify({ productName: "EasySRT Studio", version: "0.3.2" }, null, 2),
      );
      writeFileSync(
        path.join(tmp, "src-tauri", "Cargo.toml"),
        [
          "[package]",
          'name = "easysrt-studio"',
          'version = "0.3.2"',
          'edition = "2021"',
          "",
          "[dependencies]",
          'tauri = { version = "2" }',
          "",
        ].join("\n"),
      );
      writeFileSync(
        path.join(tmp, "src-tauri", "Cargo.lock"),
        [
          "[[package]]",
          'name = "anyhow"',
          'version = "1.0.95"',
          "",
          "[[package]]",
          'name = "easysrt-studio"',
          'version = "0.3.2"',
          "",
        ].join("\n"),
      );
      writeFileSync(
        path.join(tmp, "worker", "package.json"),
        JSON.stringify({ name: "easysrt-telemetry", version: "0.3.2" }, null, 2),
      );
      writeFileSync(
        path.join(tmp, "worker", "package-lock.json"),
        JSON.stringify(
          {
            name: "easysrt-telemetry",
            version: "0.3.2",
            packages: { "": { name: "easysrt-telemetry", version: "0.3.2" } },
          },
          null,
          2,
        ),
      );
      writeFileSync(
        path.join(tmp, "README.md"),
        [
          "[![Release v0.3.2](https://img.shields.io/badge/release-v0.3.2-38bdf8?style=for-the-badge)](https://example.com)",
          "### [Download EasySRT Studio v0.3.2](https://example.com)",
          "",
        ].join("\n"),
      );
      writeFileSync(
        path.join(tmp, "README.sr.md"),
        [
          "[![Izdanje v0.3.2](https://img.shields.io/badge/izdanje-v0.3.2-38bdf8?style=for-the-badge)](https://example.com)",
          "### [Preuzmi EasySRT Studio v0.3.2](https://example.com)",
          "",
        ].join("\n"),
      );

      assert.equal(checkVersions(tmp).ok, true);

      syncVersions(tmp, "0.3.3-rc.1");
      assert.equal(checkVersions(tmp).ok, true);
      assert.equal(checkVersions(tmp).expectedVersion, "0.3.3-rc.1");

      // Dependency versions must remain untouched
      const cargoToml = readFileSync(path.join(tmp, "src-tauri", "Cargo.toml"), "utf8");
      assert.match(cargoToml, /tauri = \{ version = "2" \}/);
      const cargoLock = readFileSync(path.join(tmp, "src-tauri", "Cargo.lock"), "utf8");
      assert.match(cargoLock, /name = "anyhow"\nversion = "1\.0\.95"/);

      // Sync again to stable 0.3.3 to ensure prerelease README badge round-trips cleanly
      syncVersions(tmp, "0.3.3");
      const checkStable = checkVersions(tmp);
      assert.equal(checkStable.ok, true);
      assert.equal(checkStable.expectedVersion, "0.3.3");
      const readme = readFileSync(path.join(tmp, "README.md"), "utf8");
      assert.match(readme, /release-v0\.3\.3-38bdf8/);
      assert.match(readme, /Download EasySRT Studio v0\.3\.3/);
      const readmeSr = readFileSync(path.join(tmp, "README.sr.md"), "utf8");
      assert.match(readmeSr, /izdanje-v0\.3\.3-38bdf8/);
      assert.match(readmeSr, /Preuzmi EasySRT Studio v0\.3\.3/);

      // Introduce drift and verify checkVersions catches it
      writeFileSync(
        path.join(tmp, "worker", "package.json"),
        JSON.stringify({ name: "easysrt-telemetry", version: "0.3.2" }, null, 2),
      );
      const drift = checkVersions(tmp);
      assert.equal(drift.ok, false);
      assert.equal(
        drift.entries.find((e) => e.file === "worker/package.json")?.ok,
        false,
      );
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  });
});
