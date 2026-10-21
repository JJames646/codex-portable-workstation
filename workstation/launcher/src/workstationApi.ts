import { Channel, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import type { EngineId } from "./navigation";
import type { PluginCatalog, PluginInstallResult } from "./types";

export type RuntimePhase =
  | "starting"
  | "running"
  | "stopping"
  | "saving"
  | "stopped"
  | "failed";

export interface EngineState {
  engine: EngineId;
  status: RuntimePhase;
  processPid?: number;
  loopbackPort?: number;
  windowReady: boolean;
  windowProcessId?: number;
  windowHandle?: number;
  windowTitle?: string;
  message?: string;
}

export function engineSupportsStop(engine: EngineId): boolean {
  return engine === "codex" || engine === "openclaw";
}

export function engineCanStop(
  engine: EngineId,
  state: EngineState | undefined,
): boolean {
  if (!engineSupportsStop(engine) || !state) return false;
  if (state.status === "stopping" || state.status === "saving") return false;
  return (
    state.status === "starting" ||
    state.status === "running" ||
    state.processPid !== undefined
  );
}

export function shouldSaveBeforeLaunch(
  engine: EngineId,
  state: EngineState | undefined,
  saved: boolean,
): boolean {
  const reopeningOpenClaw =
    engine === "openclaw" && state?.status === "running";
  return !saved && !reopeningOpenClaw;
}

export function runtimeActionLabel(
  engine: EngineId,
  state: EngineState | undefined,
  launching: boolean,
): string {
  if (state?.status === "saving") return "保存数据中";
  if (launching || state?.status === "starting") {
    return engine === "codex" ? "等待 Codex 窗口" : "启动中";
  }
  if (engine === "codex") {
    if (state?.status === "running" && state.windowReady) return "已启动";
    if (state?.processPid) return "重新唤起";
    return "启动";
  }
  if (engine === "openclaw" && state?.status === "running") {
    return "打开 OpenClaw";
  }
  return state?.status === "running" ? "已运行" : "启动";
}

export function runtimeStatusHint(
  engine: EngineId,
  state: EngineState | undefined,
  saved: boolean,
): string {
  if (!saved) return "请先保存当前供应商配置，再启动智能体。";
  if (state?.status === "saving") {
    return state.message ?? "Codex已关闭，正在把项目、会话与配置保存到移动盘。";
  }
  if (engine === "openclaw" && state?.status === "starting") {
    return state.message ?? "正在启动小龙虾。";
  }
  if (state?.status === "failed") {
    return state.message ?? "启动失败。";
  }
  if (engine === "codex") {
    if (state?.status === "running" && state.windowReady) {
      return "Codex窗口已经出现，工作站会继续保持打开。";
    }
    if (state?.processPid) {
      return state.message ?? "Codex后台进程存在，正在等待可见窗口。";
    }
  }
  if (engine === "openclaw" && state?.status === "running") {
    return "OpenClaw网关正在运行，可重新打开控制台或停止网关。";
  }
  return state?.status === "running"
    ? "智能体正在运行，工作站会继续保持打开。"
    : "当前配置已经保存，可以直接启动。";
}

export interface EngineDetection {
  engine: EngineId;
  ready: boolean;
  executable: string;
  missingPaths: string[];
}

export interface ProviderModels {
  default: string;
  opus: string;
  sonnet: string;
  haiku: string;
  subagent: string;
  available?: string[];
}

export interface ProviderWorkspace {
  chooseOnLaunch: boolean;
  relativePath: string;
}

export interface ProviderView {
  engine: EngineId;
  configured: boolean;
  enabled: boolean;
  displayName?: string;
  baseUrl?: string;
  models?: ProviderModels;
  protocol?: string;
  resolvedProtocol?: string;
  workspace?: ProviderWorkspace;
  credentialPresent: boolean;
  applied: boolean;
  timeoutSeconds: number;
}

export interface BootstrapResponse {
  version: string;
  paths: {
    root: string;
    data: string;
    workspace: string;
    cache: string;
    logs: string;
    backups: string;
  };
  detections: EngineDetection[];
  states: EngineState[];
  providers: ProviderView[];
}

export interface LicenseStatus {
  state: "activated" | "activation_required" | "invalid";
  installationId: string;
  message: string;
}

export interface WorkstationAnnouncement {
  id: string;
  version: string;
  title: string;
  content: string;
  level: "normal" | "important" | "urgent";
  publishedAt: string;
}

export interface CachedWorkstationAnnouncement {
  announcement: WorkstationAnnouncement;
  isRead: boolean;
}

export interface ProviderSavePayload {
  engine: EngineId;
  displayName: string;
  baseUrl: string;
  apiKey?: string;
  protocol: string;
  models: ProviderModels;
  workspace?: ProviderWorkspace;
  timeoutSeconds?: number;
}

export interface SkillCatalogItem {
  id: string;
  reference: string;
  name: string;
  description: string;
  longDescription: string;
  category: string;
  version: string;
  source: string;
  iconUrl?: string;
}

export interface SkillDependencies {
  requirementsTxt: boolean;
  pyprojectToml: boolean;
  packageJson: boolean;
  packageLock: boolean;
  pnpmLock: boolean;
  yarnLock: boolean;
}

export interface LocalSkillDescriptor {
  slug: string;
  name: string;
  description: string;
  version: string;
  relativeRoot: string;
  packageSha256: string;
  dependencies: SkillDependencies;
}

export interface LocalSkillInspection {
  source: string;
  skills: LocalSkillDescriptor[];
}

export interface InstalledSkillItem {
  slug: string;
  name: string;
  description: string;
  version: string;
  target: EngineId;
  source: string;
  verified: boolean;
}

export interface TutorialEntry {
  id: string;
  engine: EngineId;
  title: string;
  fileName: string;
  bytes: number;
}

export interface TutorialDocument {
  entry: TutorialEntry;
  markdown: string;
}

export interface TutorialAsset {
  mediaType: string;
  dataUrl: string;
}

export interface UiPreferences {
  reduceMotion: boolean;
  offlineFirst: boolean;
  compactMode: boolean;
}

export interface CodexProfileLocaleStatus {
  profile: string;
  preferences: string;
  selectedLanguages?: string;
  ready: boolean;
}

export interface CodexLocaleInitializationStatus {
  current: {
    languagePack: string;
    languagePackPresent: boolean;
    configLocale?: string;
    profileLocales: CodexProfileLocaleStatus[];
    nativeI18nVerified: boolean;
    nativeI18nState:
      | "missing"
      | "writtenPendingVerification"
      | "verified"
      | "versionMismatch";
    ready: boolean;
  };
  seed: {
    state: "pending" | "ready" | "incomplete" | "versionMismatch";
    message: string;
    manifestPath: string;
    fileCount: number;
  };
  proxy: CodexProxyStatus;
}

export function codexLocaleStateLabel(
  state: CodexLocaleInitializationStatus["current"]["nativeI18nState"],
): string {
  switch (state) {
    case "writtenPendingVerification":
      return "中文配置已写入，请启动 Codex 验证";
    case "verified":
      return "中文缓存已生效";
    case "versionMismatch":
      return "中文种子与当前 Codex 版本不匹配";
    default:
      return "官方中文功能状态尚未初始化";
  }
}

export type FactorySeedState = "ready" | "missing" | "invalid";

export interface FactorySeedStatus {
  state: FactorySeedState;
  message: string;
  manifestPath: string;
  fileCount: number;
  totalBytes: number;
  runtimeBuildId?: string;
}

export interface FactoryResetPreview {
  previewId: string;
  expiresAtUnixMs: number;
  seed: FactorySeedStatus;
  clean: string[];
  preserve: string[];
  blockers: string[];
  requiredBytes: number;
  availableBytes: number;
  canExecute: boolean;
}

export type FactoryResetState =
  | "idle"
  | "running"
  | "completed"
  | "failed"
  | "rollingBack"
  | "rolledBack";

export type FactoryResetStep =
  | "stopServices"
  | "createRestorePoint"
  | "cleanUserData"
  | "restoreFactoryDefaults"
  | "integrityCheck";

export interface FactoryResetStatus {
  schemaVersion: number;
  operationId?: string;
  state: FactoryResetState;
  currentStep?: FactoryResetStep;
  completedSteps: number;
  totalSteps: number;
  message: string;
  error?: string;
  canRollback: boolean;
  updatedAtUnixMs: number;
}

export type CodexProxyRegion =
  | "all"
  | "hong_kong"
  | "japan"
  | "singapore"
  | "united_states"
  | "other";

export interface CodexProxyNode {
  name: string;
  region: Exclude<CodexProxyRegion, "all">;
  latencyMs?: number;
  available: boolean;
}

export interface CodexProxyStatus {
  configured: boolean;
  subscriptionMasked?: string;
  subscriptionName?: string;
  updatedAtUnixMs?: number;
  nodeCount: number;
  nodes: CodexProxyNode[];
  selectedNode?: string;
  selectedRegion?: Exclude<CodexProxyRegion, "all">;
  persistentEnabled: boolean;
  pendingOnce: boolean;
  active: boolean;
  activeMode?: "once" | "persistent";
  corePid?: number;
  mihomoVersion: string;
}

export interface ChatThreadSummary {
  id: string;
  title: string;
  engine: EngineId;
  providerId: string;
  model: string;
  preview: string;
  createdAt: number;
  updatedAt: number;
}

export interface ChatMessage {
  id: string;
  threadId: string;
  role: "user" | "assistant";
  content: string;
  attachments: ChatAttachment[];
  createdAt: number;
}

export interface ChatAttachment {
  id: string;
  threadId: string;
  messageId?: string | null;
  fileName: string;
  mediaType: string;
  sizeBytes: number;
  createdAt: number;
}

export interface ChatThreadDetail {
  thread: ChatThreadSummary;
  messages: ChatMessage[];
  pendingAttachments: ChatAttachment[];
}

export interface ChatSendResult {
  thread: ChatThreadSummary;
  userMessage: ChatMessage;
  assistantMessage: ChatMessage;
}

export type ChatProviderProtocol =
  | "auto"
  | "responses"
  | "chat-completions"
  | "anthropic-messages";

export interface ChatProviderView {
  id: string;
  displayName: string;
  baseUrl: string;
  defaultModel: string;
  models: string[];
  protocol: ChatProviderProtocol;
  credentialPresent: boolean;
  timeoutSeconds: number;
  enabled: boolean;
}

export interface ChatProviderState {
  configured: boolean;
  activeProviderId?: string;
  providers: ChatProviderView[];
}

export interface ChatProviderSavePayload {
  id?: string;
  displayName: string;
  baseUrl: string;
  apiKey?: string;
  defaultModel: string;
  models: string[];
  protocol: ChatProviderProtocol;
  timeoutSeconds?: number;
}

export interface ImageProviderView {
  id: string;
  displayName: string;
  baseUrl: string;
  defaultModel: string;
  models: string[];
  credentialPresent: boolean;
  timeoutSeconds: number;
  streaming: boolean;
  enabled: boolean;
}

export interface ImageProviderState {
  configured: boolean;
  activeProviderId?: string;
  providers: ImageProviderView[];
}

export interface ImageProviderSavePayload {
  id?: string;
  displayName: string;
  baseUrl: string;
  apiKey?: string;
  defaultModel: string;
  models: string[];
  timeoutSeconds?: number;
  streaming?: boolean;
}

export interface ImageAssetRecord {
  id: string;
  jobId: string;
  relativePath: string;
  thumbnailRelativePath: string;
  mimeType: string;
  fileSize: number;
  width: number;
  height: number;
  revisedPrompt?: string;
  createdAtMs: number;
}

export interface ImageJob {
  id: string;
  providerId: string;
  providerName: string;
  model: string;
  mode: "generation" | "edit";
  prompt: string;
  negativePrompt: string;
  size: string;
  quality: string;
  requestedCount: number;
  status: "queued" | "running" | "completed" | "failed" | "cancelled" | "interrupted";
  errorMessage?: string;
  referenceRelativePath?: string;
  referenceMimeType?: string;
  createdAtMs: number;
  updatedAtMs: number;
  assets: ImageAssetRecord[];
  assetPreviews: Record<string, string>;
}

export interface ImageAsset {
  assetId: string;
  fileName: string;
  mimeType: string;
  dataUrl: string;
}

export interface ImageReference {
  jobId: string;
  fileName: string;
  mimeType: string;
  dataUrl: string;
}

export interface ImageGenerationBootstrap {
  databasePath: string;
  dataDir: string;
  providerState: ImageProviderState;
  jobs: ImageJob[];
}

export type ImageGenerationEvent =
  | { event: "started"; data: { jobId: string } }
  | { event: "progress"; data: { jobId: string; message: string } }
  | { event: "preview"; data: { jobId: string; index: number; imageDataUrl: string } }
  | { event: "completed"; data: { job: ImageJob } }
  | { event: "cancelled"; data: { jobId: string } }
  | { event: "failed"; data: { jobId: string; message: string } };

type InvokeError = {
  code?: string;
  message?: string;
};

export class WorkstationError extends Error {
  code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "WorkstationError";
    this.code = code;
  }
}

