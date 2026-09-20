#!/usr/bin/env node

import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const root = process.cwd();
const excludedDirectories = new Set([
  ".git",
  "node_modules",
  "recordings",
  "target",
  "dist",
  "gen",
  "coverage",
  ".venv",
  "__pycache__",
  "evidence",
  "notifications",
]);
const textExtensions = new Set([
  ".js",
  ".json",
  ".jsx",
  ".md",
  ".mjs",
  ".tkt",
  ".tickets",
  ".toml",
  ".ts",
  ".tsx",
  ".yaml",
  ".yml",
]);
const textBasenames = new Set([".editorconfig", ".gitignore", ".nvmrc", "pre-commit"]);

async function collectFiles(directory = root) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && excludedDirectories.has(entry.name)) continue;
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...await collectFiles(absolute));
    else if (entry.isFile()) files.push(absolute);
  }
  return files;
}

function relative(file) {
  return path.relative(root, file).split(path.sep).join("/");
}

function validateText(file, content, failures) {
  const lines = content.split("\n");
  lines.forEach((line, index) => {
    if (/[ \t]+$/.test(line)) failures.push(`${relative(file)}:${index + 1} has trailing whitespace`);
  });
  if (content.length && !content.endsWith("\n")) failures.push(`${relative(file)} needs a final newline`);
  if (content.includes("\r")) failures.push(`${relative(file)} must use LF line endings`);
}

function validateTickets(ticketFiles, contents, failures) {
  const ids = new Map();
  for (const file of ticketFiles) {
    const content = contents.get(file);
    content.split("\n").forEach((line, index) => {
      const status = line.match(/^(\s*)[@#*!$^~]- /);
      if (!status) return;
      const id = line.match(/\[([A-Z][A-Z0-9]*-\d{4,})\]/)?.[1];
      if (!id) failures.push(`${relative(file)}:${index + 1} ticket has no identifier`);
      else if (ids.has(id)) failures.push(`${relative(file)}:${index + 1} duplicates ${id} from ${ids.get(id)}`);
      else ids.set(id, `${relative(file)}:${index + 1}`);

      if (status[1].length === 0 && !line.includes("[EPIC]")) {
        failures.push(`${relative(file)}:${index + 1} top-level ticket is not an epic`);
      }
      if (status[1].length > 0 && status[1].length < 4) {
        failures.push(`${relative(file)}:${index + 1} child ticket needs at least four spaces of indentation`);
      }
    });
  }
}

function validateSensitiveLogging(contents, failures) {
  const frontendRoot = "apps/desktop/src/";
  const rustRoot = "apps/desktop/src-tauri/src/";

  for (const [file, content] of contents) {
    const name = relative(file);
    if (name.startsWith(frontendRoot) && /console\.(?:log|debug|info|warn|error)\s*\(/.test(content)) {
      failures.push(`${name} must not log from the meeting-content UI`);
    }
    if (name.startsWith(rustRoot)) {
      const productionContent = content.split("#[cfg(test)]", 1)[0];
      if (/\b(?:print|println|eprint|eprintln)!\s*\(/.test(productionContent)) {
        failures.push(`${name} must not print from the meeting-content backend`);
      }
    }
  }
}

async function main() {
  const allFiles = await collectFiles();
  const checkedFiles = allFiles.filter((file) => (
    textExtensions.has(path.extname(file)) || textBasenames.has(path.basename(file))
  ));
  const contents = new Map();
  const failures = [];

  for (const file of checkedFiles) {
    const content = await readFile(file, "utf8");
    contents.set(file, content);
    validateText(file, content, failures);
    if (path.extname(file) === ".json") {
      try { JSON.parse(content); } catch (error) { failures.push(`${relative(file)} contains invalid JSON: ${error.message}`); }
    }
  }

  const ticketFiles = checkedFiles.filter((file) => [".tkt", ".tickets"].includes(path.extname(file)));
  validateTickets(ticketFiles, contents, failures);
  validateSensitiveLogging(contents, failures);
  if (failures.length) {
    console.error(failures.map((failure) => `- ${failure}`).join("\n"));
    process.exitCode = 1;
    return;
  }

  console.log(`Structure checks passed for ${checkedFiles.length} text files and ${ticketFiles.length} ticket tracker.`);
}

main().catch((error) => {
  console.error(error.stack || error.message);
  process.exitCode = 1;
});
