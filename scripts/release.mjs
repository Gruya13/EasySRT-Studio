#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const DEFAULT_REPO_URL = "https://github.com/Gruya13/EasySRT-Studio";
const DEFAULT_DOWNLOAD_FOOTER = [
  "---",
  "",
  "### Downloads",
  "- **Windows 10/11 (x64):** Download the `.exe` Setup Installer (automatic FFmpeg / CUDA / Whisper / llama.cpp setup).",
  "- **Linux (x64):** Download the `.AppImage` (and optional `whisper-bin-ubuntu-vulkan-x64.tar.gz` for Vulkan acceleration).",
].join("\n");

const SEMVER_RE =
  /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

const CONVENTIONAL_RE = /^([a-zA-Z]+)(?:\(([^)]+)\))?(!)?:\s*(.+)$/;

const SECTION_ORDER = [
  { key: "breaking", title: "Breaking Changes" },
  { key: "added", title: "Added" },
  { key: "fixed", title: "Fixed" },
  { key: "security", title: "Security" },
  { key: "performance", title: "Performance" },
  { key: "changed", title: "Changed" },
  { key: "docs", title: "Documentation" },
  { key: "maintenance", title: "Maintenance" },
  { key: "other", title: "Other Changes" },
];

const INTERNAL_DOC_SCOPES = new Set(["planning", "dev", "internal", "wip"]);
const README_FILES = ["README.md", "README.sr.md"];

export function parseSemver(input) {
  const match = String(input ?? "").trim().match(SEMVER_RE);
  if (!match) {
    throw new Error(`Invalid semantic version: "${input}"`);
  }
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prerelease: match[4] ?? null,
    raw: `${match[1]}.${match[2]}.${match[3]}${match[4] ? `-${match[4]}` : ""}`,
  };
}

export function bumpSemver(currentVersion, spec) {
  const cur = parseSemver(currentVersion);
  const normalized = String(spec ?? "").trim();

  if (normalized === "patch") {
    return `${cur.major}.${cur.minor}.${cur.patch + 1}`;
  }
  if (normalized === "minor") {
    return `${cur.major}.${cur.minor + 1}.0`;
  }
  if (normalized === "major") {
    return `${cur.major + 1}.0.0`;
  }

  return parseSemver(normalized).raw;
}

export function parseConventionalCommit({ hash = "", subject = "", body = "" }) {
  const cleanSubject = subject.trim();
  if (!cleanSubject) return null;

  // Skip merge commits and automated release bump commits
  if (
    /^Merge (pull request|branch|remote-tracking branch)\b/i.test(cleanSubject) ||
    /^chore\(release\):/i.test(cleanSubject) ||
    /^release:\s*v?\d+\.\d+\.\d+/i.test(cleanSubject)
  ) {
    return null;
  }

  const shortHash = hash ? hash.slice(0, 7) : "";
  const match = cleanSubject.match(CONVENTIONAL_RE);
  const hasBreakingFooter = /^BREAKING[ -]CHANGE:/m.test(body ?? "");

  if (!match) {
    return {
      section: hasBreakingFooter ? "breaking" : "other",
      type: "other",
      scope: null,
      description: cleanSubject,
      shortHash,
      breaking: hasBreakingFooter,
    };
  }

  const type = match[1].toLowerCase();
  const scope = match[2]?.trim() || null;
  const scopeLower = scope ? scope.toLowerCase() : null;
  const breaking = Boolean(match[3]) || hasBreakingFooter;
  const description = match[4].trim();

  // Keep user-facing CHANGELOG.md free of internal planning/dev-doc, test, and CI noise
  if (!breaking) {
    if (type === "test" || type === "ci") {
      return null;
    }
    if (
      (type === "docs" || type === "chore") &&
      scopeLower &&
      INTERNAL_DOC_SCOPES.has(scopeLower)
    ) {
      return null;
    }
  }

  let section = "other";
  if (breaking) {
    section = "breaking";
  } else if (type === "feat") {
    section = "added";
  } else if (type === "security" || (type === "fix" && scopeLower === "security")) {
    section = "security";
  } else if (type === "fix") {
    section = "fixed";
  } else if (type === "perf") {
    section = "performance";
  } else if (type === "refactor") {
    section = "changed";
  } else if (type === "docs") {
    section = "docs";
  } else if (["chore", "build", "style", "revert"].includes(type)) {
    section = "maintenance";
  }

  return {
    section,
    type,
    scope,
    description,
    shortHash,
    breaking,
  };
}