export function isTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

async function call<T>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    if (typeof error === "string") {
      throw new WorkstationError("operation_failed", error);
    }
    const candidate = (error ?? {}) as InvokeError;
    throw new WorkstationError(
      candidate.code ?? "operation_failed",
      candidate.message ?? "工作站操作失败",
    );
  }
}

export async function bootstrapWorkstation(): Promise<BootstrapResponse | null> {
  if (!isTauriRuntime()) return null;
  return call<BootstrapResponse>("workstation_bootstrap");
}

export async function loadLicenseStatus(): Promise<LicenseStatus> {
  if (!isTauriRuntime()) {
    return {
      state: "activated",
      installationId: "browser-preview",
      message: "浏览器预览模式",
    };
  }
  return call<LicenseStatus>("license_status");
}

export async function activateLicense(licenseCode: string): Promise<LicenseStatus> {
  return call<LicenseStatus>("license_activate", {
    input: { licenseCode },
  });
}

export async function checkInWorkstation(): Promise<WorkstationAnnouncement | null> {
  if (!isTauriRuntime()) return null;
  return call<WorkstationAnnouncement | null>("license_check_in");
}

export async function loadLatestAnnouncement(): Promise<CachedWorkstationAnnouncement | null> {
  if (!isTauriRuntime()) return null;
  return call<CachedWorkstationAnnouncement | null>("license_latest_announcement");
}

