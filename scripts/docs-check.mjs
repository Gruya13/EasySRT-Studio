#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const EXTERNAL_SCHEME_RE = /^(?:https?:|mailto:|tel:|data:|javascript:|file:)/i;

export function stripFencedCodeBlocks(markdown) {
  const normalized = String(markdown ?? "").replace(/\r\n/g, "\n");
  return normalized.replace(
    /^([ \t]*)(`{3,}|~{3,})[^\n]*\n[\s\S]*?^\1\2[ \t]*$/gm,
    "",
  );
}

export function stripCodeSpans(markdown) {
  return stripFencedCodeBlocks(markdown).replace(/`+[^`\n]*`+/g, "");
}

export function countH2Headings(markdown) {
  const withoutFences = stripFencedCodeBlocks(markdown);
  const matches = withoutFences.match(/^##\s+\S.*$/gm);
  return matches ? matches.length : 0;
}

export function extractLocalLinks(markdown) {
  const cleaned = stripCodeSpans(markdown);
  const targets = [];

  const mdLinkRe = /\]\((?:<([^>\n]+)>|([^)\s\n]+))(?:\s+"[^"]*")?\)/g;
  let match;
  while ((match = mdLinkRe.exec(cleaned)) !== null) {
    const raw = (match[1] ?? match[2] ?? "").trim();
    if (raw) targets.push(raw);
  }

  const htmlAttrRe = /\b(?:href|src)\s*=\s*["']([^"']+)["']/gi;
  while ((match = htmlAttrRe.exec(cleaned)) !== null) {
    const raw = (match[1] ?? "").trim();
    if (raw) targets.push(raw);
  }

  const localTargets = [];
  for (const raw of targets) {
    if (EXTERNAL_SCHEME_RE.test(raw) || raw.startsWith("#")) {
      continue;
    }
    const withoutFragment = raw.split("#")[0].split("?")[0].trim();
    if (!withoutFragment) continue;
    let decoded = withoutFragment;
    try {
      decoded = decodeURIComponent(withoutFragment);
    } catch {
      // Keep raw string if percent-decoding fails
    }
    localTargets.push({ raw, targetPath: decoded });
  }

  return localTargets;
}

function collectMarkdownFiles(dirPath, rootDir) {
  if (!existsSync(dirPath)) return [];
  const results = [];
  for (const entry of readdirSync(dirPath, { withFileTypes: true })) {
    const abs = path.join(dirPath, entry.name);
    if (entry.isDirectory()) {
      results.push(...collectMarkdownFiles(abs, rootDir));
    } else if (entry.isFile() && entry.name.toLowerCase().endsWith(".md")) {
      results.push(path.relative(rootDir, abs).split(path.sep).join("/"));
    }
  }
  return results.sort();
}

function collectAllFiles(dirPath, rootDir) {
  if (!existsSync(dirPath)) return [];
  const results = [];
  for (const entry of readdirSync(dirPath, { withFileTypes: true })) {
    const abs = path.join(dirPath, entry.name);
    if (entry.isDirectory()) {
      results.push(...collectAllFiles(abs, rootDir));
    } else if (entry.isFile()) {
      results.push(path.relative(rootDir, abs).split(path.sep).join("/"));
    }
  }
  return results.sort();
}

function getChangedFilesSinceBase(rootDir, baseRef) {
  const attempts = [
    ["diff", "--name-only", `${baseRef}...HEAD`],
    ["diff", "--name-only", baseRef, "HEAD"],
  ];
  let lastErr = null;
  for (const args of attempts) {
    try {
      const out = execFileSync("git", args, {
        cwd: rootDir,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      }).trim();
      if (!out) return [];
      return out
        .split("\n")
        .map((line) => line.trim().replace(/\\/g, "/"))
        .filter(Boolean);
    } catch (err) {
      lastErr = err;
    }
  }
  throw new Error(
    `Unable to compute git diff against base ref "${baseRef}": ${lastErr?.message ?? String(lastErr)}`,
  );
}

export function checkLanguageCoModification(changedFiles) {
  const errors = [];
  const changed = new Set(
    changedFiles.map((f) => f.trim().replace(/\\/g, "/")),
  );

  const enReadmeChanged = changed.has("README.md");
  const srReadmeChanged = changed.has("README.sr.md");
  if (enReadmeChanged !== srReadmeChanged) {
    errors.push(
      `Marketing README language drift: "${enReadmeChanged ? "README.md" : "README.sr.md"}" was modified without updating "${enReadmeChanged ? "README.sr.md" : "README.md"}".`,
    );
  }

  const userDocPrefixEn = "docs/user/en/";
  const userDocPrefixSr = "docs/user/sr/";
  const touchedUserNames = new Set();

  for (const file of changed) {
    if (file.startsWith(userDocPrefixEn) && file.endsWith(".md")) {
      touchedUserNames.add(file.slice(userDocPrefixEn.length));
    } else if (file.startsWith(userDocPrefixSr) && file.endsWith(".md")) {
      touchedUserNames.add(file.slice(userDocPrefixSr.length));
    }
  }

  for (const relName of Array.from(touchedUserNames).sort()) {
    const enPath = `${userDocPrefixEn}${relName}`;
    const srPath = `${userDocPrefixSr}${relName}`;
    if (changed.has(enPath) !== changed.has(srPath)) {
      const modified = changed.has(enPath) ? enPath : srPath;
      const missing = changed.has(enPath) ? srPath : enPath;
      errors.push(
        `User guide language drift: "${modified}" was modified without updating "${missing}".`,
      );
    }
  }

  return errors;
}

export function checkDocs(
  rootDir,
  {
    strictMain = false,
    baseRef = null,
    changedFiles = null,
    allowSingleLang = false,
  } = {},
) {
  const errors = [];

  // 1. Bilingual root marketing READMEs
  const readmeEnPath = path.join(rootDir, "README.md");
  const readmeSrPath = path.join(rootDir, "README.sr.md");

  if (!existsSync(readmeEnPath)) {
    errors.push("Missing root marketing file: README.md");
  }
  if (!existsSync(readmeSrPath)) {
    errors.push("Missing Serbian root marketing file: README.sr.md");
  }

  if (existsSync(readmeEnPath) && existsSync(readmeSrPath)) {
    const enLinks = extractLocalLinks(readFileSync(readmeEnPath, "utf8")).map(
      (l) => path.posix.normalize(l.targetPath),
    );
    const srLinks = extractLocalLinks(readFileSync(readmeSrPath, "utf8")).map(
      (l) => path.posix.normalize(l.targetPath),
    );

    if (!enLinks.includes("README.sr.md")) {
      errors.push("README.md must include a language switch link to README.sr.md");
    }
    if (!srLinks.includes("README.md")) {
      errors.push("README.sr.md must include a language switch link to README.md");
    }
  }

  // 2. Bilingual user guides parity (docs/user/en <-> docs/user/sr)
  const userEnDir = path.join(rootDir, "docs", "user", "en");
  const userSrDir = path.join(rootDir, "docs", "user", "sr");
  const userEnFiles = collectMarkdownFiles(userEnDir, userEnDir);
  const userSrFiles = collectMarkdownFiles(userSrDir, userSrDir);

  if (userEnFiles.length === 0) {
    errors.push("docs/user/en/ must contain at least one Markdown user guide.");
  }
  if (userSrFiles.length === 0) {
    errors.push("docs/user/sr/ must contain at least one Markdown user guide.");
  }

  const enSet = new Set(userEnFiles);
  const srSet = new Set(userSrFiles);

  for (const file of userEnFiles) {
    if (!srSet.has(file)) {
      errors.push(
        `Missing Serbian user guide counterpart: docs/user/sr/${file} (for docs/user/en/${file})`,
      );
    }
  }
  for (const file of userSrFiles) {
    if (!enSet.has(file)) {
      errors.push(
        `Missing English user guide counterpart: docs/user/en/${file} (for docs/user/sr/${file})`,
      );
    }
  }

  for (const file of userEnFiles) {
    if (!srSet.has(file)) continue;
    const enAbs = path.join(userEnDir, file);
    const srAbs = path.join(userSrDir, file);
    const enText = readFileSync(enAbs, "utf8");
    const srText = readFileSync(srAbs, "utf8");

    const enH2 = countH2Headings(enText);
    const srH2 = countH2Headings(srText);
    if (enH2 !== srH2) {
      errors.push(
        `Section count mismatch between docs/user/en/${file} (${enH2} "##" headings) and docs/user/sr/${file} (${srH2} "##" headings).`,
      );
    }

    const expectedSrTarget = path.resolve(srAbs);
    const enResolvedTargets = extractLocalLinks(enText).map((l) =>
      path.resolve(path.dirname(enAbs), l.targetPath),
    );
    if (!enResolvedTargets.includes(expectedSrTarget)) {
      errors.push(
        `docs/user/en/${file} must include a language switch link to ../sr/${file}`,
      );
    }

    const expectedEnTarget = path.resolve(enAbs);
    const srResolvedTargets = extractLocalLinks(srText).map((l) =>
      path.resolve(path.dirname(srAbs), l.targetPath),
    );
    if (!srResolvedTargets.includes(expectedEnTarget)) {
      errors.push(
        `docs/user/sr/${file} must include a language switch link to ../en/${file}`,
      );
    }
  }

  // 3. Relative link & image integrity across root + docs/**/*.md
  const rootMarkdownCandidates = [
    "README.md",
    "README.sr.md",
    "CHANGELOG.md",
    "AGENTS.md",
  ].filter((rel) => existsSync(path.join(rootDir, rel)));

  const docsMarkdownFiles = collectMarkdownFiles(
    path.join(rootDir, "docs"),
    rootDir,
  );
  const allCheckedMarkdown = [
    ...rootMarkdownCandidates,
    ...docsMarkdownFiles,
  ];

  for (const relFile of allCheckedMarkdown) {
    const absFile = path.join(rootDir, relFile);
    const content = readFileSync(absFile, "utf8");
    const links = extractLocalLinks(content);

    for (const { raw, targetPath } of links) {
      const resolved = path.resolve(path.dirname(absFile), targetPath);
      if (!existsSync(resolved)) {
        errors.push(
          `Broken relative link in ${relFile}: "${raw}" -> missing path "${path.relative(rootDir, resolved).split(path.sep).join("/")}"`,
        );
        continue;
      }
      // Ensure link stays inside repository root
      const relToRoot = path.relative(rootDir, resolved);
      if (relToRoot.startsWith("..") || path.isAbsolute(relToRoot)) {
        errors.push(
          `Link escapes repository root in ${relFile}: "${raw}"`,
        );
      } else if (statSync(resolved).isDirectory()) {
        // Directory links are allowed on GitHub, no further check needed
      }
    }
  }

  // 4. Marketing & user layer must never reference docs/planning
  const publicFacingFiles = [
    "README.md",
    "README.sr.md",
    "CHANGELOG.md",
    ...docsMarkdownFiles.filter((f) => f.startsWith("docs/user/")),
  ].filter((rel) => existsSync(path.join(rootDir, rel)));

  for (const relFile of publicFacingFiles) {
    const cleaned = stripCodeSpans(
      readFileSync(path.join(rootDir, relFile), "utf8"),
    );
    if (/\bdocs\/planning\b/i.test(cleaned) || /\.\.\/planning\b/i.test(cleaned)) {
      errors.push(
        `Public/marketing file "${relFile}" must not reference internal "docs/planning".`,
      );
    }
  }

  // 5. Strict main gate: docs/planning must not exist (or must be empty) on main
  const planningDir = path.join(rootDir, "docs", "planning");
  const planningFiles = collectAllFiles(planningDir, rootDir);
  if (strictMain && planningFiles.length > 0) {
    errors.push(
      `Strict main gate failed: ephemeral planning files must be distilled and removed before merging to main (${planningFiles.join(", ")}).`,
    );
  }

  // 6. Optional git diff co-modification check
  if (!allowSingleLang && (Array.isArray(changedFiles) || baseRef)) {
    try {
      const diffList = Array.isArray(changedFiles)
        ? changedFiles
        : getChangedFilesSinceBase(rootDir, baseRef);
      errors.push(...checkLanguageCoModification(diffList));
    } catch (err) {
      errors.push(err.message ?? String(err));
    }
  }

  return {
    ok: errors.length === 0,
    errors,
    checkedMarkdownCount: allCheckedMarkdown.length,
    planningFileCount: planningFiles.length,
  };
}

function parseCliArgs(argv) {
  const flags = {
    strictMain: false,
    allowSingleLang: false,
    baseRef: null,
    help: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--strict-main") {
      flags.strictMain = true;
    } else if (arg === "--allow-single-lang") {
      flags.allowSingleLang = true;
    } else if (arg === "--base") {
      flags.baseRef = argv[i + 1] ?? null;
      i += 1;
    } else if (arg.startsWith("--base=")) {
      flags.baseRef = arg.slice("--base=".length) || null;
    } else if (arg === "--help" || arg === "-h") {
      flags.help = true;
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }
  return flags;
}

function printUsage() {
  console.log(`Usage: node scripts/docs-check.mjs [options]

Options:
  --strict-main         Fail if docs/planning/ contains any files (required for main branch / PRs to main).
  --base <git-ref>      Verify EN/SR co-modification parity across files changed since <git-ref>.
  --allow-single-lang   Bypass the --base single-language modification check (e.g. typo-only fix).
  --help, -h            Show this help message.
`);
}

function main() {
  const rootDir = path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "..",
  );
  let flags;
  try {
    flags = parseCliArgs(process.argv.slice(2));
  } catch (err) {
    console.error(err.message ?? String(err));
    printUsage();
    process.exit(1);
  }

  if (flags.help) {
    printUsage();
    process.exit(0);
  }

  const result = checkDocs(rootDir, {
    strictMain: flags.strictMain,
    baseRef: flags.baseRef,
    allowSingleLang: flags.allowSingleLang,
  });

  if (!result.ok) {
    console.error(`Documentation check failed (${result.errors.length} issue(s)):`);
    for (const err of result.errors) {
      console.error(`  [ERR] ${err}`);
    }
    process.exit(1);
  }

  const modeLabel = flags.strictMain ? "strict-main" : "staging";
  console.log(
    `Documentation check passed (${result.checkedMarkdownCount} Markdown files verified, mode: ${modeLabel}, active planning docs: ${result.planningFileCount}).`,
  );
}

const isDirectRun =
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;

if (isDirectRun) {
  main();
}
