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

function readStdinJson() {
  try {
    const input = fs.readFileSync(0, "utf8").trim();
    return input ? JSON.parse(input) : {};
  } catch (error) {
    return {};
  }
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

const PROJECT_ROOT = findProjectRoot();
const CONFIG_PATH = path.join(PROJECT_ROOT, ".claude-flow", "config.json");
const SESSIONS_DIR = path.join(PROJECT_ROOT, ".claude-flow", "sessions");
const CURRENT_SESSION_PATH = path.join(SESSIONS_DIR, "current.json");
const METRICS_DIR = path.join(PROJECT_ROOT, ".claude-flow", "metrics");
const EVENTS_PATH = path.join(METRICS_DIR, "hook-events.jsonl");

function ensureDirectory(dirPath) {
  fs.mkdirSync(dirPath, { recursive: true });
}

function defaultConfig() {
  return {
    schemaVersion: 1,
    profile: "minimal",
    features: {
      hooks: true,
      statusLine: true,
      autoMemory: false,
      agentTeams: false,
    },
    memory: {
      importOnSessionStart: false,
      syncOnStop: false,
      storePath: ".claude-flow/data/auto-memory-store.json",
    },
  };
}

function loadConfig() {
  const fallback = defaultConfig();
  const data = readJson(CONFIG_PATH, fallback);
  const features = data.features || {};
  const memory = data.memory || {};
  return {
    schemaVersion: typeof data.schemaVersion === "number" ? data.schemaVersion : 1,
    profile: data.profile === "full" ? "full" : "minimal",
    features: {
      hooks: features.hooks !== false,
      statusLine: features.statusLine !== false,
      autoMemory: features.autoMemory === true,
      agentTeams: features.agentTeams === true,
    },
    memory: {
      importOnSessionStart: memory.importOnSessionStart === true,
      syncOnStop: memory.syncOnStop === true,
      storePath: typeof memory.storePath === "string" && memory.storePath ? memory.storePath : ".claude-flow/data/auto-memory-store.json",
    },
  };
}

function appendEvent(eventName, payload) {
  ensureDirectory(METRICS_DIR);
  const entry = {
    timestamp: new Date().toISOString(),
    event: eventName,
    sessionId: payload.session_id || null,
    hookEventName: payload.hook_event_name || null,
    toolName: payload.tool_name || null,
    cwd: payload.cwd || process.cwd(),
  };
  if (payload.tool_input && typeof payload.tool_input.file_path === "string") {
    entry.filePath = payload.tool_input.file_path;
  }
  fs.appendFileSync(EVENTS_PATH, JSON.stringify(entry) + "\n", "utf8");
}

function readCurrentSession() {
  return readJson(CURRENT_SESSION_PATH, null);
}

function writeCurrentSession(session) {
  ensureDirectory(SESSIONS_DIR);
  fs.writeFileSync(CURRENT_SESSION_PATH, JSON.stringify(session, null, 2) + "\n", "utf8");
}

function updateCurrentSession(patch) {
  const existing = readCurrentSession() || {
    id: patch.id || "session-unknown",
    startedAt: new Date().toISOString(),
    hookEvents: 0,
  };
  const next = {
    ...existing,
    ...patch,
    lastSeenAt: new Date().toISOString(),
    hookEvents: (existing.hookEvents || 0) + 1,
  };
  writeCurrentSession(next);
  return next;
}

function archiveCurrentSession(reason) {
  const session = readCurrentSession();
  if (!session) {
    return;
  }
  ensureDirectory(SESSIONS_DIR);
  const ended = {
    ...session,
    endedAt: new Date().toISOString(),
    endReason: reason || "session-end",
  };
  const archiveName = `${ended.id || "session"}.json`;
  fs.writeFileSync(path.join(SESSIONS_DIR, archiveName), JSON.stringify(ended, null, 2) + "\n", "utf8");
  fs.unlinkSync(CURRENT_SESSION_PATH);
}

function denyPreTool(message) {
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: message,
    },
  }));
}

function sessionStartContext(config) {
  const parts = [
    `profile=${config.profile}`,
    `hooks=${config.features.hooks ? "on" : "off"}`,
    `memory=${config.features.autoMemory ? "on" : "off"}`,
    `teams=${config.features.agentTeams ? "on" : "off"}`,
  ];
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "SessionStart",
      additionalContext: `Claude Flow config loaded from .claude-flow/config.json (${parts.join(", ")}).`,
    },
  }));
}

function dangerousPattern(commandText) {
  const lower = String(commandText || "").toLowerCase();
  const patterns = [
    "rm -rf /",
    "git reset --hard",
    "del /s /q",
    "format c:",
    ":(){:|:&};:",
  ];
  return patterns.find((pattern) => lower.includes(pattern)) || null;
}

function handle(command) {
  const payload = readStdinJson();
  const config = loadConfig();
  if (!config.features.hooks) {
    return;
  }

  switch (command) {
    case "pre-tool-use": {
      const toolName = payload.tool_name || "";
      const toolInput = payload.tool_input || {};
      const commandText = toolInput.command || "";
      const match = toolName === "Bash" ? dangerousPattern(commandText) : null;
      if (match) {
        denyPreTool(`Blocked dangerous Bash pattern: ${match}`);
      }
      break;
    }
    case "post-tool-use":
      appendEvent("post-tool-use", payload);
      updateCurrentSession({ id: payload.session_id || "session-unknown" });
      break;
    case "session-start":
      updateCurrentSession({
        id: payload.session_id || `session-${Date.now()}`,
        transcriptPath: payload.transcript_path || null,
        cwd: PROJECT_ROOT,
        startedAt: new Date().toISOString(),
      });
      appendEvent("session-start", payload);
      sessionStartContext(config);
      break;
    case "stop":
      appendEvent("stop", payload);
      updateCurrentSession({ id: payload.session_id || "session-unknown" });
      break;
    case "session-end":
      appendEvent("session-end", payload);
      archiveCurrentSession(payload.reason || "session-end");
      break;
    case "subagent-stop":
      appendEvent("subagent-stop", payload);
      updateCurrentSession({ id: payload.session_id || "session-unknown" });
      break;
    case "task-completed":
      appendEvent("task-completed", payload);
      updateCurrentSession({ id: payload.session_id || "session-unknown" });
      break;
    case "teammate-idle":
      appendEvent("teammate-idle", payload);
      updateCurrentSession({ id: payload.session_id || "session-unknown" });
      break;
    default:
      appendEvent(`unknown:${command}`, payload);
      break;
  }
}

handle(process.argv[2] || "");