export async function markAnnouncementRead(version: string): Promise<void> {
  if (!isTauriRuntime()) return;
  await call("license_mark_announcement_read", { version });
}

export async function loadProviders(): Promise<ProviderView[]> {
  if (!isTauriRuntime()) return [];
  return call<ProviderView[]>("provider_get_all");
}

export async function saveProvider(
  payload: ProviderSavePayload,
): Promise<void> {
  await call("provider_save", { input: payload });
}

export async function discoverModels(payload: {
  engine: EngineId;
  baseUrl: string;
  apiKey?: string;
  protocol: string;
}): Promise<string[]> {
  if (!isTauriRuntime()) {
    return {
      codex: ["gpt-5.2-codex", "gpt-5.1-codex-max"],
      claude: ["claude-opus-4-8", "claude-sonnet-5", "claude-haiku-4-5"],
      openclaw: ["gpt-5.2", "claude-sonnet-4-5"],
    }[payload.engine];
  }
  const result = await call<{ models: string[] }>("provider_discover_models", {
    input: payload,
  });
  return result.models;
}

export async function testProvider(engine: EngineId): Promise<void> {
  await call("provider_test", { input: { engine } });
}

export async function startEngine(engine: EngineId): Promise<EngineState> {
  return call<EngineState>("engine_start", { input: { engine } });
}

