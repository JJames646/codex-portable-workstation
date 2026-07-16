import { invoke } from "@tauri-apps/api/core";

export type WorkstationCommandStatus = "ok" | "partial" | "unavailable";
export type BusinessServiceStatus = "not_connected";

export type BusinessServiceAdapter = {
  id: "relay-service" | "workstation-control";
  label: string;
  status: BusinessServiceStatus;
  endpointConfigured: false;
  message: string;
};

export type WorkstationOverview = {
  status: string;
  message: string;
  codex_app: {
    status: string;
    path: string | null;
  };
  codex_version: string | null;
  latest_launch: {
    status: string;
    message: string;
    started_at_ms: number;
    codex_app: string | null;
  } | null;
  current_version: string;
  update_status: string;
  settings_path: string;
  logs_path: string;
};

export type WorkstationSettings = {
  status: string;
  message: string;
  settings_path: string;
  settings: {
    codexAppPath: string;
    activeRelayId: string;
    relayProfiles: Array<{
      id: string;
      name: string;
      model: string;
      baseUrl: string;
    }>;
  };
  user_scripts: {
    scripts?: Array<{
      key: string;
      name: string;
      enabled: boolean;
    }>;
  };
};

export type WorkstationSession = {
  id: string;
  title: string;
  cwd: string;
  modelProvider: string;
  archived: boolean;
  updatedAtMs: number | null;
  rolloutPath: string;
  dbPath: string;
};

type LocalSessionsResult = {
  status: string;
  message: string;
  sessions: WorkstationSession[];
};

type LiveContextEntriesResult = {
  status: string;
  message: string;
  entries: {
    mcpServers: unknown[];
    skills: unknown[];
    plugins: unknown[];
  };
};

type ZedRemoteProjectsResult = {
  status: string;
  message: string;
  projects: unknown[];
};

export type RelayEnvironmentSnapshot = {
  status: string;
  message: string;
  clashVergeTun: {
    enabled: boolean;
    configPath: string | null;
  };
  proxyEnvironment: {
    variables: Array<{
      name: string;
      source: string;
    }>;
  };
  codexEnvFile: {
    exists: boolean;
    path: string;
  };
};

export type ProjectSummary = {
  cwd: string;
  name: string;
  sessionCount: number;
  latestActivityMs: number | null;
  providers: string[];
};

export type WorkstationSnapshot = {
  status: WorkstationCommandStatus;
  loadedAtMs: number | null;
  overview: WorkstationOverview | null;
  settings: WorkstationSettings | null;
  sessions: WorkstationSession[];
  contextCounts: {
    mcpServers: number;
    skills: number;
    plugins: number;
  };
  zedProjectCount: number;
  relayEnvironment: RelayEnvironmentSnapshot | null;
  services: {
    relay: BusinessServiceAdapter;
    control: BusinessServiceAdapter;
  };
  errors: string[];
};

type InvokeCommand = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

const commandLabels = [
  "Codex++ 概览",
  "Codex++ 设置",
  "本地会话",
  "工具与插件",
  "Zed 远程项目",
  "网络环境",
] as const;

export function createBusinessServiceAdapters(): WorkstationSnapshot["services"] {
  return {
    relay: {
      id: "relay-service",
      label: "中转站数据服务",
      status: "not_connected",
      endpointConfigured: false,
      message: "阶段 B 仅保留接口，尚未配置中转站服务器地址。",
    },
    control: {
      id: "workstation-control",
      label: "工作站控制服务",
      status: "not_connected",
      endpointConfigured: false,
      message: "阶段 B 仅保留接口，卡密、服务连接和管理员后台尚未接入。",
    },
  };
}

export function createInitialWorkstationSnapshot(): WorkstationSnapshot {
  return {
    status: "unavailable",
    loadedAtMs: null,
    overview: null,
    settings: null,
    sessions: [],
    contextCounts: {
      mcpServers: 0,
      skills: 0,
      plugins: 0,
    },
    zedProjectCount: 0,
    relayEnvironment: null,
    services: createBusinessServiceAdapters(),
    errors: [],
  };
}

