#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

function readJson(filePath, fallback) {
  try {
    if (fs.existsSync(filePath)) {
      return JSON.parse(fs.readFileSync(filePath, "utf8"));
    }
  } catch (error) {
    // Ignore malformed files and use the fallback.
  }
  return fallback;
}

function findProjectRoot() {
  const rawCandidates = [
    process.env.CLAUDE_PROJECT_DIR,
    process.cwd(),
    path.resolve(__dirname, "..", ".."),
  ].filter(Boolean);
  const seen = new Set();
  for (const candidate of rawCandidates) {
    let current = path.resolve(candidate);
    while (true) {
      if (seen.has(current)) {
        break;
      }
      seen.add(current);
      const sentinels = [
        path.join(current, ".claude-flow", "config.json"),
        path.join(current, ".claude"),
        path.join(current, ".git"),
      ];
      if (sentinels.some((entry) => fs.existsSync(entry))) {
        return current;
      }
      const parent = path.dirname(current);
      if (parent === current) {
        break;
      }
      current = parent;
    }
  }
  return path.resolve(__dirname, "..", "..");
}

const root = findProjectRoot();
const config = readJson(path.join(root, ".claude-flow", "config.json"), {
  profile: "minimal",
  features: { hooks: true, statusLine: true, autoMemory: false, agentTeams: false },
});
const session = readJson(path.join(root, ".claude-flow", "sessions", "current.json"), null);
const eventsPath = path.join(root, ".claude-flow", "metrics", "hook-events.jsonl");
let eventCount = 0;
if (fs.existsSync(eventsPath)) {
  try {
    const content = fs.readFileSync(eventsPath, "utf8").trim();
    eventCount = content ? content.split(/\r?\n/).length : 0;
  } catch (error) {
    eventCount = 0;
  }
}
const sessionLabel = session && session.id ? session.id.slice(-8) : "none";
const line = [
  `cf:${config.profile || "minimal"}`,
  `hooks:${config.features && config.features.hooks === false ? "off" : "on"}`,
  `memory:${config.features && config.features.autoMemory ? "on" : "off"}`,
  `teams:${config.features && config.features.agentTeams ? "on" : "off"}`,
  `events:${eventCount}`,
  `session:${sessionLabel}`,
].join(" ");
process.stdout.write(line + "\n");