export async function openOpenClawControl(): Promise<EngineState> {
  return call<EngineState>("openclaw_open_control");
}

export async function stopEngine(engine: EngineId): Promise<EngineState> {
  return call<EngineState>("engine_stop", { input: { engine } });
}

export async function loadEngineStates(): Promise<EngineState[]> {
  if (!isTauriRuntime()) return [];
  return call<EngineState[]>("engine_status_all");
}

export async function loadCodexLocaleStatus(): Promise<CodexLocaleInitializationStatus> {
  if (!isTauriRuntime()) {
    return {
      current: {
        languagePack: "codex/app/locales/zh-CN.pak",
        languagePackPresent: true,
        profileLocales: [],
        nativeI18nVerified: true,
        nativeI18nState: "verified",
        ready: true,
      },
      seed: {
        state: "pending",
        message: "中文缓存待固化",
        manifestPath: "Factory/Seeds/Codex/manifest.json",
        fileCount: 0,
      },
      proxy: {
        configured: false,
        nodeCount: 0,
        nodes: [],
        persistentEnabled: false,
        pendingOnce: false,
        active: false,
        mihomoVersion: "1.19.29",
      },
    };
  }
  return call<CodexLocaleInitializationStatus>("codex_locale_status");
}

export async function applyCodexLocaleSeed(): Promise<CodexLocaleInitializationStatus> {
  const result = await call<{ status: CodexLocaleInitializationStatus }>(
    "codex_locale_apply_seed",
  );
  return result.status;
}

export async function loadCodexProxyStatus(): Promise<CodexProxyStatus> {
  if (!isTauriRuntime()) {
    return {
      configured: false,
      nodeCount: 0,
      nodes: [],
      persistentEnabled: false,
      pendingOnce: false,
      active: false,
      mihomoVersion: "1.19.29",
    };
  }
  return call<CodexProxyStatus>("codex_proxy_status");
}

export async function scanCodexProxySubscription(
  subscriptionUrl?: string,
): Promise<CodexProxyStatus> {
  return call<CodexProxyStatus>("codex_proxy_subscription_scan", {
    input: { subscriptionUrl },
  });
}

export async function selectCodexProxyNode(
  nodeName: string,
): Promise<CodexProxyStatus> {
  return call<CodexProxyStatus>("codex_proxy_node_select", {
    input: { nodeName },
  });
}