export async function loadWorkstationSnapshot(
  invokeCommand: InvokeCommand = invoke,
): Promise<WorkstationSnapshot> {
  if (invokeCommand === invoke && !hasTauriRuntime()) {
    return {
      ...createInitialWorkstationSnapshot(),
      errors: ["当前是浏览器预览，真实本地数据仅在 Tauri 客户端中读取。"],
    };
  }

  const results = await Promise.allSettled([
    invokeCommand<WorkstationOverview>("load_overview"),
    invokeCommand<WorkstationSettings>("load_settings"),
    invokeCommand<LocalSessionsResult>("list_local_sessions", {
      request: { offset: 0, limit: 100 },
    }),
    invokeCommand<LiveContextEntriesResult>("read_live_context_entries"),
    invokeCommand<ZedRemoteProjectsResult>("list_zed_remote_projects"),
    invokeCommand<RelayEnvironmentSnapshot>("check_relay_environment"),
  ]);

  const errors = results.flatMap((result, index) =>
    commandResultErrors(result, commandLabels[index]),
  );
  const fulfilledCount = results.length - errors.length;
  const overview = settledValue<WorkstationOverview>(results[0]);
  const settings = settledValue<WorkstationSettings>(results[1]);
  const localSessions = settledValue<LocalSessionsResult>(results[2]);
  const context = settledValue<LiveContextEntriesResult>(results[3]);
  const zed = settledValue<ZedRemoteProjectsResult>(results[4]);
  const relayEnvironment = settledValue<RelayEnvironmentSnapshot>(results[5]);

  return {
    status:
      fulfilledCount === results.length
        ? "ok"
        : fulfilledCount > 0
          ? "partial"
          : "unavailable",
    loadedAtMs: Date.now(),
    overview,
    settings,
    sessions: localSessions?.sessions ?? [],
    contextCounts: {
      mcpServers: context?.entries.mcpServers.length ?? 0,
      skills: context?.entries.skills.length ?? 0,
      plugins: context?.entries.plugins.length ?? 0,
    },
    zedProjectCount: zed?.projects.length ?? 0,
    relayEnvironment,
    services: createBusinessServiceAdapters(),
    errors,
  };
}

export function groupSessionsByProject(
  sessions: WorkstationSession[],
): ProjectSummary[] {
  const projects = new Map<string, ProjectSummary>();
  for (const session of sessions) {
    const cwd = session.cwd.trim();
    if (!cwd) continue;
    const existing = projects.get(cwd);
    const provider = session.modelProvider.trim();
    if (existing) {
      existing.sessionCount += 1;
      existing.latestActivityMs = maxNullableTime(
        existing.latestActivityMs,
        session.updatedAtMs,
      );
      if (provider && !existing.providers.includes(provider)) {
        existing.providers.push(provider);
      }
      continue;
    }
    projects.set(cwd, {
      cwd,
      name: projectNameFromPath(cwd),
      sessionCount: 1,
      latestActivityMs: session.updatedAtMs,
      providers: provider ? [provider] : [],
    });
  }
  return Array.from(projects.values()).sort(
    (left, right) =>
      (right.latestActivityMs ?? 0) - (left.latestActivityMs ?? 0),
  );
}

function hasTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    "__TAURI_INTERNALS__" in (window as unknown as Record<string, unknown>)
  );
}

function settledValue<T>(
  result: PromiseSettledResult<unknown>,
): T | null {
  return result.status === "fulfilled" ? (result.value as T) : null;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function commandResultErrors(
  result: PromiseSettledResult<unknown>,
  label: string,
): string[] {
  if (result.status === "rejected") {
    return [`${label}：${errorMessage(result.reason)}`];
  }
  if (!isRecord(result.value)) return [];
  const status =
    typeof result.value.status === "string"
      ? result.value.status.toLowerCase()
      : "";
  if (!["failed", "error", "unavailable", "not_implemented"].includes(status)) {
    return [];
  }
  const message =
    typeof result.value.message === "string" && result.value.message.trim()
      ? result.value.message
      : `命令返回 ${status}`;
  return [`${label}：${message}`];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function projectNameFromPath(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) || path;
}

function maxNullableTime(
  left: number | null,
  right: number | null,
): number | null {
  if (left === null) return right;
  if (right === null) return left;
  return Math.max(left, right);
}
