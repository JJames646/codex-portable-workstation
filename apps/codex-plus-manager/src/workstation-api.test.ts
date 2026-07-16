import assert from "node:assert/strict";
import test from "node:test";

import {
  createBusinessServiceAdapters,
  groupSessionsByProject,
  loadWorkstationSnapshot,
  type WorkstationSession,
} from "./lib/workstation-api.ts";

test("business server adapters stay explicitly not connected", () => {
  const services = createBusinessServiceAdapters();

  assert.equal(services.relay.status, "not_connected");
  assert.equal(services.control.status, "not_connected");
  assert.equal(services.relay.endpointConfigured, false);
  assert.equal(services.control.endpointConfigured, false);
});

test("workstation snapshot keeps local data when one command fails", async () => {
  const invoke = async <T>(command: string): Promise<T> => {
    if (command === "load_overview") {
      return {
        status: "ok",
        message: "ok",
        codex_app: { status: "installed", path: "C:\\Codex" },
        codex_version: "1.0.0",
        latest_launch: null,
        current_version: "1.2.37",
        update_status: "not_checked",
        settings_path: "C:\\settings.json",
        logs_path: "C:\\diagnostic.log",
      } as T;
    }
    if (command === "load_settings") {
      return {
        status: "ok",
        message: "ok",
        settings_path: "C:\\settings.json",
        settings: {
          codexAppPath: "C:\\Codex",
          activeRelayId: "default",
          relayProfiles: [],
        },
        user_scripts: { scripts: [] },
      } as T;
    }
    if (command === "list_local_sessions") {
      return {
        status: "ok",
        message: "ok",
        sessions: [],
      } as T;
    }
    if (command === "read_live_context_entries") {
      return {
        status: "ok",
        message: "ok",
        entries: {
          mcpServers: [{}],
          skills: [{}, {}],
          plugins: [],
        },
      } as T;
    }
    if (command === "list_zed_remote_projects") {
      throw new Error("not configured");
    }
    return {
      status: "ok",
      message: "ok",
      clashVergeTun: { enabled: false, configPath: null },
      proxyEnvironment: { variables: [] },
      codexEnvFile: { exists: false, path: "C:\\.codex\\.env" },
    } as T;
  };

  const snapshot = await loadWorkstationSnapshot(invoke);

  assert.equal(snapshot.status, "partial");
  assert.equal(snapshot.contextCounts.mcpServers, 1);
  assert.equal(snapshot.contextCounts.skills, 2);
  assert.equal(snapshot.zedProjectCount, 0);
  assert.match(snapshot.errors[0], /Zed 远程项目/);
});

test("workstation snapshot treats structured command failures as partial", async () => {
  const invoke = async <T>(command: string): Promise<T> => {
    if (command === "load_settings") {
      return {
        status: "failed",
        message: "settings unavailable",
        settings_path: "",
        settings: {
          codexAppPath: "",
          activeRelayId: "",
          relayProfiles: [],
        },
        user_scripts: { scripts: [] },
      } as T;
    }
    if (command === "load_overview") {
      return {
        status: "ok",
        message: "ok",
        codex_app: { status: "missing", path: null },
        codex_version: null,
        latest_launch: null,
        current_version: "1.2.37",
        update_status: "not_checked",
        settings_path: "",
        logs_path: "",
      } as T;
    }
    if (command === "list_local_sessions") {
      return { status: "ok", message: "ok", sessions: [] } as T;
    }
    if (command === "read_live_context_entries") {
      return {
        status: "ok",
        message: "ok",
        entries: { mcpServers: [], skills: [], plugins: [] },
      } as T;
    }
    if (command === "list_zed_remote_projects") {
      return { status: "ok", message: "ok", projects: [] } as T;
    }
    return {
      status: "ok",
      message: "ok",
      clashVergeTun: { enabled: false, configPath: null },
      proxyEnvironment: { variables: [] },
      codexEnvFile: { exists: false, path: "" },
    } as T;
  };

  const snapshot = await loadWorkstationSnapshot(invoke);

  assert.equal(snapshot.status, "partial");
  assert.match(snapshot.errors[0], /Codex\+\+ 设置：settings unavailable/);
});

test("sessions are grouped into projects by real cwd", () => {
  const sessions: WorkstationSession[] = [
    session("one", "D:\\Workspace\\alpha", 100, "openai"),
    session("two", "D:\\Workspace\\alpha", 200, "deepseek"),
    session("three", "D:\\Workspace\\beta", 150, "openai"),
  ];

  const projects = groupSessionsByProject(sessions);

  assert.equal(projects.length, 2);
  assert.equal(projects[0].name, "alpha");
  assert.equal(projects[0].sessionCount, 2);
  assert.deepEqual(projects[0].providers, ["openai", "deepseek"]);
});

function session(
  id: string,
  cwd: string,
  updatedAtMs: number,
  modelProvider: string,
): WorkstationSession {
  return {
    id,
    title: id,
    cwd,
    modelProvider,
    archived: false,
    updatedAtMs,
    rolloutPath: "",
    dbPath: "",
  };
}