export async function startCodexWithInitProxy(): Promise<EngineState> {
  return call<EngineState>("codex_proxy_start_once");
}

export async function enablePersistentCodexProxy(): Promise<CodexProxyStatus> {
  return call<CodexProxyStatus>("codex_proxy_enable_persistent");
}

export async function disableCodexProxy(): Promise<CodexProxyStatus> {
  return call<CodexProxyStatus>("codex_proxy_disable");
}

export async function clearCodexProxy(): Promise<CodexProxyStatus> {
  return call<CodexProxyStatus>("codex_proxy_clear");
}

export async function verifyFactorySeed(): Promise<FactorySeedStatus> {
  if (!isTauriRuntime()) {
    return {
      state: "missing",
      message: "恢复资源未就绪",
      manifestPath: "Factory/manifest.json",
      fileCount: 0,
      totalBytes: 0,
    };
  }
  return call<FactorySeedStatus>("factory_seed_verify");
}

export async function previewFactoryReset(): Promise<FactoryResetPreview> {
  return call<FactoryResetPreview>("factory_reset_preview");
}

export async function executeFactoryReset(
  previewId: string,
): Promise<FactoryResetStatus> {
  return call<FactoryResetStatus>("factory_reset_execute", {
    input: { previewId, acknowledged: true },
  });
}

export async function loadFactoryResetStatus(): Promise<FactoryResetStatus> {
  if (!isTauriRuntime()) {
    return {
      schemaVersion: 1,
      state: "idle",
      completedSteps: 0,
      totalSteps: 5,
      message: "尚未执行恢复出厂设置",
      canRollback: false,
      updatedAtUnixMs: Date.now(),
    };
  }
  return call<FactoryResetStatus>("factory_reset_status");
}

export async function rollbackFactoryReset(
  operationId: string,
): Promise<FactoryResetStatus> {
  return call<FactoryResetStatus>("factory_reset_rollback", {
    input: { operationId },
  });
}

export async function choosePortableWorkspace(
  root: string,
): Promise<string | null> {
  if (!isTauriRuntime()) return "workspace/Claude Code";
  const selection = await open({
    directory: true,
    multiple: false,
    defaultPath: root,
    title: "选择工作站内的 Claude Code 工作目录",
  });
  if (!selection || Array.isArray(selection)) return null;
  const normalizedRoot = root.replaceAll("\\", "/").replace(/\/+$/, "");
  const normalizedSelection = selection.replaceAll("\\", "/");
  if (
    normalizedSelection !== normalizedRoot &&
    !normalizedSelection.startsWith(`${normalizedRoot}/`)
  ) {
    throw new WorkstationError(
      "path_not_portable",
      "工作目录必须位于当前移动工作站内",
    );
  }
  return normalizedSelection === normalizedRoot
    ? "workspace"
    : normalizedSelection.slice(normalizedRoot.length + 1);
}

export async function chooseLocalSkillFile(
  kind: "zip" | "markdown",
): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  const selection = await open({
    directory: false,
    multiple: false,
    title: kind === "zip" ? "选择 Skill ZIP 安装包" : "选择 Skill Markdown 文件",
    filters: [
      kind === "zip"
        ? { name: "Skill ZIP 安装包", extensions: ["zip"] }
        : { name: "Skill Markdown", extensions: ["md"] },
    ],
  });
  return !selection || Array.isArray(selection) ? null : selection;
}

export async function searchSkills(input: {
  query: string;
  category: string;
  page?: number;
  offline?: boolean;
}): Promise<SkillCatalogItem[]> {
  if (!isTauriRuntime()) return [];
  const result = await call<{ data: unknown }>("skills_search", {
    input: {
      query: input.query || undefined,
      category: input.category === "全部" ? undefined : input.category,
      page: input.page ?? 1,
      pageSize: 60,
      sortBy: "downloads",
      offline: input.offline ?? false,
    },
  });
  return normalizeSkillItems(result.data);
}

export async function loadRecommendedSkills(
  offline = false,
): Promise<SkillCatalogItem[]> {
  if (!isTauriRuntime()) return [];
  const result = await call<{ data: unknown }>("skills_recommended", { offline });
  return normalizeSkillItems(result.data);
}