export function buildChangelogSection({ version, date, commits }) {
  const grouped = new Map(SECTION_ORDER.map((s) => [s.key, []]));

  for (const rawCommit of commits) {
    const parsed = parseConventionalCommit(rawCommit);
    if (!parsed) continue;
    grouped.get(parsed.section).push(parsed);
  }

  const lines = [`## [${version}] - ${date}`];
  let hasAnyEntries = false;

  for (const { key, title } of SECTION_ORDER) {
    const items = grouped.get(key);
    if (!items || items.length === 0) continue;
    hasAnyEntries = true;
    lines.push("", `### ${title}`);
    for (const item of items) {
      const scopePrefix = item.scope ? `**${item.scope}:** ` : "";
      const hashSuffix = item.shortHash ? ` (\`${item.shortHash}\`)` : "";
      lines.push(`- ${scopePrefix}${item.description}${hashSuffix}`);
    }
  }

  if (!hasAnyEntries) {
    lines.push("", "### Maintenance", "- Maintenance and release updates.");
  }

  return lines.join("\n");
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function upsertChangelog(
  existingContent,
  { version, prevTag = null, sectionMarkdown, repoUrl = DEFAULT_REPO_URL },
) {
  const header = [
    "# Changelog",
    "",
    "All notable changes to **EasySRT Studio** will be documented in this file.",
    "",
    "The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),",
    "and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).",
    "",
    "## [Unreleased]",
    "",
  ].join("\n");

  let content = existingContent?.trim()
    ? existingContent.replace(/\r\n/g, "\n")
    : header;

  if (!/^## \[Unreleased\]/m.test(content)) {
    const firstH2 = content.search(/^## \[/m);
    if (firstH2 >= 0) {
      content = `${content.slice(0, firstH2)}## [Unreleased]\n\n${content.slice(firstH2)}`;
    } else {
      content = `${content.trimEnd()}\n\n## [Unreleased]\n`;
    }
  }

  // Separate bottom reference links from main body
  const linkBlockRe = /\n+(?:\[[^\]]+\]:\s*https?:\/\/\S+\n*)+\s*$/;
  const linkMatch = content.match(linkBlockRe);
  let body = linkMatch ? content.slice(0, linkMatch.index).trimEnd() : content.trimEnd();
  const existingLinks = new Map();

  if (linkMatch) {
    for (const line of linkMatch[0].trim().split("\n")) {
      const m = line.match(/^\[([^\]]+)\]:\s*(\S+)\s*$/);
      if (m) existingLinks.set(m[1], m[2]);
    }
  }

  // Replace existing section for this exact version if present; otherwise insert after [Unreleased]
  const escapedVersion = escapeRegExp(version);
  const existingHeadingRe = new RegExp(`^## \\[${escapedVersion}\\][^\\n]*$`, "m");
  const existingHeadingMatch = body.match(existingHeadingRe);

  if (existingHeadingMatch) {
    const startIdx = existingHeadingMatch.index;
    const afterStart = body.slice(startIdx + existingHeadingMatch[0].length);
    const nextHeadingOffset = afterStart.search(/\n## \[/);
    const endIdx =
      nextHeadingOffset >= 0
        ? startIdx + existingHeadingMatch[0].length + nextHeadingOffset
        : body.length;
    const before = body.slice(0, startIdx).trimEnd();
    const after = body.slice(endIdx).trimStart();
    const parts = [before, sectionMarkdown.trimEnd(), after].filter(Boolean);
    body = parts.join("\n\n");
  } else {
    const unreleasedMatch = body.match(/^## \[Unreleased\][^\n]*$/m);
    if (unreleasedMatch) {
      const startIdx = unreleasedMatch.index;
      const afterStart = body.slice(startIdx + unreleasedMatch[0].length);
      const nextHeadingOffset = afterStart.search(/\n## \[/);
      const insertPos =
        nextHeadingOffset >= 0
          ? startIdx + unreleasedMatch[0].length + nextHeadingOffset
          : body.length;
      const before = body.slice(0, insertPos).trimEnd();
      const after = body.slice(insertPos).trimStart();
      body = after
        ? `${before}\n\n${sectionMarkdown.trimEnd()}\n\n${after}`
        : `${before}\n\n${sectionMarkdown.trimEnd()}`;
    } else {
      body = `${body}\n\n${sectionMarkdown.trimEnd()}`;
    }
  }

  // Update compare reference links
  const updatedLinks = new Map();
  updatedLinks.set("Unreleased", `${repoUrl}/compare/v${version}...HEAD`);
  const versionUrl = prevTag
    ? `${repoUrl}/compare/${prevTag}...v${version}`
    : `${repoUrl}/releases/tag/v${version}`;
  updatedLinks.set(version, versionUrl);

  for (const [label, url] of existingLinks.entries()) {
    if (!updatedLinks.has(label)) {
      updatedLinks.set(label, url);
    }
  }

  const linkLines = Array.from(updatedLinks.entries()).map(
    ([label, url]) => `[${label}]: ${url}`,
  );

  return `${body.trimEnd()}\n\n${linkLines.join("\n")}\n`;
}

export function extractReleaseNotes(changelogContent, version) {
  const cleanVersion = parseSemver(version).raw;
  const normalized = String(changelogContent ?? "").replace(/\r\n/g, "\n");
  const escaped = escapeRegExp(cleanVersion);
  const headingRe = new RegExp(`^## \\[${escaped}\\][^\\n]*\\n`, "m");
  const startMatch = normalized.match(headingRe);
  if (!startMatch) return null;

  const afterHeading = normalized.slice(startMatch.index + startMatch[0].length);
  const nextBoundary = afterHeading.search(/^(?:## \[|\[[^\]]+\]:\s*https?:\/\/)/m);
  const sectionBody =
    nextBoundary >= 0 ? afterHeading.slice(0, nextBoundary) : afterHeading;

  const trimmed = sectionBody.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function readJson(filePath) {
  return JSON.parse(readFileSync(filePath, "utf8"));
}

function writeJson(filePath, data) {
  writeFileSync(filePath, `${JSON.stringify(data, null, 2)}\n`, "utf8");
}

function findCargoPackageSectionRange(tomlText) {
  const normalized = tomlText.replace(/\r\n/g, "\n");
  const headerMatch = normalized.match(/^\[package\][ \t]*$/m);
  if (!headerMatch) return null;
  const start = headerMatch.index + headerMatch[0].length;
  const rest = normalized.slice(start);
  const nextSectionOffset = rest.search(/\n\[/);
  const end = nextSectionOffset >= 0 ? start + nextSectionOffset : normalized.length;
  return { normalized, start, end };
}

function readCargoTomlVersion(tomlText) {
  const range = findCargoPackageSectionRange(tomlText);
  if (!range) return null;
  const section = range.normalized.slice(range.start, range.end);
  const verMatch = section.match(/^version\s*=\s*"([^"]+)"/m);
  return verMatch ? verMatch[1] : null;
}

function updateCargoTomlVersion(tomlText, newVersion) {
  const range = findCargoPackageSectionRange(tomlText);
  if (!range) {
    throw new Error("Could not find [package] section in src-tauri/Cargo.toml");
  }
  const section = range.normalized.slice(range.start, range.end);
  if (!/^version\s*=\s*"[^"]+"/m.test(section)) {
    throw new Error("Could not find version field in [package] section of Cargo.toml");
  }
  const updatedSection = section.replace(
    /^(version\s*=\s*")[^"]+(")/m,
    `$1${newVersion}$2`,
  );
  return (
    range.normalized.slice(0, range.start) +
    updatedSection +
    range.normalized.slice(range.end)
  );
}

function readCargoLockVersion(lockText, packageName = "easysrt-studio") {
  const re = new RegExp(
    `\\[\\[package\\]\\]\\s*\\nname\\s*=\\s*"${escapeRegExp(packageName)}"\\s*\\nversion\\s*=\\s*"([^"]+)"`,
  );
  const match = lockText.match(re);
  return match ? match[1] : null;
}

function updateCargoLockVersion(lockText, newVersion, packageName = "easysrt-studio") {
  const re = new RegExp(
    `(\\[\\[package\\]\\]\\s*\\nname\\s*=\\s*"${escapeRegExp(packageName)}"\\s*\\nversion\\s*=\\s*")[^"]+(")`,
  );
  if (!re.test(lockText)) {
    throw new Error(`Could not find package "${packageName}" in src-tauri/Cargo.lock`);
  }
  return lockText.replace(re, `$1${newVersion}$2`);
}

export function readReadmeVersions(readmeText) {
  const badgeAlt = readmeText.match(/\[!\[(?:Release|Izdanje) v([^\]]+)\]/);
  const badgeUrl = readmeText.match(
    /\/badge\/(?:release|izdanje)-v([0-9A-Za-z.-]+)-[0-9a-fA-F]{6}\b/,
  );
  const heading = readmeText.match(
    /\[(?:Download|Preuzmi) EasySRT Studio v([^\]]+)\]/,
  );
  const found = [
    badgeAlt?.[1],
    badgeUrl?.[1]?.replace(/--/g, "-"),
    heading?.[1],
  ].filter(Boolean);
  if (found.length === 0) return null;
  const unique = Array.from(new Set(found));
  return unique.length === 1 ? unique[0] : unique.join(" / ");
}

export function updateReadmeVersions(readmeText, newVersion) {
  const badgeVersion = newVersion.replace(/-/g, "--");
  let next = readmeText.replace(
    /\[!\[(Release|Izdanje) v[^\]]+\]\(https:\/\/img\.shields\.io\/badge\/(release|izdanje)-v[0-9A-Za-z.-]+-([0-9a-fA-F]{6}\b)/g,
    `[![$1 v${newVersion}](https://img.shields.io/badge/$2-v${badgeVersion}-$3`,
  );
  next = next.replace(
    /\[(Download|Preuzmi) EasySRT Studio v[^\]]+\]/g,
    `[$1 EasySRT Studio v${newVersion}]`,
  );
  return next;
}

export function checkVersions(rootDir) {
  const pkgPath = path.join(rootDir, "package.json");
  const expectedVersion = readJson(pkgPath).version;
  parseSemver(expectedVersion);

  const entries = [];
  const record = (relPath, version) => {
    entries.push({
      file: relPath,
      version,
      ok: version === expectedVersion,
    });
  };

  record("package.json", expectedVersion);

  const pkgLockPath = path.join(rootDir, "package-lock.json");
  if (existsSync(pkgLockPath)) {
    const lock = readJson(pkgLockPath);
    record("package-lock.json (version)", lock.version ?? null);
    record("package-lock.json (packages[''].version)", lock.packages?.[""]?.version ?? null);
  }

  const tauriConfPath = path.join(rootDir, "src-tauri", "tauri.conf.json");
  if (existsSync(tauriConfPath)) {
    record("src-tauri/tauri.conf.json", readJson(tauriConfPath).version ?? null);
  }

  const cargoTomlPath = path.join(rootDir, "src-tauri", "Cargo.toml");
  if (existsSync(cargoTomlPath)) {
    record(
      "src-tauri/Cargo.toml",
      readCargoTomlVersion(readFileSync(cargoTomlPath, "utf8")),
    );
  }

  const cargoLockPath = path.join(rootDir, "src-tauri", "Cargo.lock");
  if (existsSync(cargoLockPath)) {
    record(
      "src-tauri/Cargo.lock",
      readCargoLockVersion(readFileSync(cargoLockPath, "utf8")),
    );
  }

  const workerPkgPath = path.join(rootDir, "worker", "package.json");
  if (existsSync(workerPkgPath)) {
    record("worker/package.json", readJson(workerPkgPath).version ?? null);
  }

  const workerLockPath = path.join(rootDir, "worker", "package-lock.json");
  if (existsSync(workerLockPath)) {
    const wLock = readJson(workerLockPath);
    record("worker/package-lock.json (version)", wLock.version ?? null);
    record(
      "worker/package-lock.json (packages[''].version)",
      wLock.packages?.[""]?.version ?? null,
    );
  }

  for (const readmeFile of README_FILES) {
    const readmePath = path.join(rootDir, readmeFile);
    if (existsSync(readmePath)) {
      const readmeVersion = readReadmeVersions(readFileSync(readmePath, "utf8"));
      if (readmeVersion !== null) {
        record(readmeFile, readmeVersion);
      }
    }
  }

  return {
    ok: entries.every((e) => e.ok),
    expectedVersion,
    entries,
  };
}

export function syncVersions(rootDir, newVersion) {
  const cleanVersion = parseSemver(newVersion).raw;
  const touched = [];

  const pkgPath = path.join(rootDir, "package.json");
  const pkg = readJson(pkgPath);
  pkg.version = cleanVersion;
  writeJson(pkgPath, pkg);
  touched.push("package.json");

  const pkgLockPath = path.join(rootDir, "package-lock.json");
  if (existsSync(pkgLockPath)) {
    const lock = readJson(pkgLockPath);
    lock.version = cleanVersion;
    if (lock.packages?.[""]) {
      lock.packages[""].version = cleanVersion;
    }
    writeJson(pkgLockPath, lock);
    touched.push("package-lock.json");
  }

  const tauriConfPath = path.join(rootDir, "src-tauri", "tauri.conf.json");
  if (existsSync(tauriConfPath)) {
    const tauriConf = readJson(tauriConfPath);
    tauriConf.version = cleanVersion;
    writeJson(tauriConfPath, tauriConf);
    touched.push("src-tauri/tauri.conf.json");
  }

  const cargoTomlPath = path.join(rootDir, "src-tauri", "Cargo.toml");
  if (existsSync(cargoTomlPath)) {
    const updated = updateCargoTomlVersion(
      readFileSync(cargoTomlPath, "utf8"),
      cleanVersion,
    );
    writeFileSync(cargoTomlPath, updated, "utf8");
    touched.push("src-tauri/Cargo.toml");
  }

  const cargoLockPath = path.join(rootDir, "src-tauri", "Cargo.lock");
  if (existsSync(cargoLockPath)) {
    const updated = updateCargoLockVersion(
      readFileSync(cargoLockPath, "utf8"),
      cleanVersion,
    );
    writeFileSync(cargoLockPath, updated, "utf8");
    touched.push("src-tauri/Cargo.lock");
  }

  const workerPkgPath = path.join(rootDir, "worker", "package.json");
  if (existsSync(workerPkgPath)) {
    const wPkg = readJson(workerPkgPath);
    wPkg.version = cleanVersion;
    writeJson(workerPkgPath, wPkg);
    touched.push("worker/package.json");
  }

  const workerLockPath = path.join(rootDir, "worker", "package-lock.json");
  if (existsSync(workerLockPath)) {
    const wLock = readJson(workerLockPath);
    wLock.version = cleanVersion;
    if (wLock.packages?.[""]) {
      wLock.packages[""].version = cleanVersion;
    }
    writeJson(workerLockPath, wLock);
    touched.push("worker/package-lock.json");
  }

  for (const readmeFile of README_FILES) {
    const readmePath = path.join(rootDir, readmeFile);
    if (existsSync(readmePath)) {
      const updated = updateReadmeVersions(
        readFileSync(readmePath, "utf8"),
        cleanVersion,
      );
      writeFileSync(readmePath, updated, "utf8");
      touched.push(readmeFile);
    }
  }

  return touched;
}

function runGit(rootDir, args) {
  return execFileSync("git", args, {
    cwd: rootDir,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
}

export function getLastStableTag(rootDir) {
  const tagsRaw = runGit(rootDir, ["tag", "--sort=-v:refname"]);
  if (!tagsRaw) return null;
  for (const tag of tagsRaw.split("\n")) {
    if (/^v\d+\.\d+\.\d+$/.test(tag.trim())) {
      return tag.trim();
    }
  }
  return null;
}

export function getGitCommitsSince(rootDir, fromTag) {
  const range = fromTag ? `${fromTag}..HEAD` : "HEAD";
  const format = "%H%x1f%s%x1f%b%x1e";
  const raw = runGit(rootDir, ["log", "--no-merges", `--format=${format}`, range]);
  if (!raw) return [];

  return raw
    .split("\x1e")
    .map((record) => record.trim())
    .filter(Boolean)
    .map((record) => {
      const [hash = "", subject = "", body = ""] = record.split("\x1f");
      return { hash: hash.trim(), subject: subject.trim(), body: body.trim() };
    });
}

function parseCliArgs(argv) {
  const positional = [];
  const flags = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg.startsWith("--")) {
      const key = arg.slice(2);
      const next = argv[i + 1];
      if (next !== undefined && !next.startsWith("--")) {
        flags[key] = next;
        i += 1;
      } else {
        flags[key] = true;
      }
    } else {
      positional.push(arg);
    }
  }
  return { positional, flags };
}

function formatTodayUtc() {
  return new Date().toISOString().slice(0, 10);
}

function printUsage() {
  console.log(`Usage: node scripts/release.mjs <command> [options]

Commands:
  check
      Verify all project manifests & README.md match package.json version.

  prepare <patch|minor|major|X.Y.Z> [--date YYYY-MM-DD] [--from <tag>] [--dry-run]
      Generate/update CHANGELOG.md from commits since the last stable tag
      and synchronize the new version across all manifests.

  notes <version> [--with-footer] [--prerelease]
      Print release notes for <version> from CHANGELOG.md (or commit log for --prerelease).
`);
}

function main() {
  const rootDir = path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "..",
  );
  const { positional, flags } = parseCliArgs(process.argv.slice(2));
  const command = positional[0];

  if (!command || command === "help" || flags.help) {
    printUsage();
    process.exit(0);
  }

  if (command === "check") {
    const result = checkVersions(rootDir);
    if (!result.ok) {
      console.error(
        `Version mismatch detected (expected ${result.expectedVersion} from package.json):`,
      );
      for (const entry of result.entries) {
        const icon = entry.ok ? "OK " : "ERR";
        console.error(`  [${icon}] ${entry.file}: ${entry.version ?? "<missing>"}`);
      }
      process.exit(1);
    }
    console.log(
      `All ${result.entries.length} version references match ${result.expectedVersion}.`,
    );
    return;
  }

  if (command === "prepare") {
    const spec = positional[1];
    if (!spec) {
      console.error("Missing version bump target: pass patch, minor, major, or X.Y.Z");
      process.exit(1);
    }

    const currentVersion = readJson(path.join(rootDir, "package.json")).version;
    const nextVersion = bumpSemver(currentVersion, spec);
    const date = typeof flags.date === "string" ? flags.date : formatTodayUtc();
    const prevTag =
      typeof flags.from === "string" ? flags.from : getLastStableTag(rootDir);
    const commits = getGitCommitsSince(rootDir, prevTag);
    const sectionMarkdown = buildChangelogSection({
      version: nextVersion,
      date,
      commits,
    });

    if (flags["dry-run"]) {
      console.log(`Dry run: ${currentVersion} -> ${nextVersion} (since ${prevTag ?? "initial commit"})`);
      console.log("");
      console.log(sectionMarkdown);
      return;
    }

    const changelogPath = path.join(rootDir, "CHANGELOG.md");
    const existingChangelog = existsSync(changelogPath)
      ? readFileSync(changelogPath, "utf8")
      : "";
    const nextChangelog = upsertChangelog(existingChangelog, {
      version: nextVersion,
      prevTag,
      sectionMarkdown,
    });
    writeFileSync(changelogPath, nextChangelog, "utf8");

    const touched = syncVersions(rootDir, nextVersion);
    const verification = checkVersions(rootDir);
    if (!verification.ok) {
      console.error("Post-sync version verification failed:", verification.entries);
      process.exit(1);
    }

    console.log(`Prepared release v${nextVersion} (previous tag: ${prevTag ?? "none"})`);
    console.log(`Updated CHANGELOG.md and ${touched.length} manifest files:`);
    for (const file of touched) {
      console.log(`  - ${file}`);
    }
    return;
  }

  if (command === "notes") {
    const versionArg = positional[1];
    if (!versionArg) {
      console.error("Missing version argument for `notes` command.");
      process.exit(1);
    }

    const withFooter = Boolean(flags["with-footer"]);
    const isPrerelease = Boolean(flags.prerelease);
    const changelogPath = path.join(rootDir, "CHANGELOG.md");

    let body = null;
    if (!isPrerelease && existsSync(changelogPath)) {
      body = extractReleaseNotes(readFileSync(changelogPath, "utf8"), versionArg);
    }

    if (!body) {
      const prevTag = getLastStableTag(rootDir);
      const commits = getGitCommitsSince(rootDir, prevTag);
      const generated = buildChangelogSection({
        version: versionArg.replace(/^v/, ""),
        date: formatTodayUtc(),
        commits,
      });
      // Strip the top ## [...] heading for GitHub Release body
      const sectionLines = generated.split("\n").slice(1).join("\n").trim();
      body = isPrerelease
        ? `> **Staging Pre-release** — built from \`staging\` for testing. Production auto-updater (\`latest.json\`) is not affected.\n\n${sectionLines}`
        : sectionLines;
    }

    const output = withFooter ? `${body}\n\n${DEFAULT_DOWNLOAD_FOOTER}` : body;
    process.stdout.write(`${output}\n`);
    return;
  }

  console.error(`Unknown command: ${command}`);
  printUsage();
  process.exit(1);
}

const isDirectRun =
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;

if (isDirectRun) {
  main();
}