export async function loadInstalledSkills(): Promise<InstalledSkillItem[]> {
  if (!isTauriRuntime()) return [];
  const result = await call<
    Array<{
      record: {
        slug: string;
        name: string;
        description?: string;
        version: string;
        target: EngineId;
        source: string;
      };
      verified: boolean;
    }>
  >("skills_list_installed");
  return result.map((item) => ({
    slug: item.record.slug,
    name: item.record.name,
    description: item.record.description ?? "",
    version: item.record.version,
    target: item.record.target,
    source: item.record.source,
    verified: item.verified,
  }));
}

export async function inspectLocalSkill(
  source: string,
): Promise<LocalSkillInspection> {
  return call<LocalSkillInspection>("skills_inspect_local", {
    input: { source },
  });
}

export async function installSkill(
  reference: string,
  target: EngineId,
  options: { installDependencies?: boolean; offline?: boolean } = {},
): Promise<void> {
  await call("skills_install", {
    input: {
      reference,
      target,
      installDependencies: options.installDependencies ?? false,
      offline: options.offline ?? false,
    },
  });
}

export async function loadPluginCatalog(
  query = "",
  category = "",
): Promise<PluginCatalog> {
  if (!isTauriRuntime()) {
    throw new WorkstationError("preview_mode", "浏览器预览不读取移动盘插件");
  }
  return call<PluginCatalog>("plugins_list", {
    input: {
      query: query || undefined,
      category: category === "全部" ? undefined : category,
      page: 1,
      pageSize: 500,
    },
  });
}

export async function installCodexPlugin(
  id: string,
): Promise<PluginInstallResult> {
  return call<PluginInstallResult>("plugins_install", { input: { id } });
}

export async function installAllCodexPlugins(ids: string[]): Promise<void> {
  const results = await call<Array<{ id: string; success: boolean; error?: string }>>(
    "plugins_install_batch",
    { input: { ids } },
  );
  const failed = results.filter((item) => !item.success);
  if (failed.length) {
    throw new WorkstationError(
      "plugin_batch_partial_failure",
      `${results.length - failed.length} 个成功，${failed.length} 个失败`,
    );
  }
}

export async function loadTutorials(
  engine: EngineId,
): Promise<TutorialEntry[]> {
  if (!isTauriRuntime()) return [];
  return call<TutorialEntry[]>("tutorial_list", { input: { engine } });
}

export async function readTutorial(
  engine: EngineId,
  fileName: string,
): Promise<TutorialDocument> {
  return call<TutorialDocument>("tutorial_read", {
    input: { engine, fileName },
  });
}

export async function readTutorialAsset(
  engine: EngineId,
  assetPath: string,
): Promise<TutorialAsset> {
  return call<TutorialAsset>("tutorial_asset_read", {
    input: { engine, assetPath },
  });
}

export async function openExternalUrl(url: string): Promise<void> {
  if (!isTauriRuntime()) {
    window.open(url, "_blank", "noopener,noreferrer");
    return;
  }
  await call("open_external_url", { url });
}

export async function runDiagnosis(): Promise<unknown> {
  return call("workstation_diagnose");
}

export async function openLogs(): Promise<void> {
  await call("logs_open");
}

export async function openBackups(): Promise<void> {
  await call("backups_open");
}

export async function loadUiPreferences(): Promise<UiPreferences> {
  if (!isTauriRuntime()) {
    return { reduceMotion: false, offlineFirst: false, compactMode: false };
  }
  return call<UiPreferences>("ui_preferences_load");
}

export async function saveUiPreferences(
  preferences: UiPreferences,
): Promise<UiPreferences> {
  return call<UiPreferences>("ui_preferences_save", { preferences });
}

export async function initializeChat(): Promise<void> {
  if (!isTauriRuntime()) return;
  await call("chat_initialize");
}

export async function listChatThreads(
  engine: EngineId,
): Promise<ChatThreadSummary[]> {
  if (!isTauriRuntime()) return [];
  return call<ChatThreadSummary[]>("chat_list_threads", {
    input: { engine },
  });
}

export async function createChatThread(
  engine: EngineId,
): Promise<ChatThreadDetail> {
  return call<ChatThreadDetail>("chat_create_thread", {
    input: { engine },
  });
}

export async function loadChatThread(
  threadId: string,
): Promise<ChatThreadDetail> {
  return call<ChatThreadDetail>("chat_load_thread", {
    input: { threadId },
  });
}

export async function sendChatMessage(
  threadId: string,
  content: string,
  attachmentIds: string[] = [],
): Promise<ChatSendResult> {
  return call<ChatSendResult>("chat_send", {
    input: { threadId, content, attachmentIds },
  });
}

export async function chooseChatAttachmentFiles(): Promise<string[]> {
  const selected = await open({
    multiple: true,
    directory: false,
    filters: [
      {
        name: "AI 对话附件",
        extensions: ["png", "jpg", "jpeg", "webp", "txt", "md", "csv", "json"],
      },
    ],
  });
  if (!selected) return [];
  return Array.isArray(selected) ? selected : [selected];
}

export async function importChatAttachments(
  threadId: string,
  paths: string[],
): Promise<ChatAttachment[]> {
  return call<ChatAttachment[]>("chat_import_attachments", {
    input: { threadId, paths },
  });
}

export async function removeChatAttachment(
  attachmentId: string,
): Promise<void> {
  await call("chat_remove_attachment", { input: { attachmentId } });
}

export async function setChatThreadProvider(input: {
  threadId: string;
  providerId: string;
  model: string;
}): Promise<ChatThreadDetail> {
  return call<ChatThreadDetail>("chat_set_thread_provider", { input });
}

export async function loadChatProviders(): Promise<ChatProviderState> {
  if (!isTauriRuntime()) {
    return { configured: false, providers: [] };
  }
  return call<ChatProviderState>("chat_provider_list");
}

export async function saveChatProvider(
  input: ChatProviderSavePayload,
): Promise<ChatProviderState> {
  return call<ChatProviderState>("chat_provider_save", { input });
}

export async function setActiveChatProvider(
  providerId: string,
): Promise<ChatProviderState> {
  return call<ChatProviderState>("chat_provider_set_active", {
    input: { providerId },
  });
}

export async function discoverChatProviderModels(input: {
  providerId?: string;
  baseUrl: string;
  apiKey?: string;
  protocol: ChatProviderProtocol;
  timeoutSeconds?: number;
}): Promise<{ models: string[]; elapsedMs: number }> {
  return call("chat_provider_discover_models", { input });
}

export async function testChatProvider(providerId: string): Promise<void> {
  await call("chat_provider_test", { input: { providerId } });
}

export async function deleteChatThread(threadId: string): Promise<void> {
  await call("chat_delete_thread", { input: { threadId } });
}

export async function renameChatThread(
  threadId: string,
  title: string,
): Promise<ChatThreadSummary> {
  return call<ChatThreadSummary>("chat_rename_thread", {
    input: { threadId, title },
  });
}

export async function exportChatThread(threadId: string): Promise<string> {
  return call<string>("chat_export_thread", { input: { threadId } });
}

export async function initializeImageStudio(): Promise<ImageGenerationBootstrap> {
  if (!isTauriRuntime()) {
    return {
      databasePath: "",
      dataDir: "",
      providerState: { configured: false, providers: [] },
      jobs: [],
    };
  }
  return call<ImageGenerationBootstrap>("imagegen_initialize");
}

export async function loadImageProviders(): Promise<ImageProviderState> {
  if (!isTauriRuntime()) return { configured: false, providers: [] };
  return call<ImageProviderState>("image_provider_list");
}

export async function saveImageProvider(
  input: ImageProviderSavePayload,
): Promise<ImageProviderState> {
  return call<ImageProviderState>("image_provider_save", { input });
}

export async function setActiveImageProvider(
  providerId: string,
): Promise<ImageProviderState> {
  return call<ImageProviderState>("image_provider_set_active", {
    input: { providerId },
  });
}

export async function discoverImageProviderModels(input: {
  providerId?: string;
  baseUrl: string;
  apiKey?: string;
  timeoutSeconds?: number;
}): Promise<{ models: string[]; elapsedMs: number }> {
  return call("image_provider_discover_models", { input });
}

export async function testImageProvider(providerId: string): Promise<{
  success: boolean;
  statusCode?: number;
  message: string;
  elapsedMs: number;
}> {
  return call("image_provider_test", { input: { providerId } });
}

export async function generateImage(
  input: {
    providerId: string;
    mode: "generation" | "edit";
    prompt: string;
    negativePrompt: string;
    size: string;
    quality: string;
    count: number;
    referenceDataUrl?: string;
  },
  onEvent: (event: ImageGenerationEvent) => void,
): Promise<ImageJob> {
  const channel = new Channel<ImageGenerationEvent>();
  channel.onmessage = onEvent;
  return call<ImageJob>("imagegen_generate", { input, onEvent: channel });
}

export async function cancelImageGeneration(jobId: string): Promise<boolean> {
  return call<boolean>("imagegen_cancel", { jobId });
}

export async function listImageHistory(limit = 80): Promise<ImageJob[]> {
  if (!isTauriRuntime()) return [];
  return call<ImageJob[]>("imagegen_list_history", { input: { limit } });
}

export async function readImageAsset(assetId: string): Promise<ImageAsset> {
  return call<ImageAsset>("imagegen_read_asset", { input: { assetId } });
}

export async function readImageReference(jobId: string): Promise<ImageReference | null> {
  return call<ImageReference | null>("imagegen_read_reference", {
    input: { jobId },
  });
}

export async function deleteImageJob(jobId: string): Promise<boolean> {
  return call<boolean>("imagegen_delete_job", { input: { jobId } });
}

export async function clearImageHistory(): Promise<void> {
  await call("imagegen_clear_history");
}

export async function exportImageAsset(assetId: string): Promise<string> {
  return call<string>("imagegen_export_asset", { input: { assetId } });
}

export async function openImageOutputDirectory(): Promise<void> {
  await call("imagegen_open_output_directory");
}

export function normalizeSkillItems(data: unknown): SkillCatalogItem[] {
  const root = asRecord(data);
  const candidate =
    asArray(root.items) ??
    asArray(root.skills) ??
    asArray(asRecord(root.data).items) ??
    asArray(asRecord(root.data).skills) ??
    [];
  return candidate
    .map(asRecord)
    .map((item) => {
      const slug = text(item.slug) || text(item.id);
      const resolvedVersion =
        safeSkillHubCoordinatePart(text(item.version)) ||
        safeSkillHubCoordinatePart(text(asRecord(item.latestVersion).version));
      const version = resolvedVersion || "latest";
      const namespace = resolveSkillHubNamespace(item, slug);
      const suppliedReference = text(item.reference);
      const reference = isCanonicalSkillHubReference(suppliedReference)
        ? suppliedReference
        : namespace && slug
          ? `skillhub:@${namespace}/${slug}${resolvedVersion ? `@${resolvedVersion}` : ""}`
          : "";
      return {
        id: slug || reference,
        reference,
        name:
          text(item.displayName) ||
          text(item.name) ||
          text(item.title) ||
          slug ||
          "未命名 Skill",
        description:
          text(item.description_zh) ||
          text(item.descriptionZh) ||
          text(item.summary_zh) ||
          text(item.summaryZh) ||
          text(item.description) ||
          text(item.summary) ||
          "暂无中文简介",
        longDescription:
          text(item.description_zh) ||
          text(item.descriptionZh) ||
          text(item.description) ||
          text(item.summary) ||
          "暂无详细说明",
        category: text(item.category) || "其他",
        version,
        source: "SkillHub",
        iconUrl: text(item.iconUrl) || text(item.icon),
      };
    })
    .filter((item) => item.id && item.reference);
}

function resolveSkillHubNamespace(
  item: Record<string, unknown>,
  slug: string,
): string {
  const namespace = item.namespace;
  if (typeof namespace === "string") {
    return safeSkillHubNamespace(namespace);
  }

  const namespaceRecord = asRecord(namespace);
  const canonicalNamespace = namespaceFromCanonicalName(
    text(namespaceRecord.canonicalName),
    slug,
  );
  if (canonicalNamespace) return canonicalNamespace;

  const candidates = [
    text(namespaceRecord.handle),
    text(namespaceRecord.displayName),
    text(asRecord(item.owner).handle),
    text(asRecord(item.owner).slug),
    text(asRecord(item.publisher).orgId),
  ];
  for (const candidate of candidates) {
    const safeNamespace = safeSkillHubNamespace(candidate);
    if (safeNamespace) return safeNamespace;
  }
  return "";
}

function namespaceFromCanonicalName(
  canonicalName: string,
  expectedSlug: string,
): string {
  const value = canonicalName.startsWith("@")
    ? canonicalName.slice(1)
    : canonicalName;
  const separator = value.indexOf("/");
  if (separator < 1) return "";
  const namespace = safeSkillHubNamespace(value.slice(0, separator));
  const canonicalSlug = safeSkillHubCoordinatePart(value.slice(separator + 1));
  if (!namespace || !canonicalSlug || (expectedSlug && canonicalSlug !== expectedSlug)) {
    return "";
  }
  return namespace;
}

function safeSkillHubNamespace(value: string): string {
  const normalized = value.startsWith("@") ? value.slice(1) : value;
  return safeSkillHubCoordinatePart(normalized);
}

function safeSkillHubCoordinatePart(value: string): string {
  return value.length <= 160 && /^[A-Za-z0-9._-]+$/.test(value) ? value : "";
}

function isCanonicalSkillHubReference(value: string): boolean {
  return /^skillhub:@[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+(?:@[A-Za-z0-9._-]+)?$/.test(
    value,
  );
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function asArray(value: unknown): unknown[] | null {
  return Array.isArray(value) ? value : null;
}

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}
