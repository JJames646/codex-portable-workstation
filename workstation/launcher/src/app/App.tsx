import { ArrowSquareOut } from "@phosphor-icons/react/ArrowSquareOut";
import { ArrowsClockwise } from "@phosphor-icons/react/ArrowsClockwise";
import { Books } from "@phosphor-icons/react/Books";
import { CaretDown } from "@phosphor-icons/react/CaretDown";
import { CaretLeft } from "@phosphor-icons/react/CaretLeft";
import { CaretRight } from "@phosphor-icons/react/CaretRight";
import { ChatCircleDots } from "@phosphor-icons/react/ChatCircleDots";
import { Check } from "@phosphor-icons/react/Check";
import { CheckCircle } from "@phosphor-icons/react/CheckCircle";
import { Cloud } from "@phosphor-icons/react/Cloud";
import { Database } from "@phosphor-icons/react/Database";
import { DownloadSimple } from "@phosphor-icons/react/DownloadSimple";
import { Eye } from "@phosphor-icons/react/Eye";
import { EyeSlash } from "@phosphor-icons/react/EyeSlash";
import { FileMd } from "@phosphor-icons/react/FileMd";
import { FileZip } from "@phosphor-icons/react/FileZip";
import { FolderOpen } from "@phosphor-icons/react/FolderOpen";
import { GearSix } from "@phosphor-icons/react/GearSix";
import { GraduationCap } from "@phosphor-icons/react/GraduationCap";
import { House } from "@phosphor-icons/react/House";
import { ImageSquare } from "@phosphor-icons/react/ImageSquare";
import { Info } from "@phosphor-icons/react/Info";
import { Key } from "@phosphor-icons/react/Key";
import { Lightning } from "@phosphor-icons/react/Lightning";
import { LinkSimple } from "@phosphor-icons/react/LinkSimple";
import { MagnifyingGlass } from "@phosphor-icons/react/MagnifyingGlass";
import { Megaphone } from "@phosphor-icons/react/Megaphone";
import { PaperPlaneTilt } from "@phosphor-icons/react/PaperPlaneTilt";
import { PencilSimple } from "@phosphor-icons/react/PencilSimple";
import { Play } from "@phosphor-icons/react/Play";
import { Plus } from "@phosphor-icons/react/Plus";
import { PuzzlePiece } from "@phosphor-icons/react/PuzzlePiece";
import { ShieldCheck } from "@phosphor-icons/react/ShieldCheck";
import { Sparkle } from "@phosphor-icons/react/Sparkle";
import { SpinnerGap } from "@phosphor-icons/react/SpinnerGap";
import { Stop } from "@phosphor-icons/react/Stop";
import { Trash } from "@phosphor-icons/react/Trash";
import { Wrench } from "@phosphor-icons/react/Wrench";
import { X } from "@phosphor-icons/react/X";
import {
  lazy,
  Suspense,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type CSSProperties,
  type Dispatch,
  type FormEvent,
  type ReactNode,
  type SetStateAction,
} from "react";

import { mockCatalog } from "../mockCatalog";
import { skillCategoryLabel } from "../skillCategories";
import adobeIcon from "../assets/catalog/adobe.svg";
import figmaIcon from "../assets/catalog/figma.svg";
import gitIcon from "../assets/catalog/git.svg";
import googleAnalyticsIcon from "../assets/catalog/googleanalytics.svg";
import microsoftIcon from "../assets/catalog/microsoft.svg";
import yunshuLogo from "../assets/branding/yunshu-workstation.png";
import claudeCodeIcon from "../assets/engines/claudecode.svg";
import codexIcon from "../assets/engines/codex.svg";
import openClawIcon from "../assets/engines/openclaw.svg";
import {
  resolveEngineSwitchPage,
  type EngineId,
  type PageId,
} from "../navigation";
import {
  activateLicense,
  applyCodexLocaleSeed,
  codexLocaleStateLabel,
  bootstrapWorkstation,
  checkInWorkstation,
  cancelImageGeneration,
  chooseChatAttachmentFiles,
  chooseLocalSkillFile,
  choosePortableWorkspace,
  clearCodexProxy,
  clearImageHistory,
  createChatThread,
  deleteChatThread,
  discoverImageProviderModels,
  discoverModels,
  discoverChatProviderModels,
  disableCodexProxy,
  enablePersistentCodexProxy,
  executeFactoryReset,
  exportImageAsset,
  generateImage,
  initializeChat,
  initializeImageStudio,
  importChatAttachments,
  installAllCodexPlugins,
  installCodexPlugin,
  installSkill as installSkillToEngine,
  inspectLocalSkill,
  isTauriRuntime,
  listChatThreads,
  loadChatProviders,
  loadEngineStates,
  loadFactoryResetStatus,
  loadChatThread,
  loadCodexLocaleStatus,
  loadCodexProxyStatus,
  listImageHistory,
  loadInstalledSkills,
  loadLicenseStatus,
  loadLatestAnnouncement,
  loadPluginCatalog,
  loadRecommendedSkills,
  loadTutorials,
  loadUiPreferences,
  markAnnouncementRead,
  openImageOutputDirectory,
  openBackups,
  openExternalUrl,
  openOpenClawControl,
  openLogs,
  readImageAsset,
  readImageReference,
  readTutorial,
  renameChatThread,
  removeChatAttachment,
  previewFactoryReset,
  rollbackFactoryReset,
  runDiagnosis,
  saveProvider as saveProviderToDisk,
  saveChatProvider,
  scanCodexProxySubscription,
  selectCodexProxyNode,
  saveImageProvider,
  saveUiPreferences,
  searchSkills,
  sendChatMessage,
  setChatThreadProvider,
  setActiveImageProvider,
  startEngine,
  startCodexWithInitProxy,
  stopEngine,
  engineCanStop,
  engineSupportsStop,
  shouldSaveBeforeLaunch,
  runtimeActionLabel,
  runtimeStatusHint,
  testProvider,
  testChatProvider,
  testImageProvider,
  verifyFactorySeed,
  type ChatAttachment,
  type ChatMessage,
  type ChatProviderProtocol,
  type ChatProviderState,
  type ChatProviderView,
  type ChatThreadSummary,
  type CodexProxyRegion,
  type CodexLocaleInitializationStatus,
  type EngineState,
  type FactoryResetPreview,
  type FactoryResetStatus,
  type FactoryResetStep,
  type FactorySeedStatus,
  type ImageAsset,
  type ImageGenerationEvent,
  type ImageJob,
  type ImageProviderState,
  type ImageProviderView,
  type InstalledSkillItem,
  type LicenseStatus,
  type LocalSkillDescriptor,
  type LocalSkillInspection,
  type SkillCatalogItem,
  type TutorialDocument,
  type TutorialEntry,
  type ProviderView,
  type UiPreferences,
  type WorkstationAnnouncement,
} from "../workstationApi";
import type { PluginItem } from "../types";
import {
  credentialDisplayValue,
  isSavedCredentialMasked,
} from "../credentialField";

const MarkdownTutorial = lazy(async () => {
  const module = await import("../features/tutorials/MarkdownTutorial");
  return { default: module.MarkdownTutorial };
});

type NoticeKind = "success" | "neutral";
type IconComponent = ComponentType<{
  "aria-hidden"?: boolean | "true" | "false";
  className?: string;
  weight?: "thin" | "light" | "regular" | "bold" | "fill" | "duotone";
}>;

type ProviderDraft = {
  provider: string;
  baseUrl: string;
  apiKey: string;
  model: string;
  protocol: string;
  availableModels?: string[];
};

type ClaudeRuntimeDraft = {
  chooseWorkspaceOnLaunch: boolean;
  workspacePath: string;
  opusModel: string;
  sonnetModel: string;
  haikuModel: string;
  subagentModel: string;
};

type Engine = {
  id: EngineId;
  name: string;
  shortName: string;
  description: string;
  accent: string;
  iconSrc: string;
};

type SkillItem = {
  id: string;
  reference?: string;
  name: string;
  description: string;
  category: string;
  version: string;
  source: string;
  iconSrc: string;
  longDescription: string;
};

type LibraryView = "catalog" | "installed";

type SkillLibraryState = {
  query: string;
  category: string;
  view: LibraryView;
  catalogItems: SkillItem[];
};

type SkillImportDraft = {
  kind: "zip" | "markdown";
  inspection: LocalSkillInspection;
  installDependencies: boolean;
};

const ENGINES: Engine[] = [
  {
    id: "codex",
    name: "Codex",
    shortName: "CX",
    description: "代码编写、项目维护与自动化任务",
    accent: "#111827",
    iconSrc: codexIcon,
  },
  {
    id: "claude",
    name: "Claude Code",
    shortName: "CC",
    description: "代码理解、长上下文分析与协作开发",
    accent: "#b45309",
    iconSrc: claudeCodeIcon,
  },
  {
    id: "openclaw",
    name: "OpenClaw",
    shortName: "OC",
    description: "本地智能体、工具调用与多步骤工作流",
    accent: "#e64545",
    iconSrc: openClawIcon,
  },
];

const DEFAULT_PROVIDER_BASE_URL = "https://sub2api.jackchen6660.dpdns.org/v1";
const CODEX_PROXY_DISCLAIMER_STORAGE_KEY =
  "yunshu.codexProxyDisclaimerDismissed.v1";

const DEFAULT_PROVIDERS: Record<EngineId, ProviderDraft> = {
  codex: {
    provider: "云枢 API",
    baseUrl: DEFAULT_PROVIDER_BASE_URL,
    apiKey: "",
    model: "gpt-5.2-codex",
    protocol: "自动识别（推荐）",
  },
  claude: {
    provider: "Anthropic 兼容服务",
    baseUrl: DEFAULT_PROVIDER_BASE_URL,
    apiKey: "",
    model: "sonnet",
    protocol: "自动识别（推荐）",
  },
  openclaw: {
    provider: "OpenAI 兼容服务",
    baseUrl: DEFAULT_PROVIDER_BASE_URL,
    apiKey: "",
    model: "gpt-5.2",
    protocol: "自动识别（推荐）",
  },
};

const DEFAULT_CLAUDE_RUNTIME: ClaudeRuntimeDraft = {
  chooseWorkspaceOnLaunch: true,
  workspacePath: "Workspace/Claude Code",
  opusModel: "opus",
  sonnetModel: "sonnet",
  haikuModel: "haiku",
  subagentModel: "inherit",
};

const BASE_NAV_ITEMS: Array<{
  id: PageId;
  label: string;
  description: string;
  icon: IconComponent;
}> = [
  { id: "home", label: "首页", description: "启动与服务", icon: House },
  { id: "chat", label: "AI 对话", description: "本地会话", icon: ChatCircleDots },
  { id: "image", label: "AI 生图", description: "图片创作", icon: ImageSquare },
  { id: "skills", label: "Skill 库", description: "智能体技能", icon: Books },
  { id: "plugins", label: "插件库", description: "Codex 插件", icon: PuzzlePiece },
  { id: "tutorial", label: "教程", description: "使用指南", icon: GraduationCap },
  { id: "settings", label: "设置", description: "工作站管理", icon: GearSix },
];

const SKILL_ITEMS: SkillItem[] = [
  {
    id: "find-skills",
    name: "Find Skills",
    description: "根据任务目标查找、筛选并推荐合适的智能体 Skill。",
    category: "效率工具",
    version: "1.0.0",
    source: "SkillHub",
    iconSrc: googleAnalyticsIcon,
    longDescription:
      "根据用户的目标检索适合的 Skill，比较用途、来源和安装要求，帮助智能体快速补齐能力。",
  },
  {
    id: "documents",
    name: "Documents",
    description: "创建、读取和修改 Word 文档，并检查排版与内容结构。",
    category: "办公创作",
    version: "1.2.0",
    source: "SkillHub",
    iconSrc: microsoftIcon,
    longDescription:
      "面向 Word 和通用办公文档的创建、读取、改写与排版检查能力，适合报告和方案整理。",
  },
  {
    id: "pdf",
    name: "PDF",
    description: "读取、提取、合并和生成 PDF，支持页面级检查。",
    category: "办公创作",
    version: "1.1.3",
    source: "SkillHub",
    iconSrc: adobeIcon,
    longDescription:
      "提供 PDF 文本与页面读取、合并、生成和检查能力，并保留页面级处理信息。",
  },
  {
    id: "frontend-design",
    name: "Frontend Design",
    description: "辅助构建现代、清晰且可以直接运行的产品界面。",
    category: "开发工具",
    version: "3.0.1",
    source: "SkillHub",
    iconSrc: figmaIcon,
    longDescription:
      "提供前端信息架构、组件设计、视觉规范和交互实现建议，适合构建正式产品界面。",
  },
  {
    id: "git-essentials",
    name: "Git Essentials",
    description: "处理仓库检查、分支管理、提交和常用协作流程。",
    category: "开发工具",
    version: "1.0.0",
    source: "SkillHub",
    iconSrc: gitIcon,
    longDescription:
      "处理 Git 仓库检查、分支、提交、差异确认与常见协作流程，避免破坏用户已有修改。",
  },
  {
    id: "market-research",
    name: "Market Research",
    description: "整理竞品、市场结构与用户需求，生成结构化调研结果。",
    category: "研究分析",
    version: "1.0.0",
    source: "SkillHub",
    iconSrc: googleAnalyticsIcon,
    longDescription:
      "从公开资料中整理市场、竞品、用户需求与机会点，输出便于继续决策的结构化结果。",
  },
];

export function App() {
  const pageStageRef = useRef<HTMLElement | null>(null);
  const authorizationHydrationStarted = useRef(false);
  const checkInStarted = useRef(false);
  const bootstrapStarted = useRef(false);
  const skillHydrationStarted = useRef(false);
  const pluginHydrationStarted = useRef(false);
  const initialSkillItems = isTauriRuntime() ? [] : SKILL_ITEMS;
  const [page, setPage] = useState<PageId>("home");
  const [activeEngine, setActiveEngine] = useState<EngineId>("codex");
  const [providerDrafts, setProviderDrafts] =
    useState<Record<EngineId, ProviderDraft>>(DEFAULT_PROVIDERS);
  const [claudeRuntimeDraft, setClaudeRuntimeDraft] =
    useState<ClaudeRuntimeDraft>(DEFAULT_CLAUDE_RUNTIME);
  const [savedEngines, setSavedEngines] = useState<Set<EngineId>>(
    () => new Set(),
  );
  const [credentialReady, setCredentialReady] = useState<
    Record<EngineId, boolean>
  >({ codex: false, claude: false, openclaw: false });
  const [runtimeStates, setRuntimeStates] = useState<
    Partial<Record<EngineId, EngineState>>
  >({});
  const [launchingEngine, setLaunchingEngine] = useState<EngineId | null>(null);
  const [stoppingEngine, setStoppingEngine] = useState<EngineId | null>(null);
  const [workstationRoot, setWorkstationRoot] = useState("");
  const [skillItems, setSkillItems] = useState<SkillItem[]>(initialSkillItems);
  const [skillCatalogReady, setSkillCatalogReady] = useState(
    () => !isTauriRuntime(),
  );
  const [skillLibraryState, setSkillLibraryState] =
    useState<SkillLibraryState>({
      query: "",
      category: "全部",
      view: "catalog",
      catalogItems: initialSkillItems,
    });
  const [pluginItems, setPluginItems] = useState<PluginItem[]>(
    () => (isTauriRuntime() ? [] : mockCatalog.items),
  );
  const [pluginCategories, setPluginCategories] = useState<string[]>(
    () => (isTauriRuntime() ? [] : mockCatalog.categories),
  );
  const [installedSkills, setInstalledSkills] = useState<
    Record<EngineId, Set<string>>
  >(() => ({
    codex: isTauriRuntime() ? new Set() : new Set(["find-skills", "pdf"]),
    claude: isTauriRuntime() ? new Set() : new Set(["documents"]),
    openclaw: new Set(),
  }));
  const [installedPlugins, setInstalledPlugins] = useState<Set<string>>(
    () =>
      isTauriRuntime()
        ? new Set()
        : new Set(
            mockCatalog.items
              .filter((plugin) => plugin.installed)
              .map((plugin) => plugin.id),
          ),
  );
  const [toast, setToast] = useState<{
    message: string;
    kind: NoticeKind;
  } | null>(null);
  const [licenseStatus, setLicenseStatus] = useState<LicenseStatus | null>(
    () =>
      isTauriRuntime()
        ? null
        : {
            state: "activated",
            installationId: "browser-preview",
            message: "浏览器预览模式",
          },
  );
  const [announcement, setAnnouncement] =
    useState<WorkstationAnnouncement | null>(null);
  const [latestAnnouncement, setLatestAnnouncement] =
    useState<WorkstationAnnouncement | null>(null);

  useEffect(() => {
    if (authorizationHydrationStarted.current || !isTauriRuntime()) return;
    authorizationHydrationStarted.current = true;
    void loadLicenseStatus()
      .then(setLicenseStatus)
      .catch((error) => {
        setLicenseStatus({
          state: "invalid",
          installationId: "",
          message: errorMessage(error),
        });
      });
  }, []);

  useEffect(() => {
    if (licenseStatus?.state !== "activated") return;
    if (bootstrapStarted.current) return;
    bootstrapStarted.current = true;
    let cancelled = false;
    const hydrate = async () => {
      try {
        const bootstrap = await bootstrapWorkstation();
        if (!bootstrap || cancelled) return;
        setWorkstationRoot(bootstrap.paths.root);
        setRuntimeStates(indexEngineStates(bootstrap.states));
        hydrateProviders(
          bootstrap.providers,
          setProviderDrafts,
          setClaudeRuntimeDraft,
          setSavedEngines,
          setCredentialReady,
        );

      } catch (error) {
        if (!cancelled) {
          setToast({ message: errorMessage(error), kind: "neutral" });
        }
      }
    };
    void hydrate();
    return () => {
      cancelled = true;
    };
  }, [licenseStatus?.state]);

  useEffect(() => {
    if (
      licenseStatus?.state !== "activated" ||
      !isTauriRuntime() ||
      checkInStarted.current
    ) {
      return;
    }
    checkInStarted.current = true;
    // 上报和公告完全在后台执行；网络异常不会改变本地授权，也不会阻止主界面。
    void (async () => {
      const cachedAnnouncement = await loadLatestAnnouncement().catch(() => null);
      if (cachedAnnouncement) {
        setLatestAnnouncement(cachedAnnouncement.announcement);
        if (!cachedAnnouncement.isRead) {
          setAnnouncement(cachedAnnouncement.announcement);
        }
      }

      const nextAnnouncement = await checkInWorkstation().catch(() => null);
      if (nextAnnouncement) {
        setLatestAnnouncement(nextAnnouncement);
        setAnnouncement(nextAnnouncement);
      }
    })();
  }, [licenseStatus?.state]);

  useEffect(() => {
    if (page !== "skills" || !isTauriRuntime() || skillHydrationStarted.current) {
      return;
    }
    skillHydrationStarted.current = true;
    const hydrateSkills = async () => {
      try {
        const [installed, recommended] = await Promise.all([
          loadInstalledSkills(),
          loadRecommendedSkills().catch(() => []),
        ]);
        setInstalledSkills(indexInstalledSkills(installed));
        const catalogItems = mergeSkillItems(
          recommended.map(skillCatalogToItem),
          installed.map(installedSkillToItem),
        );
        setSkillItems(catalogItems);
        setSkillLibraryState((current) => ({
          ...current,
          catalogItems:
            current.query || current.category !== "全部"
              ? current.catalogItems
              : catalogItems,
        }));
      } catch (error) {
        skillHydrationStarted.current = false;
        setToast({ message: errorMessage(error), kind: "neutral" });
      } finally {
        setSkillCatalogReady(true);
      }
    };
    void hydrateSkills();
  }, [page]);

  useEffect(() => {
    if (
      page !== "plugins" ||
      activeEngine !== "codex" ||
      !isTauriRuntime() ||
      pluginHydrationStarted.current
    ) {
      return;
    }
    pluginHydrationStarted.current = true;
    const hydratePlugins = async () => {
      try {
        const plugins = await loadPluginCatalog();
        setPluginItems(plugins.items);
        setPluginCategories(plugins.categories);
        setInstalledPlugins(
          new Set(
            plugins.items
              .filter((plugin) => plugin.installed)
              .map((plugin) => plugin.id),
          ),
        );
      } catch (error) {
        pluginHydrationStarted.current = false;
        setToast({ message: errorMessage(error), kind: "neutral" });
      }
    };
    void hydratePlugins();
  }, [activeEngine, page]);

  useEffect(() => {
    if (!isTauriRuntime() || licenseStatus?.state !== "activated") return;
    let cancelled = false;
    let inFlight = false;
    const refresh = async () => {
      // 状态命令可能涉及磁盘和进程查询。上一轮未结束时跳过本轮，
      // 防止慢 U 盘或异常进程导致请求堆积，最终拖死 WebView。
      if (inFlight) return;
      inFlight = true;
      try {
        const states = await loadEngineStates();
        if (!cancelled) setRuntimeStates(indexEngineStates(states));
      } catch {
        // 状态轮询失败不会打断页面操作；下一轮会自动恢复。
      } finally {
        inFlight = false;
      }
    };
    void refresh();
    // 启动、停止命令完成后会直接刷新对应状态；后台轮询只负责兜底。
    // 降到15秒一次，避免慢U盘上频繁读取状态文件影响输入流畅度。
    const timer = window.setInterval(() => void refresh(), 15_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [licenseStatus?.state]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 2400);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    pageStageRef.current?.scrollTo({ top: 0 });
  }, [page]);

  useEffect(() => {
    if (page === "plugins" && activeEngine !== "codex") {
      setPage("engine");
    }
  }, [activeEngine, page]);

  const openEngineConfig = (engineId: EngineId) => {
    setActiveEngine(engineId);
    setPage("engine");
  };

  const switchEngine = (engineId: EngineId) => {
    const nextPage = resolveEngineSwitchPage(page, engineId);
    const leftCodexPluginPage =
      page === "plugins" && engineId !== "codex";

    setActiveEngine(engineId);
    setPage(nextPage);

    if (leftCodexPluginPage) {
      const engine = ENGINES.find((item) => item.id === engineId);
      setToast({
        message: `插件库仅支持 Codex，已进入 ${engine?.name ?? "当前智能体"} 配置`,
        kind: "neutral",
      });
    }
  };

  const saveProvider = async (engineId: EngineId) => {
    const draft = providerDrafts[engineId];
    const claude = engineId === "claude" ? claudeRuntimeDraft : undefined;
    try {
      await saveProviderToDisk({
        engine: engineId,
        displayName: draft.provider,
        baseUrl: draft.baseUrl,
        apiKey: draft.apiKey || undefined,
        protocol: protocolToWire(draft.protocol),
        models: {
          available: engineId === "codex" ? draft.availableModels ?? [] : undefined,
          default: draft.model,
          opus: claude?.opusModel || draft.model,
          sonnet: claude?.sonnetModel || draft.model,
          haiku: claude?.haikuModel || draft.model,
          subagent: claude?.subagentModel || "inherit",
        },
        workspace: claude
          ? {
              chooseOnLaunch: claude.chooseWorkspaceOnLaunch,
              relativePath: claude.workspacePath || "workspace",
            }
          : undefined,
      });
      setSavedEngines((current) => new Set(current).add(engineId));
      setCredentialReady((current) => ({ ...current, [engineId]: true }));
      setProviderDrafts((current) => ({
        ...current,
        [engineId]: { ...current[engineId], apiKey: "" },
      }));
      const engine = ENGINES.find((item) => item.id === engineId);
      setToast({
        message: `${engine?.name ?? "智能体"} 供应商配置已保存`,
        kind: "success",
      });
    } catch (error) {
      setToast({ message: errorMessage(error), kind: "neutral" });
      throw error;
    }
  };

  const launchEngine = async (engineId: EngineId) => {
    if (launchingEngine) return;
    const engine = ENGINES.find((item) => item.id === engineId);
    setLaunchingEngine(engineId);
    try {
      const reopeningOpenClaw =
        engineId === "openclaw" &&
        runtimeStates.openclaw?.status === "running";
      if (
        shouldSaveBeforeLaunch(
          engineId,
          runtimeStates[engineId],
          savedEngines.has(engineId),
        )
      ) {
        await saveProvider(engineId);
      }
      const state =
        reopeningOpenClaw
          ? await openOpenClawControl()
          : await startEngine(engineId);
      setRuntimeStates((current) => ({ ...current, [engineId]: state }));
      setToast({
        message:
          engineId === "openclaw"
            ? (state.message ?? "OpenClaw网关已启动，正在打开浏览器控制台")
            : `${engine?.name ?? "智能体"} 已从移动工作站启动`,
        kind: "success",
      });
    } catch (error) {
      setToast({ message: errorMessage(error), kind: "neutral" });
      throw error;
    } finally {
      setLaunchingEngine(null);
    }
  };

  const stopRunningEngine = async (engineId: EngineId) => {
    if (stoppingEngine) return;
    const engine = ENGINES.find((item) => item.id === engineId);
    setStoppingEngine(engineId);
    try {
      const state = await stopEngine(engineId);
      setRuntimeStates((current) => ({ ...current, [engineId]: state }));
      setToast({
        message:
          state.status === "saving"
            ? `${engine?.name ?? "智能体"} 已关闭，正在保存随盘数据`
            : `${engine?.name ?? "智能体"} 已安全停止`,
        kind: "success",
      });
    } catch (error) {
      setToast({ message: errorMessage(error), kind: "neutral" });
      throw error;
    } finally {
      setStoppingEngine(null);
    }
  };

  const installSkill = async (engineId: EngineId, skill: SkillItem) => {
    try {
      if (isTauriRuntime()) {
        await installSkillToEngine(skill.reference ?? skill.id, engineId);
      }
      setInstalledSkills((current) => ({
        ...current,
        [engineId]: new Set(current[engineId]).add(skill.id),
      }));
      const engine = ENGINES.find((item) => item.id === engineId);
      setToast({
        message: `${skill.name} 已安装到 ${engine?.name ?? "当前智能体"}`,
        kind: "success",
      });
    } catch (error) {
      setToast({ message: errorMessage(error), kind: "neutral" });
      throw error;
    }
  };

  const importLocalSkills = async (
    engineId: EngineId,
    draft: SkillImportDraft,
  ) => {
    if (!draft.inspection.skills.length) {
      throw new Error("安装包中没有可安装的 SKILL.md");
    }
    if (isTauriRuntime()) {
      await installSkillToEngine(draft.inspection.source, engineId, {
        installDependencies: draft.installDependencies,
        offline: true,
      });
    }

    const importedItems = draft.inspection.skills.map(localSkillToItem);
    setSkillItems((current) => mergeSkillItems(importedItems, current));
    setSkillLibraryState((current) => ({
      ...current,
      catalogItems: mergeSkillItems(importedItems, current.catalogItems),
    }));

    if (isTauriRuntime()) {
      const installed = await loadInstalledSkills();
      setInstalledSkills(indexInstalledSkills(installed));
    } else {
      setInstalledSkills((current) => {
        const next = new Set(current[engineId]);
        draft.inspection.skills.forEach((skill) => next.add(skill.slug));
        return { ...current, [engineId]: next };
      });
    }

    const engine = ENGINES.find((item) => item.id === engineId);
    setToast({
      message: `${draft.inspection.skills.length} 个 Skill 已导入 ${engine?.name ?? "当前智能体"}`,
      kind: "success",
    });
  };

  const installPlugin = async (pluginId: string) => {
    try {
      if (isTauriRuntime()) await installCodexPlugin(pluginId);
      setInstalledPlugins((current) => new Set(current).add(pluginId));
      const plugin = pluginItems.find((item) => item.id === pluginId);
      setToast({
        message: `${plugin?.displayName ?? "插件"} 已安装到 Codex`,
        kind: "success",
      });
    } catch (error) {
      setToast({ message: errorMessage(error), kind: "neutral" });
      throw error;
    }
  };

  const installAllPlugins = async (ids: string[]) => {
    try {
      if (isTauriRuntime()) await installAllCodexPlugins(ids);
      setInstalledPlugins((current) => {
        const next = new Set(current);
        ids.forEach((id) => next.add(id));
        return next;
      });
      setToast({ message: "Codex 插件装配完成", kind: "success" });
    } catch (error) {
      setToast({ message: errorMessage(error), kind: "neutral" });
      throw error;
    }
  };

  const currentEngine =
    ENGINES.find((engine) => engine.id === activeEngine) ?? ENGINES[0];

  if (!licenseStatus) {
    return <LicenseLoadingScreen />;
  }

  if (licenseStatus.state !== "activated") {
    return (
      <ActivationPage
        status={licenseStatus}
        onActivated={setLicenseStatus}
      />
    );
  }

  const dismissAnnouncement = () => {
    const version = announcement?.version;
    setAnnouncement(null);
    if (version) void markAnnouncementRead(version).catch(() => undefined);
  };

  return (
    <div className="app-shell">
      <Sidebar
        activeEngine={currentEngine}
        activePage={page}
        onNavigate={setPage}
      />

      <div className="app-workspace">
        <Topbar
          activeEngine={activeEngine}
          onEngineClick={switchEngine}
          page={page}
        />

        <main
          className={`page-stage page-stage--${page}`}
          ref={pageStageRef}
        >
          {page === "home" ? (
            <HomePage
              announcement={latestAnnouncement}
              onEngineClick={openEngineConfig}
              onAnnouncementClick={setAnnouncement}
              onNavigate={setPage}
              savedEngines={savedEngines}
            />
          ) : null}
          {page === "engine" ? (
            <EnginePage
              announcement={latestAnnouncement}
              draft={providerDrafts[activeEngine]}
              engine={currentEngine}
              claudeRuntimeDraft={
                activeEngine === "claude" ? claudeRuntimeDraft : undefined
              }
              onChange={(nextDraft) => {
                setProviderDrafts((current) => ({
                  ...current,
                  [activeEngine]: nextDraft,
                }));
                setSavedEngines((current) => {
                  const next = new Set(current);
                  next.delete(activeEngine);
                  return next;
                });
              }}
              onClaudeRuntimeChange={(nextDraft) => {
                setClaudeRuntimeDraft(nextDraft);
                setSavedEngines((current) => {
                  const next = new Set(current);
                  next.delete("claude");
                  return next;
                });
              }}
              onLaunch={() => launchEngine(activeEngine)}
              onAnnouncementClick={setAnnouncement}
              onSave={() => saveProvider(activeEngine)}
              onStop={() => stopRunningEngine(activeEngine)}
              credentialPresent={credentialReady[activeEngine]}
              launching={launchingEngine === activeEngine}
              stopping={stoppingEngine === activeEngine}
              runtimeState={runtimeStates[activeEngine]}
              workstationRoot={workstationRoot}
              saved={savedEngines.has(activeEngine)}
            />
          ) : null}
          {page === "chat" ? <ChatPage /> : null}
          {page === "image" ? <ImagePage engine={currentEngine} /> : null}
          {page === "skills" ? (
            <SkillLibraryPage
              catalogReady={skillCatalogReady}
              engine={currentEngine}
              installed={installedSkills[activeEngine]}
              items={skillItems}
              onImport={(draft) => importLocalSkills(activeEngine, draft)}
              onInstall={(skill) => installSkill(activeEngine, skill)}
              onStateChange={setSkillLibraryState}
              state={skillLibraryState}
            />
          ) : null}
          {page === "plugins" && activeEngine === "codex" ? (
            <PluginLibraryPage
              categories={pluginCategories}
              installed={installedPlugins}
              items={pluginItems}
              onInstallAll={installAllPlugins}
              onInstall={installPlugin}
            />
          ) : null}
          {page === "tutorial" ? (
            <TutorialPage engine={currentEngine} />
          ) : null}
          {page === "settings" ? (
            <SettingsPage
              activeEngine={currentEngine}
              onOpenProvider={openEngineConfig}
              savedEngines={savedEngines}
              runtimeStates={runtimeStates}
              workstationRoot={workstationRoot}
            />
          ) : null}
        </main>
      </div>

      {toast ? (
        <div className={`toast toast--${toast.kind}`} role="status">
          {toast.kind === "success" ? (
            <CheckCircle aria-hidden="true" weight="fill" />
          ) : (
            <Info aria-hidden="true" weight="fill" />
          )}
          <span>{toast.message}</span>
        </div>
      ) : null}
      {announcement ? (
        <div className="announcement-overlay" role="presentation">
          <section
            aria-labelledby="startup-announcement-title"
            aria-modal="true"
            className={`startup-announcement startup-announcement--${announcement.level}`}
            role="dialog"
          >
            <div className="startup-announcement__icon">
              <Megaphone aria-hidden="true" weight="duotone" />
            </div>
            <p className="eyebrow">云枢工作站公告</p>
            <h2 id="startup-announcement-title">{announcement.title}</h2>
            <div className="startup-announcement__content">
              {announcement.content}
            </div>
            <button className="primary-button" onClick={dismissAnnouncement}>
              我已阅读
            </button>
          </section>
        </div>
      ) : null}
    </div>
  );
}

function LicenseLoadingScreen() {
  return (
    <main className="license-shell">
      <section className="license-card license-card--loading">
        <img alt="云枢AI便携工作站" src={yunshuLogo} />
        <SpinnerGap aria-hidden="true" className="spin" weight="bold" />
        <h1>正在验证移动工作站</h1>
        <p>只读取移动盘本地永久授权，不等待网络连接。</p>
      </section>
    </main>
  );
}

function ActivationPage({
  status,
  onActivated,
}: {
  status: LicenseStatus;
  onActivated: (status: LicenseStatus) => void;
}) {
  const [licenseCode, setLicenseCode] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (submitting || !licenseCode.trim()) return;
    setSubmitting(true);
    setError("");
    try {
      const activated = await activateLicense(licenseCode);
      setLicenseCode("");
      onActivated(activated);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <main className="license-shell">
      <section className="license-card">
        <div className="license-card__brand">
          <img alt="云枢AI便携工作站" src={yunshuLogo} />
          <div>
            <p className="eyebrow">YUNSHU AI PORTABLE WORKSTATION</p>
            <h1>首次激活移动工作站</h1>
          </div>
        </div>
        <p className="license-card__message">{status.message}</p>
        <form onSubmit={submit}>
          <label htmlFor="workstation-license-code">云枢工作站卡密</label>
          <div className="license-input">
            <Key aria-hidden="true" weight="duotone" />
            <input
              autoComplete="off"
              autoFocus
              id="workstation-license-code"
              maxLength={256}
              onChange={(event) => setLicenseCode(event.target.value)}
              placeholder="请输入卡密"
              spellCheck={false}
              value={licenseCode}
            />
          </div>
          {error ? <div className="license-error">{error}</div> : null}
          <button
            className="primary-button license-submit"
            disabled={submitting || licenseCode.trim().length < 8}
            type="submit"
          >
            {submitting ? (
              <>
                <SpinnerGap aria-hidden="true" className="spin" weight="bold" />
                正在安全激活
              </>
            ) : (
              <>
                <ShieldCheck aria-hidden="true" weight="duotone" />
                激活并进入工作站
              </>
            )}
          </button>
        </form>
        <div className="license-card__notes">
          <span>仅首次激活联网</span>
          <span>授权保存在当前移动盘</span>
          <span>后续可离线启动</span>
        </div>
      </section>
    </main>
  );
}

function indexEngineStates(
  states: EngineState[],
): Partial<Record<EngineId, EngineState>> {
  return Object.fromEntries(
    states.map((state) => [state.engine, state]),
  ) as Partial<Record<EngineId, EngineState>>;
}

function indexInstalledSkills(
  records: InstalledSkillItem[],
): Record<EngineId, Set<string>> {
  const result: Record<EngineId, Set<string>> = {
    codex: new Set(),
    claude: new Set(),
    openclaw: new Set(),
  };
  records
    .filter((record) => record.verified)
    .forEach((record) => result[record.target].add(record.slug));
  return result;
}

function installedSkillToItem(skill: InstalledSkillItem): SkillItem {
  const description = skill.description || "从移动工作站本地安装的 Skill。";
  return {
    id: skill.slug,
    reference: skill.slug,
    name: skill.name || skill.slug,
    description,
    longDescription: description,
    category: "本地导入",
    version: skill.version || "local",
    source: localSkillSourceLabel(skill.source),
    iconSrc: googleAnalyticsIcon,
  };
}

function localSkillToItem(skill: LocalSkillDescriptor): SkillItem {
  const description = skill.description || "从本地文件导入的 Skill。";
  return {
    id: skill.slug,
    reference: skill.slug,
    name: skill.name || skill.slug,
    description,
    longDescription: description,
    category: "本地导入",
    version: skill.version || "local",
    source: "本地导入",
    iconSrc: googleAnalyticsIcon,
  };
}

function localSkillSourceLabel(source: string): string {
  if (source === "local-zip" || source === "local:zip") return "本地 ZIP";
  if (source === "local-file" || source === "local:file") return "本地 Markdown";
  return source || "本地导入";
}

function mergeSkillItems(
  primary: SkillItem[],
  secondary: SkillItem[],
): SkillItem[] {
  const merged = new Map(secondary.map((item) => [item.id, item]));
  primary.forEach((item) => merged.set(item.id, item));
  return [...merged.values()];
}

function sameSkillList(left: SkillItem[], right: SkillItem[]): boolean {
  return (
    left.length === right.length &&
    left.every((item, index) => {
      const other = right[index];
      return (
        item.id === other?.id &&
        item.name === other.name &&
        item.description === other.description &&
        item.version === other.version
      );
    })
  );
}

function fileNameFromPath(path: string): string {
  const parts = path.replaceAll("\\", "/").split("/").filter(Boolean);
  return parts.at(-1) || path;
}

function hasSkillDependencies(skill: LocalSkillDescriptor): boolean {
  return Object.values(skill.dependencies).some(Boolean);
}

function skillCatalogToItem(skill: SkillCatalogItem): SkillItem {
  return {
    id: skill.id,
    reference: skill.reference,
    name: skill.name,
    description: skill.description,
    longDescription: skill.longDescription,
    category: skill.category,
    version: skill.version,
    source: skill.source,
    iconSrc: skill.iconUrl || googleAnalyticsIcon,
  };
}

function hydrateProviders(
  providers: ProviderView[],
  setDrafts: Dispatch<SetStateAction<Record<EngineId, ProviderDraft>>>,
  setClaude: Dispatch<SetStateAction<ClaudeRuntimeDraft>>,
  setSaved: Dispatch<SetStateAction<Set<EngineId>>>,
  setCredentials: Dispatch<SetStateAction<Record<EngineId, boolean>>>,
) {
  const configured = new Set<EngineId>();
  const credentials: Record<EngineId, boolean> = {
    codex: false,
    claude: false,
    openclaw: false,
  };
  providers.forEach((provider) => {
    if (provider.configured) configured.add(provider.engine);
    credentials[provider.engine] = provider.credentialPresent;
  });
  setDrafts((current) => {
    const next = { ...current };
    providers.forEach((provider) => {
      if (!provider.configured) return;
      next[provider.engine] = {
        provider: provider.displayName || current[provider.engine].provider,
        baseUrl: provider.baseUrl || "",
        apiKey: "",
        model:
          provider.models?.default || current[provider.engine].model,
        protocol: protocolFromWire(provider.protocol),
        availableModels: provider.engine === "codex" ? provider.models?.available ?? [] : undefined,
      };
    });
    return next;
  });
  const claude = providers.find(
    (provider) => provider.engine === "claude" && provider.models,
  );
  if (claude?.models) {
    setClaude({
      chooseWorkspaceOnLaunch:
        claude.workspace?.chooseOnLaunch ?? false,
      workspacePath:
        claude.workspace?.relativePath || "workspace/Claude Code",
      opusModel: claude.models.opus,
      sonnetModel: claude.models.sonnet,
      haikuModel: claude.models.haiku,
      subagentModel: claude.models.subagent,
    });
  }
  setSaved(configured);
  setCredentials(credentials);
}

function protocolToWire(value: string): string {
  if (value.includes("Responses")) return "responses";
  if (value.includes("Chat")) return "chat-completions";
  if (value.includes("Anthropic")) return "anthropic-messages";
  return "auto";
}

function protocolFromWire(value?: string): string {
  if (value === "responses") return "OpenAI Responses";
  if (value === "chat-completions") return "OpenAI Chat Completions";
  if (value === "anthropic-messages") return "Anthropic Messages";
  return "自动识别（推荐）";
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "操作失败，请进入设置运行诊断";
}

function Sidebar({
  activeEngine,
  activePage,
  onNavigate,
}: {
  activeEngine: Engine;
  activePage: PageId;
  onNavigate: (page: PageId) => void;
}) {
  const availableItems = BASE_NAV_ITEMS.filter(
    (item) => item.id !== "plugins" || activeEngine.id === "codex",
  );
  const mainItems = availableItems.filter(
    (item) => item.id !== "tutorial" && item.id !== "settings",
  );
  const bottomItems = availableItems.filter(
    (item) => item.id === "tutorial" || item.id === "settings",
  );

  return (
    <aside className="sidebar">
      <div className="brand">
        <span className="brand-mark" aria-hidden="true">
          <img alt="" src={yunshuLogo} />
        </span>
        <span>
          <strong>云枢 AI</strong>
          <small>便携工作站</small>
        </span>
      </div>

      <nav className="primary-nav" aria-label="主导航">
        <div className="nav-group">
          {mainItems.map((item) => (
            <SidebarNavButton
              active={activePage === item.id}
              item={item}
              key={item.id}
              onNavigate={onNavigate}
            />
          ))}
        </div>
        <div className="nav-group nav-group--bottom">
          {bottomItems.map((item) => (
            <SidebarNavButton
              active={activePage === item.id}
              item={item}
              key={item.id}
              onNavigate={onNavigate}
            />
          ))}
        </div>
      </nav>

      <div className="sidebar-foot">
        <EngineMark engine={activeEngine} />
        <span>
          <strong>{activeEngine.name}</strong>
          <small>当前智能体</small>
        </span>
      </div>
    </aside>
  );
}

function SidebarNavButton({
  active,
  item,
  onNavigate,
}: {
  active: boolean;
  item: (typeof BASE_NAV_ITEMS)[number];
  onNavigate: (page: PageId) => void;
}) {
  const Icon = item.icon;
  return (
    <button
      className={active ? "active" : ""}
      onClick={() => onNavigate(item.id)}
      type="button"
    >
      <Icon aria-hidden="true" weight={active ? "fill" : "regular"} />
      <span>
        <strong>{item.label}</strong>
        <small>{item.description}</small>
      </span>
    </button>
  );
}

function Topbar({
  activeEngine,
  onEngineClick,
  page,
}: {
  activeEngine: EngineId;
  onEngineClick: (engineId: EngineId) => void;
  page: PageId;
}) {
  const currentEngine =
    ENGINES.find((engine) => engine.id === activeEngine) ?? ENGINES[0];
  const pageInfo =
    page === "engine"
      ? {
          label: `${currentEngine.name} 配置`,
          description: "供应商与启动",
        }
      : BASE_NAV_ITEMS.find((item) => item.id === page) ?? BASE_NAV_ITEMS[0];

  return (
    <header className="topbar">
      <div className="page-heading">
        <span>{pageInfo.label}</span>
        <small>{pageInfo.description}</small>
      </div>
      <div className="engine-switcher" aria-label="切换当前智能体">
        {ENGINES.map((engine) => (
          <button
            aria-label={`切换到 ${engine.name}`}
            className={activeEngine === engine.id ? "active" : ""}
            key={engine.id}
            onClick={() => onEngineClick(engine.id)}
            title={`切换到 ${engine.name}`}
            type="button"
          >
            <EngineMark engine={engine} />
            <span>{engine.name}</span>
            <CaretRight aria-hidden="true" />
          </button>
        ))}
      </div>
    </header>
  );
}

function HomePage({
  announcement,
  onAnnouncementClick,
  onEngineClick,
  onNavigate,
  savedEngines,
}: {
  announcement: WorkstationAnnouncement | null;
  onAnnouncementClick: (announcement: WorkstationAnnouncement) => void;
  onEngineClick: (engineId: EngineId) => void;
  onNavigate: (page: PageId) => void;
  savedEngines: Set<EngineId>;
}) {
  return (
    <div className="home-page">
      <section className="welcome-panel">
        <div className="welcome-copy">
          <h1>选择智能体，进入专属工作台</h1>
          <p>
            点击 Codex、Claude Code 或 OpenClaw，右侧会进入对应的供应商与启动页面。
            Skill 库会自动切换安装目标。
          </p>
        </div>
        <button
          className="quiet-link"
          onClick={() => onNavigate("settings")}
          type="button"
        >
          工作站设置
          <CaretRight aria-hidden="true" />
        </button>
      </section>

      <section className="engine-launch-list" aria-label="智能体入口">
        {ENGINES.map((engine) => (
          <button
            className="engine-launch-row"
            key={engine.id}
            onClick={() => onEngineClick(engine.id)}
            type="button"
          >
            <EngineMark engine={engine} large />
            <span className="engine-launch-copy">
              <strong>{engine.name}</strong>
              <small>{engine.description}</small>
            </span>
            <span
              className={`engine-config-state ${
                savedEngines.has(engine.id) ? "configured" : ""
              }`}
            >
              {savedEngines.has(engine.id) ? (
                <>
                  <Check aria-hidden="true" />
                  已保存
                </>
              ) : (
                "供应商配置"
              )}
            </span>
            <CaretRight className="engine-row-caret" aria-hidden="true" />
          </button>
        ))}
      </section>

      <ServicesAndAnnouncements
        announcement={announcement}
        onAnnouncementClick={onAnnouncementClick}
      />
    </div>
  );
}

function ServicesAndAnnouncements({
  announcement,
  onAnnouncementClick,
}: {
  announcement?: WorkstationAnnouncement | null;
  onAnnouncementClick?: (announcement: WorkstationAnnouncement) => void;
} = {}) {
  return (
    <section className="home-lower-grid">
      <article className="content-card services-card">
        <header className="card-header">
          <h2>服务入口</h2>
          <small>使用默认浏览器打开</small>
        </header>
        <div className="service-list">
          <button
            className="service-row"
            onClick={() =>
              void openExternalUrl(
                "https://go1.jackchen6660.dpdns.org",
              )
            }
            type="button"
          >
            <span className="service-icon service-icon--api">
              <LinkSimple aria-hidden="true" />
            </span>
            <span>
              <strong>云枢 API</strong>
              <small>模型服务、密钥获取与用量查询</small>
            </span>
            <ArrowSquareOut aria-hidden="true" />
          </button>
          <button
            className="service-row"
            onClick={() =>
              void openExternalUrl(
                "https://go2.jackchen6660.dpdns.org",
              )
            }
            type="button"
          >
            <span className="service-icon service-icon--network">
              <Cloud aria-hidden="true" />
            </span>
            <span>
              <strong>云枢AI便携工作站官网</strong>
              <small>产品介绍、下载与使用说明</small>
            </span>
            <ArrowSquareOut aria-hidden="true" />
          </button>
        </div>
      </article>

      <article
        className={`content-card announcement-card ${
          announcement ? "announcement-card--interactive" : ""
        }`}
        onClick={
          announcement && onAnnouncementClick
            ? () => onAnnouncementClick(announcement)
            : undefined
        }
        onKeyDown={
          announcement && onAnnouncementClick
            ? (event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  onAnnouncementClick(announcement);
                }
              }
            : undefined
        }
        role={announcement && onAnnouncementClick ? "button" : undefined}
        tabIndex={announcement && onAnnouncementClick ? 0 : undefined}
      >
        <header className="card-header">
          <h2>最新公告</h2>
          <Megaphone aria-hidden="true" />
        </header>
        <div className="announcement-list">
          {announcement ? (
            <div className="announcement-item">
              <span className="announcement-dot" />
              <div>
                <strong>{announcement.title}</strong>
                <p>{announcement.content}</p>
              </div>
            </div>
          ) : (
            <div className="announcement-item">
              <span className="announcement-dot announcement-dot--muted" />
              <div>
                <strong>暂无最新公告</strong>
                <p>联网启动后会自动读取云枢工作站公告。</p>
              </div>
            </div>
          )}
          <div className="announcement-item">
            <span className="announcement-dot" />
            <div>
              <strong>欢迎使用云枢 AI 便携工作站</strong>
              <p>三引擎配置相互独立，切换智能体不会覆盖其他配置。</p>
            </div>
          </div>
        </div>
      </article>
    </section>
  );
}

function EnginePage({
  announcement,
  claudeRuntimeDraft,
  credentialPresent,
  draft,
  engine,
  launching,
  stopping,
  onChange,
  onAnnouncementClick,
  onClaudeRuntimeChange,
  onLaunch,
  onSave,
  onStop,
  runtimeState,
  saved,
  workstationRoot,
}: {
  announcement: WorkstationAnnouncement | null;
  claudeRuntimeDraft?: ClaudeRuntimeDraft;
  credentialPresent: boolean;
  draft: ProviderDraft;
  engine: Engine;
  launching: boolean;
  stopping: boolean;
  onChange: (draft: ProviderDraft) => void;
  onAnnouncementClick: (announcement: WorkstationAnnouncement) => void;
  onClaudeRuntimeChange: (draft: ClaudeRuntimeDraft) => void;
  onLaunch: () => Promise<void>;
  onSave: () => Promise<void>;
  onStop: () => Promise<void>;
  runtimeState?: EngineState;
  saved: boolean;
  workstationRoot: string;
}) {
  return (
    <div className="engine-page">
      <section className="engine-page-heading">
        <EngineMark engine={engine} large />
        <div>
          <h1>{engine.name} 工作台</h1>
          <p>{engine.description}。供应商配置只作用于当前智能体。</p>
        </div>
        <span className={`engine-ready-state ${saved ? "configured" : ""}`}>
          {saved ? (
            <>
              <CheckCircle aria-hidden="true" weight="fill" />
              配置已保存
            </>
          ) : (
            "等待配置"
          )}
        </span>
      </section>

      <ProviderPanel
        claudeRuntimeDraft={claudeRuntimeDraft}
        credentialPresent={credentialPresent}
        draft={draft}
        engine={engine}
        launching={launching}
        stopping={stopping}
        onChange={onChange}
        onClaudeRuntimeChange={onClaudeRuntimeChange}
        onLaunch={onLaunch}
        onSave={onSave}
        onStop={onStop}
        runtimeState={runtimeState}
        saved={saved}
        workstationRoot={workstationRoot}
      />

      <ServicesAndAnnouncements
        announcement={announcement}
        onAnnouncementClick={onAnnouncementClick}
      />
    </div>
  );
}

function ChatPage() {
  const [threads, setThreads] = useState<ChatThreadSummary[]>([]);
  const [activeThreadId, setActiveThreadId] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [pendingAttachments, setPendingAttachments] = useState<
    ChatAttachment[]
  >([]);
  const [providerState, setProviderState] = useState<ChatProviderState>({
    configured: false,
    providers: [],
  });
  const [providerOpen, setProviderOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [message, setMessage] = useState("");
  const [answering, setAnswering] = useState(false);
  const [loading, setLoading] = useState(true);
  const [operationMessage, setOperationMessage] = useState("");
  const [editingThreadId, setEditingThreadId] = useState("");
  const [editingTitle, setEditingTitle] = useState("");
  const [deletingThreadId, setDeletingThreadId] = useState("");
  const [threadActionId, setThreadActionId] = useState("");

  useEffect(() => {
    let cancelled = false;
    const hydrate = async () => {
      setLoading(true);
      setOperationMessage("");
      setThreads([]);
      setMessages([]);
      setPendingAttachments([]);
      setActiveThreadId("");
      setEditingThreadId("");
      setEditingTitle("");
      setDeletingThreadId("");
      setThreadActionId("");
      try {
        await initializeChat();
        const nextProviderState = await loadChatProviders();
        if (cancelled) return;
        setProviderState(nextProviderState);
        setProviderOpen(!nextProviderState.configured);
        const nextThreads = await listChatThreads("codex");
        if (cancelled) return;
        setThreads(nextThreads);
        const first = nextThreads[0];
        if (first) {
          const detail = await loadChatThread(first.id);
          if (cancelled) return;
          setActiveThreadId(first.id);
          setMessages(detail.messages);
          setPendingAttachments(detail.pendingAttachments);
        }
      } catch (error) {
        if (!cancelled) setOperationMessage(errorMessage(error));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    void hydrate();
    return () => {
      cancelled = true;
    };
  }, []);

  const filteredThreads = useMemo(
    () =>
      threads.filter((thread) =>
        thread.title.toLowerCase().includes(search.trim().toLowerCase()),
      ),
    [search, threads],
  );
  const activeThread =
    threads.find((thread) => thread.id === activeThreadId);
  const activeProvider = providerState.providers.find(
    (provider) =>
      provider.id ===
      (activeThread?.providerId || providerState.activeProviderId),
  );

  const createThread = async () => {
    if (!isTauriRuntime()) {
      setOperationMessage("请在桌面工作站中使用本地对话功能");
      return undefined;
    }
    if (!providerState.configured) {
      setProviderOpen(true);
      setOperationMessage("首次使用请先配置 AI 对话专用供应商");
      return undefined;
    }
    setOperationMessage("");
    try {
      const detail = await createChatThread("codex");
      setThreads((current) => [detail.thread, ...current]);
      setActiveThreadId(detail.thread.id);
      setMessages(detail.messages);
      setPendingAttachments(detail.pendingAttachments);
      return detail.thread;
    } catch (error) {
      setOperationMessage(errorMessage(error));
      return undefined;
    }
  };

  const selectThread = async (threadId: string) => {
    setEditingThreadId("");
    setEditingTitle("");
    setDeletingThreadId("");
    setActiveThreadId(threadId);
    setOperationMessage("");
    try {
      const detail = await loadChatThread(threadId);
      setMessages(detail.messages);
      setPendingAttachments(detail.pendingAttachments);
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  const beginRenameThread = (thread: ChatThreadSummary) => {
    if (answering || threadActionId) return;
    setDeletingThreadId("");
    setEditingThreadId(thread.id);
    setEditingTitle(thread.title);
  };

  const cancelRenameThread = () => {
    setEditingThreadId("");
    setEditingTitle("");
  };

  const saveThreadTitle = async (threadId: string) => {
    const title = editingTitle.trim();
    if (!title) {
      setOperationMessage("对话名称不能为空");
      return;
    }
    if (answering || threadActionId) return;
    setThreadActionId(threadId);
    setOperationMessage("");
    try {
      const updated = await renameChatThread(threadId, title);
      setThreads((current) =>
        current.map((thread) => (thread.id === updated.id ? updated : thread)),
      );
      cancelRenameThread();
    } catch (error) {
      setOperationMessage(errorMessage(error));
    } finally {
      setThreadActionId("");
    }
  };

  const removeThread = async (threadId: string) => {
    if (answering || threadActionId) return;
    setThreadActionId(threadId);
    setOperationMessage("");
    try {
      await deleteChatThread(threadId);
      const remaining = threads.filter((thread) => thread.id !== threadId);
      setThreads(remaining);
      setDeletingThreadId("");
      if (activeThreadId === threadId) {
        const nextThread = remaining[0];
        setActiveThreadId(nextThread?.id ?? "");
        setMessages([]);
        setPendingAttachments([]);
        if (nextThread) {
          const detail = await loadChatThread(nextThread.id);
          setMessages(detail.messages);
          setPendingAttachments(detail.pendingAttachments);
        }
      }
    } catch (error) {
      setOperationMessage(errorMessage(error));
    } finally {
      setThreadActionId("");
    }
  };

  const sendMessage = async (event: FormEvent) => {
    event.preventDefault();
    const cleanMessage = message.trim();
    if ((!cleanMessage && pendingAttachments.length === 0) || answering) return;
    let targetThread = activeThread;
    if (!targetThread) {
      targetThread = await createThread();
      if (!targetThread) return;
    }
    const temporaryMessage: ChatMessage = {
      id: `pending-${Date.now()}`,
      threadId: targetThread.id,
      role: "user",
      content: cleanMessage,
      attachments: pendingAttachments,
      createdAt: Date.now(),
    };
    setMessage("");
    setOperationMessage("");
    setAnswering(true);
    setMessages((current) => [...current, temporaryMessage]);
    try {
      const result = await sendChatMessage(
        targetThread.id,
        cleanMessage,
        pendingAttachments.map((attachment) => attachment.id),
      );
      setPendingAttachments([]);
      setMessages((current) => [
        ...current.filter((item) => item.id !== temporaryMessage.id),
        result.userMessage,
        result.assistantMessage,
      ]);
      setThreads((current) => [
        result.thread,
        ...current.filter((thread) => thread.id !== result.thread.id),
      ]);
    } catch (error) {
      setOperationMessage(errorMessage(error));
      if (isTauriRuntime()) {
        try {
          const detail = await loadChatThread(targetThread.id);
          setMessages(detail.messages);
          setPendingAttachments(detail.pendingAttachments);
          setThreads((current) => [
            detail.thread,
            ...current.filter((thread) => thread.id !== detail.thread.id),
          ]);
        } catch {
          // 保留原错误，下一次打开页面时会从移动盘重新载入。
        }
      }
    } finally {
      setAnswering(false);
    }
  };

  const addAttachments = async () => {
    if (answering) return;
    if (!providerState.configured) {
      setProviderOpen(true);
      setOperationMessage("请先配置 AI 对话专用供应商");
      return;
    }
    let targetThread = activeThread;
    if (!targetThread) {
      targetThread = await createThread();
      if (!targetThread) return;
    }
    try {
      const selectedPaths = await chooseChatAttachmentFiles();
      if (selectedPaths.length === 0) return;
      const imported = await importChatAttachments(
        targetThread.id,
        selectedPaths,
      );
      setPendingAttachments((current) => [...current, ...imported]);
      setOperationMessage("");
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  const discardAttachment = async (attachmentId: string) => {
    try {
      await removeChatAttachment(attachmentId);
      setPendingAttachments((current) =>
        current.filter((attachment) => attachment.id !== attachmentId),
      );
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  const changeModel = async (model: string) => {
    if (!activeThread || !activeProvider || !model) return;
    try {
      const detail = await setChatThreadProvider({
        threadId: activeThread.id,
        providerId: activeProvider.id,
        model,
      });
      setThreads((current) => [
        detail.thread,
        ...current.filter((thread) => thread.id !== detail.thread.id),
      ]);
      setMessages(detail.messages);
      setPendingAttachments(detail.pendingAttachments);
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  const applyProvider = async (
    nextState: ChatProviderState,
    provider: ChatProviderView,
  ) => {
    setProviderState(nextState);
    if (activeThread) {
      const detail = await setChatThreadProvider({
        threadId: activeThread.id,
        providerId: provider.id,
        model: provider.defaultModel,
      });
      setThreads((current) => [
        detail.thread,
        ...current.filter((thread) => thread.id !== detail.thread.id),
      ]);
      setMessages(detail.messages);
      setPendingAttachments(detail.pendingAttachments);
    }
    setProviderOpen(false);
    setOperationMessage(`已切换到 ${provider.displayName}`);
  };

  return (
    <section className="chat-layout">
      <aside className="chat-history">
        <button
          className="primary-action chat-new"
          disabled={answering}
          onClick={() => void createThread()}
          type="button"
        >
          <Plus aria-hidden="true" />
          新建对话
        </button>
        <label className="search-field">
          <MagnifyingGlass aria-hidden="true" />
          <input
            onChange={(event) => setSearch(event.target.value)}
            placeholder="搜索本地对话"
            value={search}
          />
        </label>
        <div className="list-heading">
          <span>本地历史</span>
          <small>{filteredThreads.length}</small>
        </div>
        <div className="thread-list">
          {filteredThreads.map((thread) => (
            <div
              className={`thread-row ${
                thread.id === activeThreadId ? "active" : ""
              }`}
              key={thread.id}
            >
              {editingThreadId === thread.id ? (
                <div className="thread-edit">
                  <input
                    aria-label="编辑对话名称"
                    autoFocus
                    maxLength={200}
                    onChange={(event) => setEditingTitle(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        void saveThreadTitle(thread.id);
                      }
                      if (event.key === "Escape") cancelRenameThread();
                    }}
                    value={editingTitle}
                  />
                  <button
                    aria-label="保存对话名称"
                    disabled={answering || threadActionId === thread.id}
                    onClick={() => void saveThreadTitle(thread.id)}
                    type="button"
                  >
                    <Check aria-hidden="true" />
                  </button>
                  <button
                    aria-label="取消编辑"
                    disabled={threadActionId === thread.id}
                    onClick={cancelRenameThread}
                    type="button"
                  >
                    <X aria-hidden="true" />
                  </button>
                </div>
              ) : (
                <>
                  <button
                    className="thread-select"
                    onClick={() => void selectThread(thread.id)}
                    type="button"
                  >
                    <strong>{thread.title}</strong>
                    <span>{thread.preview}</span>
                    <small>{thread.model}</small>
                  </button>
                  {deletingThreadId === thread.id ? (
                    <div className="thread-delete-confirm">
                      <button
                        disabled={answering || threadActionId === thread.id}
                        onClick={() => void removeThread(thread.id)}
                        type="button"
                      >
                        确认删除
                      </button>
                      <button
                        disabled={threadActionId === thread.id}
                        onClick={() => setDeletingThreadId("")}
                        type="button"
                      >
                        取消
                      </button>
                    </div>
                  ) : (
                    <div className="thread-actions">
                      <button
                        aria-label={`重命名 ${thread.title}`}
                        disabled={answering || Boolean(threadActionId)}
                        onClick={() => beginRenameThread(thread)}
                        type="button"
                      >
                        <PencilSimple aria-hidden="true" />
                      </button>
                      <button
                        aria-label={`删除 ${thread.title}`}
                        disabled={answering || Boolean(threadActionId)}
                        onClick={() => {
                          setEditingThreadId("");
                          setDeletingThreadId(thread.id);
                        }}
                        type="button"
                      >
                        <Trash aria-hidden="true" />
                      </button>
                    </div>
                  )}
                </>
              )}
            </div>
          ))}
        </div>
        <div className="portable-note">
          <CheckCircle aria-hidden="true" weight="fill" />
          <span>对话将保存在移动盘</span>
        </div>
      </aside>

      <div className="chat-main">
        <header className="chat-toolbar">
          <div>
            <strong>{activeThread?.title ?? "新的对话"}</strong>
            <small>
              {activeProvider?.displayName ?? "等待配置供应商"} · 对话数据保存在移动盘
            </small>
          </div>
          <label>
            <span>模型</span>
            <select
              aria-label="当前对话模型"
              disabled={!activeThread || !activeProvider || answering}
              onChange={(event) => void changeModel(event.target.value)}
              value={activeThread?.model ?? "等待供应商配置"}
            >
              {!activeProvider ? (
                <option>等待供应商配置</option>
              ) : (
                Array.from(
                  new Set([
                    activeThread?.model ?? activeProvider.defaultModel,
                    ...activeProvider.models,
                  ]),
                ).map((model) => (
                  <option key={model} value={model}>
                    {model}
                  </option>
                ))
              )}
            </select>
          </label>
          <button
            className="secondary-action"
            onClick={() => setProviderOpen(true)}
            type="button"
          >
            <GearSix aria-hidden="true" />
            配置供应商
          </button>
        </header>

        <div className="conversation">
          {loading ? (
            <div className="chat-empty">
              <SpinnerGap className="spin" aria-hidden="true" />
              <h2>正在读取移动盘对话</h2>
              <p>历史记录只从当前工作站的数据目录读取。</p>
            </div>
          ) : messages.length === 0 ? (
            <div className="chat-empty">
              <span className="chat-empty-icon">
                <ChatCircleDots aria-hidden="true" />
              </span>
              <h2>今天想一起完成什么？</h2>
              <p>可以继续已有任务，也可以从一个新想法开始。</p>
              <div className="prompt-suggestions">
                {["整理一个项目方案", "解释一段代码", "优化产品文案"].map(
                  (suggestion) => (
                    <button
                      key={suggestion}
                      onClick={() => setMessage(suggestion)}
                      type="button"
                    >
                      {suggestion}
                    </button>
                  ),
                )}
              </div>
            </div>
          ) : (
            <div className="message-stack">
              {messages.map((item) =>
                item.role === "user" ? (
                  <div className="message message--user" key={item.id}>
                    <MessageAttachments attachments={item.attachments} />
                    {item.content ? <span>{item.content}</span> : null}
                  </div>
                ) : (
                  <div className="message message--assistant" key={item.id}>
                    <span className="assistant-avatar">
                      <Sparkle aria-hidden="true" weight="fill" />
                    </span>
                    <div>
                      <p>{item.content}</p>
                    </div>
                  </div>
                ),
              )}
              {answering ? (
                <div className="message message--assistant">
                  <span className="assistant-avatar">
                    <Sparkle aria-hidden="true" weight="fill" />
                  </span>
                  <span className="typing">
                    <i />
                    <i />
                    <i />
                  </span>
                </div>
              ) : null}
            </div>
          )}
        </div>

        <form className="composer" onSubmit={sendMessage}>
          {operationMessage ? (
            <div className="inline-operation-message">{operationMessage}</div>
          ) : null}
          {pendingAttachments.length ? (
            <div className="chat-attachment-list" aria-label="待发送附件">
              {pendingAttachments.map((attachment) => (
                <span className="chat-attachment-chip" key={attachment.id}>
                  <strong>{attachment.fileName}</strong>
                  <small>{formatFileSize(attachment.sizeBytes)}</small>
                  <button
                    aria-label={`移除 ${attachment.fileName}`}
                    disabled={answering}
                    onClick={() => void discardAttachment(attachment.id)}
                    type="button"
                  >
                    <X aria-hidden="true" />
                  </button>
                </span>
              ))}
            </div>
          ) : null}
          <textarea
            aria-label="输入消息"
            onChange={(event) => setMessage(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                event.currentTarget.form?.requestSubmit();
              }
            }}
            placeholder="给 AI 发送消息"
            rows={3}
            value={message}
          />
          <div className="composer-actions">
            <button
              className="icon-button"
              disabled={answering || pendingAttachments.length >= 5}
              onClick={() => void addAttachments()}
              title="添加文件"
              type="button"
            >
              <Plus aria-hidden="true" />
            </button>
            <span>Enter 发送，Shift + Enter 换行</span>
            <button
              className="send-button"
              disabled={
                (!message.trim() && pendingAttachments.length === 0) || answering
              }
              type="submit"
            >
              <PaperPlaneTilt aria-hidden="true" weight="fill" />
            </button>
          </div>
        </form>

        {providerOpen ? (
          <ChatProviderSetup
            activeThread={activeThread}
            onApplied={(state, provider) =>
              void applyProvider(state, provider).catch((error) => {
                setOperationMessage(errorMessage(error));
              })
            }
            onClose={() => setProviderOpen(false)}
            state={providerState}
          />
        ) : null}
      </div>
    </section>
  );
}

function MessageAttachments({ attachments }: { attachments: ChatAttachment[] }) {
  if (!attachments.length) return null;
  return (
    <span className="message-attachments">
      {attachments.map((attachment) => (
        <span key={attachment.id}>
          <FileMd aria-hidden="true" />
          <span>{attachment.fileName}</span>
        </span>
      ))}
    </span>
  );
}

type ChatProviderDraft = {
  id?: string;
  displayName: string;
  baseUrl: string;
  apiKey: string;
  defaultModel: string;
  models: string[];
  protocol: ChatProviderProtocol;
  credentialPresent: boolean;
};

const emptyChatProviderDraft = (): ChatProviderDraft => ({
  displayName: "",
  baseUrl: DEFAULT_PROVIDER_BASE_URL,
  apiKey: "",
  defaultModel: "",
  models: [],
  protocol: "chat-completions",
  credentialPresent: false,
});

function chatProviderDraft(provider: ChatProviderView): ChatProviderDraft {
  return {
    id: provider.id,
    displayName: provider.displayName,
    baseUrl: provider.baseUrl,
    apiKey: "",
    defaultModel: provider.defaultModel,
    models: provider.models,
    protocol: provider.protocol,
    credentialPresent: provider.credentialPresent,
  };
}

function ChatProviderSetup({
  activeThread,
  onApplied,
  onClose,
  state,
}: {
  activeThread?: ChatThreadSummary;
  onApplied: (state: ChatProviderState, provider: ChatProviderView) => void;
  onClose: () => void;
  state: ChatProviderState;
}) {
  const initialProvider =
    state.providers.find(
      (provider) => provider.id === activeThread?.providerId,
    ) ??
    state.providers.find((provider) => provider.id === state.activeProviderId) ??
    state.providers[0];
  const [draft, setDraft] = useState<ChatProviderDraft>(() =>
    initialProvider
      ? chatProviderDraft(initialProvider)
      : emptyChatProviderDraft(),
  );
  const [editingKey, setEditingKey] = useState(false);
  const [showKey, setShowKey] = useState(false);
  const [working, setWorking] = useState<"save" | "models" | "test" | "">("");
  const [status, setStatus] = useState("");

  const selectProvider = (providerId: string) => {
    const provider = state.providers.find((item) => item.id === providerId);
    setDraft(
      provider ? chatProviderDraft(provider) : emptyChatProviderDraft(),
    );
    setEditingKey(false);
    setShowKey(false);
    setStatus("");
  };

  const savedCredentialMasked = isSavedCredentialMasked(
    draft.apiKey,
    draft.credentialPresent,
    editingKey,
  );
  const displayedApiKey = credentialDisplayValue(
    draft.apiKey,
    draft.credentialPresent,
    editingKey,
  );

  const discover = async () => {
    if (!draft.baseUrl.trim() || (!draft.apiKey && !draft.credentialPresent)) {
      setStatus("请先填写 API 地址和 API Key");
      return;
    }
    setWorking("models");
    setStatus("");
    try {
      const result = await discoverChatProviderModels({
        providerId: draft.id,
        baseUrl: draft.baseUrl,
        apiKey: draft.apiKey || undefined,
        protocol: draft.protocol,
      });
      setDraft((current) => ({
        ...current,
        models: result.models,
        defaultModel: result.models.includes(current.defaultModel)
          ? current.defaultModel
          : result.models[0],
      }));
      setStatus(`已获取 ${result.models.length} 个模型，耗时 ${result.elapsedMs}ms`);
    } catch (error) {
      setStatus(errorMessage(error));
    } finally {
      setWorking("");
    }
  };

  const save = async () => {
    setWorking("save");
    setStatus("");
    try {
      const nextState = await saveChatProvider({
        id: draft.id,
        displayName: draft.displayName,
        baseUrl: draft.baseUrl,
        apiKey: draft.apiKey || undefined,
        defaultModel: draft.defaultModel,
        models: draft.models,
        protocol: draft.protocol,
      });
      const provider = nextState.providers.find(
        (item) => item.id === nextState.activeProviderId,
      );
      if (!provider) throw new Error("供应商已保存，但无法读取当前配置");
      onApplied(nextState, provider);
    } catch (error) {
      setStatus(errorMessage(error));
    } finally {
      setWorking("");
    }
  };

  const test = async () => {
    if (!draft.id) {
      setStatus("请先保存供应商，再测试连接");
      return;
    }
    setWorking("test");
    setStatus("");
    try {
      await testChatProvider(draft.id);
      setStatus("连接成功，对话模型已正常回应");
    } catch (error) {
      setStatus(errorMessage(error));
    } finally {
      setWorking("");
    }
  };

  return (
    <section className="chat-provider-overlay" aria-label="AI 对话供应商配置">
      <div className="chat-provider-dialog content-card">
        <header className="provider-workspace-header">
          <div>
            <h2>AI 对话供应商</h2>
            <p>此配置只用于 AI 对话。</p>
          </div>
          <button
            className="icon-button"
            disabled={!state.configured}
            onClick={onClose}
            title={state.configured ? "返回对话" : "请先完成首次配置"}
            type="button"
          >
            <X aria-hidden="true" />
          </button>
        </header>

        <div className="chat-provider-body">
          <div className="chat-provider-selector">
            <label className="form-field">
              <span>已保存的供应商</span>
              <select
                onChange={(event) => selectProvider(event.target.value)}
                value={draft.id ?? "new"}
              >
                {state.providers.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.displayName}
                  </option>
                ))}
                <option value="new">+ 新增供应商</option>
              </select>
            </label>
            <span>
              {activeThread
                ? `保存后会切换当前对话：${activeThread.title}`
                : "保存后新对话将默认使用此供应商"}
            </span>
          </div>

          <div className="provider-form provider-form--inline chat-provider-form">
            <label className="form-field">
              <span>供应商名称</span>
              <input
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    displayName: event.target.value,
                  }))
                }
                placeholder="例如 云枢 API"
                value={draft.displayName}
              />
            </label>
            <label className="form-field">
              <span>接口协议</span>
              <select
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    protocol: event.target.value as ChatProviderProtocol,
                  }))
                }
                value={draft.protocol}
              >
                <option value="auto">自动识别</option>
                <option value="responses">OpenAI Responses</option>
                <option value="chat-completions">OpenAI Chat Completions</option>
                <option value="anthropic-messages">Anthropic Messages</option>
              </select>
            </label>
            <label className="form-field provider-field--wide">
              <span>API 地址</span>
              <input
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    baseUrl: event.target.value,
                    models: [],
                  }))
                }
                placeholder="https://example.com/v1"
                spellCheck={false}
                value={draft.baseUrl}
              />
            </label>
            <label className="form-field provider-field--wide">
              <span>API Key</span>
              <div
                className={`secret-input ${savedCredentialMasked ? "secret-input--saved" : ""}`}
              >
                <input
                  onBlur={() => {
                    if (!draft.apiKey) setEditingKey(false);
                  }}
                  onChange={(event) => {
                    setEditingKey(true);
                    setDraft((current) => ({
                      ...current,
                      apiKey: event.target.value,
                    }));
                  }}
                  onFocus={() => {
                    if (savedCredentialMasked) {
                      setEditingKey(true);
                      setShowKey(false);
                    }
                  }}
                  placeholder={draft.credentialPresent ? "输入新密钥" : "粘贴密钥"}
                  readOnly={savedCredentialMasked}
                  spellCheck={false}
                  type={savedCredentialMasked || showKey ? "text" : "password"}
                  value={displayedApiKey}
                />
                <button
                  disabled={savedCredentialMasked}
                  onClick={() => setShowKey((current) => !current)}
                  type="button"
                >
                  {savedCredentialMasked ? (
                    <ShieldCheck aria-hidden="true" weight="fill" />
                  ) : showKey ? (
                    <EyeSlash aria-hidden="true" />
                  ) : (
                    <Eye aria-hidden="true" />
                  )}
                </button>
              </div>
              <small>
                {draft.credentialPresent
                  ? "密钥已保存在移动盘加密存储中"
                  : "首次保存必须填写 API Key"}
              </small>
            </label>
            <div className="form-field provider-field--wide">
              <span>默认模型</span>
              <div className="model-picker-row">
                {draft.models.length ? (
                  <select
                    onChange={(event) =>
                      setDraft((current) => ({
                        ...current,
                        defaultModel: event.target.value,
                      }))
                    }
                    value={draft.defaultModel}
                  >
                    {draft.models.map((model) => (
                      <option key={model} value={model}>
                        {model}
                      </option>
                    ))}
                  </select>
                ) : (
                  <input
                    onChange={(event) =>
                      setDraft((current) => ({
                        ...current,
                        defaultModel: event.target.value,
                      }))
                    }
                    placeholder="输入模型名称"
                    value={draft.defaultModel}
                  />
                )}
                <button
                  className="model-fetch-button"
                  disabled={working !== ""}
                  onClick={() => void discover()}
                  type="button"
                >
                  {working === "models" ? (
                    <SpinnerGap className="spin" aria-hidden="true" />
                  ) : (
                    <ArrowsClockwise aria-hidden="true" />
                  )}
                  自动获取模型
                </button>
              </div>
            </div>
          </div>

          {status ? <div className="inline-operation-message">{status}</div> : null}
          <footer className="chat-provider-actions">
            <button
              className="secondary-action"
              disabled={!draft.id || working !== ""}
              onClick={() => void test()}
              type="button"
            >
              {working === "test" ? (
                <SpinnerGap className="spin" aria-hidden="true" />
              ) : (
                <Lightning aria-hidden="true" />
              )}
              测试连接
            </button>
            <button
              className="primary-action"
              disabled={
                working !== "" ||
                !draft.displayName.trim() ||
                !draft.baseUrl.trim() ||
                !draft.defaultModel.trim() ||
                (!draft.apiKey.trim() && !draft.credentialPresent)
              }
              onClick={() => void save()}
              type="button"
            >
              {working === "save" ? (
                <SpinnerGap className="spin" aria-hidden="true" />
              ) : (
                <Check aria-hidden="true" />
              )}
              保存配置并返回对话
            </button>
          </footer>
        </div>
      </div>
    </section>
  );
}

function formatFileSize(sizeBytes: number): string {
  if (sizeBytes < 1024) return `${sizeBytes} B`;
  if (sizeBytes < 1024 * 1024) return `${(sizeBytes / 1024).toFixed(1)} KB`;
  return `${(sizeBytes / (1024 * 1024)).toFixed(1)} MB`;
}

function ImagePage({ engine: _engine }: { engine: Engine }) {
  const referenceInputRef = useRef<HTMLInputElement>(null);
  const [prompt, setPrompt] = useState("");
  const [negativePrompt, setNegativePrompt] = useState("");
  const [mode, setMode] = useState<"generation" | "edit">("generation");
  const [generating, setGenerating] = useState(false);
  const [activeJobId, setActiveJobId] = useState("");
  const [ratio, setRatio] = useState("1:1");
  const [quality, setQuality] = useState("standard");
  const [count, setCount] = useState(1);
  const [referenceDataUrl, setReferenceDataUrl] = useState("");
  const [referenceName, setReferenceName] = useState("");
  const [providerState, setProviderState] = useState<ImageProviderState>({
    configured: false,
    providers: [],
  });
  const [providerOpen, setProviderOpen] = useState(false);
  const [asset, setAsset] = useState<ImageAsset | null>(null);
  const [selectedJobId, setSelectedJobId] = useState("");
  const [history, setHistory] = useState<ImageJob[]>([]);
  const [partialPreview, setPartialPreview] = useState("");
  const [operationMessage, setOperationMessage] = useState("");

  const activeProvider = providerState.providers.find(
    (provider) => provider.id === providerState.activeProviderId,
  );
  const selectedJob = history.find((job) => job.id === selectedJobId);

  useEffect(() => {
    let cancelled = false;
    const hydrate = async () => {
      setAsset(null);
      setOperationMessage("");
      try {
        const bootstrap = await initializeImageStudio();
        if (cancelled) return;
        setProviderState(bootstrap.providerState);
        setHistory(bootstrap.jobs);
        setSelectedJobId(bootstrap.jobs[0]?.id ?? "");
        setProviderOpen(!bootstrap.providerState.configured);
        if (!bootstrap.providerState.configured) {
          setOperationMessage("首次使用请先配置 AI 生图供应商");
        }
      } catch (error) {
        if (!cancelled) setOperationMessage(errorMessage(error));
      }
    };
    void hydrate();
    return () => {
      cancelled = true;
    };
  }, []);

  const upsertJob = (job: ImageJob) => {
    setHistory((current) => [
      job,
      ...current.filter((item) => item.id !== job.id),
    ]);
    setSelectedJobId(job.id);
  };

  const showFirstAsset = async (job: ImageJob) => {
    const first = job.assets[0];
    if (!first) {
      setAsset(null);
      return;
    }
    setAsset(await readImageAsset(first.id));
  };

  const handleGenerationEvent = (event: ImageGenerationEvent) => {
    if (event.event === "started") {
      setActiveJobId(event.data.jobId);
      setOperationMessage("图片任务已提交");
    } else if (event.event === "progress") {
      setOperationMessage(event.data.message);
    } else if (event.event === "preview") {
      setPartialPreview(event.data.imageDataUrl);
      setOperationMessage("正在接收流式预览…");
    } else if (event.event === "completed") {
      upsertJob(event.data.job);
      setOperationMessage("生成完成，原图已保存到移动盘");
    } else if (event.event === "cancelled") {
      setOperationMessage("图片生成已取消");
    } else if (event.event === "failed") {
      setOperationMessage(event.data.message);
    }
  };

  const generate = async (event: FormEvent) => {
    event.preventDefault();
    if (!prompt.trim() || generating) return;
    if (!activeProvider) {
      setProviderOpen(true);
      return;
    }
    if (mode === "edit" && !referenceDataUrl) {
      setOperationMessage("图生图需要先添加参考图片");
      return;
    }
    if (!isTauriRuntime()) {
      setOperationMessage("请在桌面工作站中使用真实图片生成服务");
      return;
    }
    setGenerating(true);
    setPartialPreview("");
    setAsset(null);
    setOperationMessage("正在准备图片请求…");
    try {
      const generated = await generateImage(
        {
          providerId: activeProvider.id,
          mode,
          prompt: prompt.trim(),
          negativePrompt,
          size: ratio,
          quality,
          count,
          referenceDataUrl: mode === "edit" ? referenceDataUrl : undefined,
        },
        handleGenerationEvent,
      );
      upsertJob(generated);
      await showFirstAsset(generated);
      setOperationMessage("图片已保存到移动工作站");
    } catch (error) {
      setOperationMessage(errorMessage(error));
      try {
        setHistory(await listImageHistory());
      } catch {
        // 保留原历史，不用二次错误覆盖真实生图错误。
      }
    } finally {
      setGenerating(false);
      setActiveJobId("");
      setPartialPreview("");
    }
  };

  const cancelGeneration = async () => {
    if (!activeJobId) return;
    try {
      const accepted = await cancelImageGeneration(activeJobId);
      setOperationMessage(
        accepted ? "正在取消图片任务…" : "图片任务已经结束",
      );
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  const chooseReference = async (file?: File) => {
    if (!file) return;
    if (!["image/png", "image/jpeg", "image/webp"].includes(file.type)) {
      setOperationMessage("参考图片仅支持 PNG、JPEG 和 WebP");
      return;
    }
    if (file.size > 20 * 1024 * 1024) {
      setOperationMessage("参考图片不能超过 20 MB");
      return;
    }
    try {
      setReferenceDataUrl(await imageFileToDataUrl(file));
      setReferenceName(file.name);
      setMode("edit");
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  const openHistoryAsset = async (job: ImageJob) => {
    setSelectedJobId(job.id);
    setOperationMessage("");
    try {
      await showFirstAsset(job);
      if (!job.assets.length) {
        setOperationMessage(job.errorMessage ?? "这条任务没有可显示的图片");
      }
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  const openJobAsset = async (assetId: string) => {
    try {
      setAsset(await readImageAsset(assetId));
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  const repeatSelectedJob = async () => {
    if (!selectedJob) return;
    setPrompt(selectedJob.prompt);
    setNegativePrompt(selectedJob.negativePrompt);
    setMode(selectedJob.mode);
    setRatio(selectedJob.size);
    setQuality(selectedJob.quality);
    setCount(selectedJob.requestedCount);
    if (selectedJob.mode === "edit") {
      try {
        const reference = await readImageReference(selectedJob.id);
        setReferenceDataUrl(reference?.dataUrl ?? "");
        setReferenceName(reference?.fileName ?? "");
      } catch (error) {
        setOperationMessage(errorMessage(error));
      }
    }
    setOperationMessage("已恢复该任务的生成参数");
  };

  const exportCurrent = async () => {
    if (!asset) return;
    try {
      const path = await exportImageAsset(asset.assetId);
      setOperationMessage(`已导出到 ${path}`);
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  const clearHistory = async () => {
    try {
      if (isTauriRuntime()) await clearImageHistory();
      setHistory([]);
      setSelectedJobId("");
      setAsset(null);
      setOperationMessage("生图历史已清空");
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  const switchProvider = async (providerId: string) => {
    try {
      setProviderState(await setActiveImageProvider(providerId));
      setOperationMessage("已切换 AI 生图供应商");
    } catch (error) {
      setOperationMessage(errorMessage(error));
    }
  };

  return (
    <>
      <section className="image-layout">
        <form className="image-controls content-card" onSubmit={generate}>
          <header className="image-panel-heading">
            <h2>AI 生图</h2>
            <div className="image-heading-actions">
              <select
                aria-label="快速切换图片供应商"
                disabled={generating || !providerState.providers.length}
                onChange={(event) => void switchProvider(event.target.value)}
                value={providerState.activeProviderId ?? ""}
              >
                {!providerState.providers.length ? <option value="">未配置</option> : null}
                {providerState.providers.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.displayName}
                  </option>
                ))}
              </select>
              <button
                className="icon-button"
                onClick={() => setProviderOpen(true)}
                title="配置图片供应商"
                type="button"
              >
                <GearSix aria-hidden="true" />
              </button>
            </div>
          </header>
          <label className="form-field">
            <span>图片供应商</span>
            <select
              aria-label="当前图片供应商"
              disabled={generating || !providerState.providers.length}
              onChange={(event) => void switchProvider(event.target.value)}
              value={providerState.activeProviderId ?? ""}
            >
              {!providerState.providers.length ? <option value="">等待配置</option> : null}
              {providerState.providers.map((provider) => (
                <option key={provider.id} value={provider.id}>
                  {provider.displayName} · {provider.defaultModel}
                </option>
              ))}
            </select>
            <small>AI 生图使用独立供应商，不改动三引擎配置</small>
          </label>
          <div className="image-option-grid">
            <label className="form-field">
              <span>生成模式</span>
              <select
                disabled={generating}
                onChange={(event) => setMode(event.target.value as "generation" | "edit")}
                value={mode}
              >
                <option value="generation">文生图</option>
                <option value="edit">图生图</option>
              </select>
            </label>
            <label className="form-field">
              <span>生成数量</span>
              <select
                disabled={generating}
                onChange={(event) => setCount(Number(event.target.value))}
                value={count}
              >
                {[1, 2, 3, 4].map((value) => <option key={value}>{value}</option>)}
              </select>
            </label>
          </div>
          {mode === "edit" ? (
            <div className="image-reference-control">
              <input
                accept="image/png,image/jpeg,image/webp"
                hidden
                onChange={(event) => {
                  void chooseReference(event.currentTarget.files?.[0]);
                  event.currentTarget.value = "";
                }}
                ref={referenceInputRef}
                type="file"
              />
              <button
                className="secondary-action"
                disabled={generating}
                onClick={() => referenceInputRef.current?.click()}
                type="button"
              >
                <ImageSquare aria-hidden="true" />
                {referenceName || "选择参考图片"}
              </button>
              {referenceDataUrl ? (
                <button
                  className="icon-button"
                  onClick={() => {
                    setReferenceDataUrl("");
                    setReferenceName("");
                  }}
                  title="移除参考图片"
                  type="button"
                >
                  <X aria-hidden="true" />
                </button>
              ) : null}
            </div>
          ) : null}
          <label className="form-field">
            <span>图片描述</span>
            <textarea
              disabled={generating}
              maxLength={8000}
              onChange={(event) => setPrompt(event.target.value)}
              placeholder="描述画面主体、风格、构图和光线"
              rows={7}
              value={prompt}
            />
            <small>{prompt.length} / 8000</small>
          </label>
          <details className="image-advanced-options">
            <summary>更多生成选项</summary>
            <label className="form-field">
              <span>排除内容（可选）</span>
              <textarea
                disabled={generating}
                maxLength={8000}
                onChange={(event) => setNegativePrompt(event.target.value)}
                placeholder="例如：文字、水印、模糊"
                rows={3}
                value={negativePrompt}
              />
            </label>
          </details>
          <div className="image-option-grid">
            <label className="form-field">
              <span>比例</span>
              <select disabled={generating} onChange={(event) => setRatio(event.target.value)} value={ratio}>
                <option>1:1</option>
                <option>4:3</option>
                <option>16:9</option>
                <option>9:16</option>
              </select>
            </label>
            <label className="form-field">
              <span>质量</span>
              <select disabled={generating} onChange={(event) => setQuality(event.target.value)} value={quality}>
                <option value="standard">标准</option>
                <option value="high">高清</option>
              </select>
            </label>
          </div>
          <button
            className={generating ? "danger-action generate-button" : "primary-action generate-button"}
            disabled={!generating && (!prompt.trim() || !activeProvider)}
            onClick={generating ? () => void cancelGeneration() : undefined}
            type={generating ? "button" : "submit"}
          >
            {generating ? <Stop aria-hidden="true" weight="fill" /> : <Sparkle aria-hidden="true" weight="fill" />}
            {generating ? "停止生成" : "开始生成"}
          </button>
          {operationMessage ? <div className="inline-operation-message">{operationMessage}</div> : null}
        </form>

        <div className="image-result content-card">
          <header className="card-header">
            <h2>作品预览</h2>
            <div className="image-result-actions">
              {selectedJob ? (
                <button className="quiet-link" onClick={() => void repeatSelectedJob()} type="button">
                  再次创作
                </button>
              ) : null}
              {asset ? (
                <button className="icon-button" onClick={() => void exportCurrent()} title="导出图片" type="button">
                  <DownloadSimple aria-hidden="true" />
                </button>
              ) : null}
            </div>
          </header>
          <div className={`image-canvas ${generating ? "loading" : ""} ${asset || partialPreview ? "has-image" : ""}`}>
            {partialPreview ? (
              <>
                <img alt="生成中的预览" src={partialPreview} />
                <span className="preview-badge">流式预览 · 生成中</span>
              </>
            ) : generating ? (
              <div className="generation-progress">
                <SpinnerGap className="spin" aria-hidden="true" />
                <strong>正在生成图片</strong>
                <span>{operationMessage || "正在准备预览"}</span>
              </div>
            ) : asset ? (
              <img alt={selectedJob?.prompt ?? "AI 生图作品"} src={asset.dataUrl} />
            ) : (
              <div className="image-empty">
                <ImageSquare aria-hidden="true" />
                <strong>图片会显示在这里</strong>
                <span>填写描述后点击开始生成</span>
              </div>
            )}
          </div>
          {selectedJob && selectedJob.assets.length > 1 ? (
            <div className="image-asset-strip">
              {selectedJob.assets.map((item, index) => (
                <button key={item.id} onClick={() => void openJobAsset(item.id)} title={`查看第 ${index + 1} 张`} type="button">
                  {selectedJob.assetPreviews[item.id] ? <img alt="" src={selectedJob.assetPreviews[item.id]} /> : <ImageSquare aria-hidden="true" />}
                </button>
              ))}
            </div>
          ) : null}
          <div className="image-result-meta">
            <span>
              {selectedJob
                ? `${selectedJob.providerName} · ${selectedJob.model} · ${selectedJob.size}`
                : "生成内容将直接保存到移动盘"}
            </span>
            <button className="quiet-link" onClick={() => void openImageOutputDirectory().catch((error) => setOperationMessage(errorMessage(error)))} type="button">
              打开作品目录
              <FolderOpen aria-hidden="true" />
            </button>
          </div>
        </div>

        <aside className="image-history content-card">
          <header className="card-header">
            <h2>历史记录</h2>
            {history.length ? (
              <button className="icon-button" onClick={() => void clearHistory()} title="清空记录" type="button">
                <Trash aria-hidden="true" />
              </button>
            ) : null}
          </header>
          {history.length ? (
            <div className="image-history-list">
              {history.map((item) => {
                const first = item.assets[0];
                return (
                  <button key={item.id} onClick={() => void openHistoryAsset(item)} type="button">
                    <span className="history-thumb">
                      {first && item.assetPreviews[first.id] ? <img alt="" src={item.assetPreviews[first.id]} /> : item.status === "running" ? <SpinnerGap className="spin" aria-hidden="true" /> : <ImageSquare aria-hidden="true" />}
                    </span>
                    <span>
                      <strong>{item.prompt}</strong>
                      <small>{item.providerName} · {new Date(item.updatedAtMs).toLocaleString("zh-CN")}</small>
                    </span>
                  </button>
                );
              })}
            </div>
          ) : (
            <div className="small-empty">
              <ImageSquare aria-hidden="true" />
              <span>暂无生成记录</span>
            </div>
          )}
        </aside>
      </section>

      {providerOpen ? (
        <ImageProviderSetup
          onApplied={(state) => {
            setProviderState(state);
            setProviderOpen(false);
            setOperationMessage("AI 生图供应商已保存");
          }}
          onClose={() => setProviderOpen(false)}
          state={providerState}
        />
      ) : null}
    </>
  );
}

type ImageProviderDraft = {
  id?: string;
  displayName: string;
  baseUrl: string;
  apiKey: string;
  defaultModel: string;
  models: string[];
  credentialPresent: boolean;
  streaming: boolean;
};

const emptyImageProviderDraft = (): ImageProviderDraft => ({
  displayName: "",
  baseUrl: DEFAULT_PROVIDER_BASE_URL,
  apiKey: "",
  defaultModel: "",
  models: [],
  credentialPresent: false,
  streaming: false,
});

function imageProviderDraft(provider: ImageProviderView): ImageProviderDraft {
  return {
    id: provider.id,
    displayName: provider.displayName,
    baseUrl: provider.baseUrl,
    apiKey: "",
    defaultModel: provider.defaultModel,
    models: provider.models,
    credentialPresent: provider.credentialPresent,
    streaming: provider.streaming,
  };
}

function ImageProviderSetup({
  onApplied,
  onClose,
  state,
}: {
  onApplied: (state: ImageProviderState) => void;
  onClose: () => void;
  state: ImageProviderState;
}) {
  const initialProvider =
    state.providers.find((provider) => provider.id === state.activeProviderId) ??
    state.providers[0];
  const [draft, setDraft] = useState<ImageProviderDraft>(() =>
    initialProvider ? imageProviderDraft(initialProvider) : emptyImageProviderDraft(),
  );
  const [editingKey, setEditingKey] = useState(false);
  const [showKey, setShowKey] = useState(false);
  const [working, setWorking] = useState<"save" | "models" | "test" | "">("");
  const [status, setStatus] = useState("");

  const savedCredentialMasked = isSavedCredentialMasked(
    draft.apiKey,
    draft.credentialPresent,
    editingKey,
  );
  const displayedApiKey = credentialDisplayValue(
    draft.apiKey,
    draft.credentialPresent,
    editingKey,
  );

  const selectProvider = (providerId: string) => {
    const provider = state.providers.find((item) => item.id === providerId);
    setDraft(provider ? imageProviderDraft(provider) : emptyImageProviderDraft());
    setEditingKey(false);
    setShowKey(false);
    setStatus("");
  };

  const discover = async () => {
    if (!draft.baseUrl.trim() || (!draft.apiKey && !draft.credentialPresent)) {
      setStatus("请先填写 API 地址和 API Key");
      return;
    }
    setWorking("models");
    setStatus("");
    try {
      const result = await discoverImageProviderModels({
        providerId: draft.id,
        baseUrl: draft.baseUrl,
        apiKey: draft.apiKey || undefined,
      });
      setDraft((current) => ({
        ...current,
        models: result.models,
        defaultModel: result.models.includes(current.defaultModel)
          ? current.defaultModel
          : result.models[0],
      }));
      setStatus(`已获取 ${result.models.length} 个模型，耗时 ${result.elapsedMs}ms`);
    } catch (error) {
      setStatus(errorMessage(error));
    } finally {
      setWorking("");
    }
  };

  const save = async () => {
    setWorking("save");
    setStatus("");
    try {
      const nextState = await saveImageProvider({
        id: draft.id,
        displayName: draft.displayName,
        baseUrl: draft.baseUrl,
        apiKey: draft.apiKey || undefined,
        defaultModel: draft.defaultModel,
        models: draft.models,
        streaming: draft.streaming,
      });
      onApplied(nextState);
    } catch (error) {
      setStatus(errorMessage(error));
    } finally {
      setWorking("");
    }
  };

  const test = async () => {
    if (!draft.id) {
      setStatus("请先保存供应商，再测试连接");
      return;
    }
    setWorking("test");
    setStatus("");
    try {
      const result = await testImageProvider(draft.id);
      setStatus(result.message);
    } catch (error) {
      setStatus(errorMessage(error));
    } finally {
      setWorking("");
    }
  };

  return (
    <aside
      aria-label="AI 生图供应商配置"
      aria-modal="false"
      className="image-provider-drawer"
      role="dialog"
    >
      <div className="image-provider-panel content-card">
        <header className="provider-workspace-header">
          <div>
            <h2>AI 生图供应商</h2>
            <p>此配置只用于 AI 生图。</p>
          </div>
          <button
            className="icon-button"
            onClick={onClose}
            title="关闭供应商配置"
            type="button"
          >
            <X aria-hidden="true" />
          </button>
        </header>
        <div className="chat-provider-body">
          <div className="chat-provider-selector">
            <label className="form-field">
              <span>已保存的供应商</span>
              <select onChange={(event) => selectProvider(event.target.value)} value={draft.id ?? "new"}>
                {state.providers.map((provider) => (
                  <option key={provider.id} value={provider.id}>{provider.displayName}</option>
                ))}
                <option value="new">+ 新增供应商</option>
              </select>
            </label>
            <span>支持 OpenAI Compatible Images API；保存后可在页面右上角切换。</span>
          </div>
          <div className="provider-form provider-form--inline chat-provider-form">
            <label className="form-field">
              <span>供应商名称</span>
              <input onChange={(event) => setDraft((current) => ({ ...current, displayName: event.target.value }))} placeholder="例如 云枢生图" value={draft.displayName} />
            </label>
            <label className="form-field">
              <span>流式预览</span>
              <select onChange={(event) => setDraft((current) => ({ ...current, streaming: event.target.value === "true" }))} value={String(draft.streaming)}>
                <option value="false">关闭（兼容性更好）</option>
                <option value="true">开启</option>
              </select>
            </label>
            <label className="form-field provider-field--wide">
              <span>API 地址</span>
              <input onChange={(event) => setDraft((current) => ({ ...current, baseUrl: event.target.value, models: [] }))} placeholder="https://example.com/v1" spellCheck={false} value={draft.baseUrl} />
            </label>
            <label className="form-field provider-field--wide">
              <span>API Key</span>
              <div className={`secret-input ${savedCredentialMasked ? "secret-input--saved" : ""}`}>
                <input
                  onBlur={() => { if (!draft.apiKey) setEditingKey(false); }}
                  onChange={(event) => { setEditingKey(true); setDraft((current) => ({ ...current, apiKey: event.target.value })); }}
                  onFocus={() => { if (savedCredentialMasked) { setEditingKey(true); setShowKey(false); } }}
                  placeholder={draft.credentialPresent ? "输入新密钥" : "粘贴密钥"}
                  readOnly={savedCredentialMasked}
                  spellCheck={false}
                  type={savedCredentialMasked || showKey ? "text" : "password"}
                  value={displayedApiKey}
                />
                <button disabled={savedCredentialMasked} onClick={() => setShowKey((current) => !current)} type="button">
                  {savedCredentialMasked ? <ShieldCheck aria-hidden="true" weight="fill" /> : showKey ? <EyeSlash aria-hidden="true" /> : <Eye aria-hidden="true" />}
                </button>
              </div>
              <small>{draft.credentialPresent ? "密钥已保存在移动盘加密存储中" : "首次保存必须填写 API Key"}</small>
            </label>
            <div className="form-field provider-field--wide">
              <span>图片模型</span>
              <div className="model-picker-row">
                {draft.models.length ? (
                  <select onChange={(event) => setDraft((current) => ({ ...current, defaultModel: event.target.value }))} value={draft.defaultModel}>
                    {draft.models.map((model) => <option key={model}>{model}</option>)}
                  </select>
                ) : (
                  <input onChange={(event) => setDraft((current) => ({ ...current, defaultModel: event.target.value }))} placeholder="输入图片模型名称" value={draft.defaultModel} />
                )}
                <button className="model-fetch-button" disabled={working !== ""} onClick={() => void discover()} type="button">
                  {working === "models" ? <SpinnerGap className="spin" aria-hidden="true" /> : <ArrowsClockwise aria-hidden="true" />}
                  自动获取模型
                </button>
              </div>
            </div>
          </div>
          {status ? <div className="inline-operation-message">{status}</div> : null}
          <footer className="chat-provider-actions">
            <button className="secondary-action" disabled={!draft.id || working !== ""} onClick={() => void test()} type="button">
              {working === "test" ? <SpinnerGap className="spin" aria-hidden="true" /> : <Lightning aria-hidden="true" />}
              测试连接
            </button>
            <button
              className="primary-action"
              disabled={working !== "" || !draft.displayName.trim() || !draft.baseUrl.trim() || !draft.defaultModel.trim() || (!draft.apiKey.trim() && !draft.credentialPresent)}
              onClick={() => void save()}
              type="button"
            >
              {working === "save" ? <SpinnerGap className="spin" aria-hidden="true" /> : <Check aria-hidden="true" />}
              保存配置并返回 AI 生图
            </button>
          </footer>
        </div>
      </div>
    </aside>
  );
}

function imageFileToDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") resolve(reader.result);
      else reject(new Error("无法读取参考图片"));
    };
    reader.onerror = () => reject(new Error("无法读取参考图片"));
    reader.readAsDataURL(file);
  });
}

function SkillLibraryPage({
  catalogReady,
  engine,
  installed,
  items,
  onImport,
  onInstall,
  onStateChange,
  state,
}: {
  catalogReady: boolean;
  engine: Engine;
  installed: Set<string>;
  items: SkillItem[];
  onImport: (draft: SkillImportDraft) => Promise<void>;
  onInstall: (skill: SkillItem) => Promise<void>;
  onStateChange: Dispatch<SetStateAction<SkillLibraryState>>;
  state: SkillLibraryState;
}) {
  const { query, category, view, catalogItems } = state;
  const searchRequestRef = useRef(0);
  const categoryRailRef = useRef<HTMLDivElement | null>(null);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [syncingCatalog, setSyncingCatalog] = useState(false);
  const [catalogError, setCatalogError] = useState("");
  const [choosingImport, setChoosingImport] = useState<
    "zip" | "markdown" | null
  >(null);
  const [importDraft, setImportDraft] = useState<SkillImportDraft | null>(null);
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState("");
  const [categoryScroll, setCategoryScroll] = useState({
    canScrollLeft: false,
    canScrollRight: false,
  });

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const requestId = ++searchRequestRef.current;
    if (!query.trim() && category === "全部") {
      setSyncingCatalog(false);
      setCatalogError("");
      onStateChange((current) =>
        sameSkillList(current.catalogItems, items)
          ? current
          : { ...current, catalogItems: items },
      );
      return;
    }

    let cancelled = false;
    const timer = window.setTimeout(() => {
      setSyncingCatalog(true);
      setCatalogError("");
      void searchSkills({ query, category })
        .then((skills) => {
          if (!cancelled && requestId === searchRequestRef.current) {
            onStateChange((current) => ({
              ...current,
              catalogItems: skills.map(skillCatalogToItem),
            }));
          }
        })
        .catch((error) => {
          if (!cancelled && requestId === searchRequestRef.current) {
            setCatalogError(errorMessage(error));
          }
        })
        .finally(() => {
          if (!cancelled && requestId === searchRequestRef.current) {
            setSyncingCatalog(false);
          }
        });
    }, 320);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [category, items, onStateChange, query]);

  const categories = [
    "全部",
    ...new Set([...items, ...catalogItems].map((item) => item.category)),
  ];

  useEffect(() => {
    const rail = categoryRailRef.current;
    if (!rail) return;

    const updateScrollState = () => {
      const maxScrollLeft = Math.max(0, rail.scrollWidth - rail.clientWidth);
      setCategoryScroll({
        canScrollLeft: rail.scrollLeft > 2,
        canScrollRight: rail.scrollLeft < maxScrollLeft - 2,
      });
    };

    updateScrollState();
    rail.addEventListener("scroll", updateScrollState, { passive: true });
    const resizeObserver = new ResizeObserver(updateScrollState);
    resizeObserver.observe(rail);
    return () => {
      rail.removeEventListener("scroll", updateScrollState);
      resizeObserver.disconnect();
    };
  }, [categories.join("|")]);

  const scrollCategories = (direction: -1 | 1) => {
    categoryRailRef.current?.scrollBy({
      behavior: "smooth",
      left: direction * 240,
    });
  };

  const filteredSkills = catalogItems.filter(
    (skill) =>
      (view === "catalog" || installed.has(skill.id)) &&
      (category === "全部" || skill.category === category) &&
      (!query.trim() ||
        `${skill.name}${skill.description}${skillCategoryLabel(skill.category)}`
          .toLowerCase()
          .includes(query.trim().toLowerCase())),
  );

  const install = async (skill: SkillItem) => {
    if (installingId) return;
    setInstallingId(skill.id);
    try {
      await onInstall(skill);
    } finally {
      setInstallingId(null);
    }
  };

  const chooseImport = async (kind: "zip" | "markdown") => {
    if (choosingImport || importing) return;
    setChoosingImport(kind);
    setImportError("");
    try {
      const source = await chooseLocalSkillFile(kind);
      if (!source) return;
      const inspection = await inspectLocalSkill(source);
      if (!inspection.skills.length) {
        throw new Error("选中的文件中没有可安装的 SKILL.md");
      }
      setImportDraft({
        kind,
        inspection,
        installDependencies: false,
      });
    } catch (error) {
      setImportError(errorMessage(error));
    } finally {
      setChoosingImport(null);
    }
  };

  const confirmImport = async () => {
    if (!importDraft || importing) return;
    setImporting(true);
    setImportError("");
    try {
      await onImport(importDraft);
      setImportDraft(null);
    } catch (error) {
      setImportError(errorMessage(error));
    } finally {
      setImporting(false);
    }
  };

  const loadingCatalog = !catalogReady && catalogItems.length === 0;

  return (
    <div className="library-page skill-library-page">
      <LibraryHeading
        description={`当前安装目标是 ${engine.name}。切换智能体后，页面不变，只改变安装位置。`}
        engine={engine}
        title="Skill 库"
      />

      <section
        className="library-toolbar library-toolbar--skills"
        aria-label="Skill 筛选"
      >
        <div className="library-view-tabs" aria-label="Skill 资源范围">
          <button
            className={view === "catalog" ? "active" : ""}
            onClick={() =>
              onStateChange((current) => ({ ...current, view: "catalog" }))
            }
            type="button"
          >
            资源库
          </button>
          <button
            className={view === "installed" ? "active" : ""}
            onClick={() =>
              onStateChange((current) => ({ ...current, view: "installed" }))
            }
            type="button"
          >
            本地已安装
          </button>
        </div>
        <label className="library-search">
          <MagnifyingGlass aria-hidden="true" />
          <input
            onChange={(event) =>
              onStateChange((current) => ({
                ...current,
                query: event.target.value,
              }))
            }
            placeholder="搜索 Skill 名称或功能"
            value={query}
          />
        </label>
        <div className="skill-category-rail">
          <button
            aria-label="向左查看更多分类"
            className="skill-category-scroll"
            disabled={!categoryScroll.canScrollLeft}
            onClick={() => scrollCategories(-1)}
            type="button"
          >
            <CaretLeft aria-hidden="true" />
          </button>
          <div
            className="library-categories"
            aria-label="Skill 分类"
            ref={categoryRailRef}
          >
            {categories.map((item) => (
              <button
                className={category === item ? "active" : ""}
                key={item}
                onClick={() =>
                  onStateChange((current) => ({
                    ...current,
                    category: item,
                  }))
                }
                type="button"
              >
                {skillCategoryLabel(item)}
              </button>
            ))}
          </div>
          <button
            aria-label="向右查看更多分类"
            className="skill-category-scroll"
            disabled={!categoryScroll.canScrollRight}
            onClick={() => scrollCategories(1)}
            type="button"
          >
            <CaretRight aria-hidden="true" />
          </button>
        </div>
        <div className="library-import-actions" aria-label="导入本地 Skill">
          <button
            disabled={choosingImport !== null || importing}
            onClick={() => void chooseImport("zip")}
            type="button"
          >
            {choosingImport === "zip" ? (
              <SpinnerGap className="spin" aria-hidden="true" />
            ) : (
              <FileZip aria-hidden="true" />
            )}
            导入 ZIP
          </button>
          <button
            disabled={choosingImport !== null || importing}
            onClick={() => void chooseImport("markdown")}
            type="button"
          >
            {choosingImport === "markdown" ? (
              <SpinnerGap className="spin" aria-hidden="true" />
            ) : (
              <FileMd aria-hidden="true" />
            )}
            导入 MD
          </button>
        </div>
      </section>

      {syncingCatalog ? (
        <div className="library-sync-note" role="status">
          <SpinnerGap className="spin" aria-hidden="true" />
          正在更新 SkillHub 结果…
        </div>
      ) : null}
      {catalogError ? (
        <div className="library-sync-note library-sync-note--warning" role="status">
          <Info aria-hidden="true" />
          SkillHub 暂时不可用，已保留当前页面：{catalogError}
        </div>
      ) : null}
      {importError && !importDraft ? (
        <div className="library-sync-note library-sync-note--warning" role="alert">
          <Info aria-hidden="true" />
          {importError}
        </div>
      ) : null}

      <section className="library-grid" aria-live="polite">
        {loadingCatalog ? (
          <div className="library-empty">
            <SpinnerGap className="spin" aria-hidden="true" />
            <strong>正在读取 SkillHub</strong>
            <span>搜索结果会保留在当前智能体页面。</span>
          </div>
        ) : null}
        {!loadingCatalog && filteredSkills.length ? (
          filteredSkills.map((skill) => {
            const isInstalled = installed.has(skill.id);
            const isInstalling = installingId === skill.id;
            return (
              <article className="library-card" key={skill.id}>
                <header className="library-card-header">
                  <span className="library-mark library-mark--skill">
                    <img alt="" src={skill.iconSrc} />
                  </span>
                  <button
                    className={isInstalled ? "secondary-action" : "primary-action"}
                    disabled={isInstalling}
                    onClick={() => void install(skill)}
                    type="button"
                  >
                    {isInstalling ? (
                      <SpinnerGap className="spin" aria-hidden="true" />
                    ) : isInstalled ? (
                      <Check aria-hidden="true" />
                    ) : (
                      <Plus aria-hidden="true" />
                    )}
                    {isInstalling ? "安装中" : isInstalled ? "已安装" : "安装"}
                  </button>
                </header>
                <div className="library-copy">
                  <div className="library-title-line">
                    <strong>{skill.name}</strong>
                    {isInstalled ? <em>已安装</em> : null}
                  </div>
                  <p>{skill.description}</p>
                  <small>
                    {skillCategoryLabel(skill.category)} · v{skill.version} ·{" "}
                    {skill.source}
                  </small>
                </div>
              </article>
            );
          })
        ) : !loadingCatalog ? (
          <div className="library-empty">
            <MagnifyingGlass aria-hidden="true" />
            <strong>没有找到匹配的 Skill</strong>
            <span>可以缩短关键词或切换到“全部”分类。</span>
          </div>
        ) : null}
      </section>

      <footer className="library-footer-note">
        <ShieldCheck aria-hidden="true" />
        <span>安装前会校验官方签名和内容哈希，失败时不会写入智能体目录。</span>
      </footer>

      {importDraft ? (
        <div
          className="skill-import-backdrop"
          onMouseDown={(event) => {
            if (event.currentTarget === event.target && !importing) {
              setImportDraft(null);
            }
          }}
          role="presentation"
        >
          <section
            aria-labelledby="skill-import-title"
            aria-modal="true"
            className="skill-import-dialog"
            role="dialog"
          >
            <header>
              <div>
                <span className="skill-import-kicker">本地 Skill 导入</span>
                <h2 id="skill-import-title">确认安装到 {engine.name}</h2>
                <p>{fileNameFromPath(importDraft.inspection.source)}</p>
              </div>
              <button
                aria-label="关闭导入确认"
                className="icon-button"
                disabled={importing}
                onClick={() => setImportDraft(null)}
                type="button"
              >
                <X aria-hidden="true" />
              </button>
            </header>

            <div className="skill-import-list">
              {importDraft.inspection.skills.map((skill) => (
                <article key={`${skill.slug}-${skill.relativeRoot}`}>
                  <span className="library-mark library-mark--skill">
                    {importDraft.kind === "zip" ? (
                      <FileZip aria-hidden="true" />
                    ) : (
                      <FileMd aria-hidden="true" />
                    )}
                  </span>
                  <div>
                    <strong>{skill.name || skill.slug}</strong>
                    <p>{skill.description || "该 Skill 暂无详细介绍。"}</p>
                    <small>
                      {skill.slug} · v{skill.version || "local"}
                      {installed.has(skill.slug) ? " · 将更新已安装版本" : ""}
                    </small>
                  </div>
                </article>
              ))}
            </div>

            {importDraft.inspection.skills.some((skill) =>
              hasSkillDependencies(skill),
            ) ? (
              <label className="skill-import-dependencies">
                <input
                  checked={importDraft.installDependencies}
                  onChange={(event) =>
                    setImportDraft((current) =>
                      current
                        ? {
                            ...current,
                            installDependencies: event.target.checked,
                          }
                        : current,
                    )
                  }
                  type="checkbox"
                />
                <span>
                  <strong>同时安装 Skill 声明的依赖</strong>
                  <small>默认关闭；只有明确信任该 Skill 时再开启。</small>
                </span>
              </label>
            ) : null}

            {importError ? (
              <div className="skill-import-error" role="alert">
                <Info aria-hidden="true" />
                {importError}
              </div>
            ) : null}

            <footer>
              <button
                className="secondary-action"
                disabled={importing}
                onClick={() => setImportDraft(null)}
                type="button"
              >
                取消
              </button>
              <button
                className="primary-action"
                disabled={importing}
                onClick={() => void confirmImport()}
                type="button"
              >
                {importing ? (
                  <SpinnerGap className="spin" aria-hidden="true" />
                ) : (
                  <DownloadSimple aria-hidden="true" />
                )}
                {importing ? "正在安装" : `安装到 ${engine.name}`}
              </button>
            </footer>
          </section>
        </div>
      ) : null}
    </div>
  );
}

function PluginLibraryPage({
  categories: catalogCategories,
  installed,
  items,
  onInstall,
  onInstallAll,
}: {
  categories: string[];
  installed: Set<string>;
  items: PluginItem[];
  onInstall: (pluginId: string) => Promise<void>;
  onInstallAll: (ids: string[]) => Promise<void>;
}) {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("全部");
  const [view, setView] = useState<LibraryView>("catalog");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [batch, setBatch] = useState({
    active: false,
    completed: 0,
    total: 0,
    current: "",
  });
  const categories = ["全部", ...catalogCategories];
  const filteredPlugins = items.filter(
    (plugin) =>
      (view === "catalog" || installed.has(plugin.id)) &&
      (category === "全部" || plugin.category === category) &&
      (!query.trim() ||
        `${plugin.displayName}${plugin.shortDescription}`
          .toLowerCase()
          .includes(query.trim().toLowerCase())),
  );

  const install = async (pluginId: string) => {
    if (installingId) return;
    setInstallingId(pluginId);
    try {
      await onInstall(pluginId);
    } finally {
      setInstallingId(null);
    }
  };

  const installAll = async () => {
    if (batch.active) return;
    const queue = items.filter((plugin) => !installed.has(plugin.id));
    if (!queue.length) {
      setBatch({ active: false, completed: 0, total: 0, current: "" });
      return;
    }
    setBatch({
      active: true,
      completed: 0,
      total: queue.length,
      current: queue[0].displayName,
    });
    try {
      await onInstallAll(queue.map((plugin) => plugin.id));
      setBatch((current) => ({
        ...current,
        completed: queue.length,
      }));
    } finally {
      setInstallingId(null);
      setBatch((current) => ({
        ...current,
        active: false,
        current: "",
      }));
    }
  };

  const uninstalledCount = items.filter(
    (plugin) => !installed.has(plugin.id),
  ).length;

  return (
    <div className="library-page plugin-library-page">
      <LibraryHeading
        action={
          <button
            className="batch-install-button"
            disabled={batch.active || uninstalledCount === 0}
            onClick={() => void installAll()}
            type="button"
          >
            {batch.active ? (
              <SpinnerGap className="spin" aria-hidden="true" />
            ) : (
              <Lightning aria-hidden="true" weight="fill" />
            )}
            {batch.active
              ? `正在装配 ${batch.completed}/${batch.total}`
              : uninstalledCount
                ? "一键装配全部"
                : "已全部装配"}
          </button>
        }
        description="Codex 专属插件中心。插件主体来自移动盘预置快照，不会安装到另外两个智能体。"
        engine={ENGINES[0]}
        title="插件库"
      />

      <section
        className="library-toolbar library-toolbar--plugins"
        aria-label="插件筛选"
      >
        <div className="library-view-tabs" aria-label="插件资源范围">
          <button
            className={view === "catalog" ? "active" : ""}
            onClick={() => setView("catalog")}
            type="button"
          >
            资源库
          </button>
          <button
            className={view === "installed" ? "active" : ""}
            onClick={() => setView("installed")}
            type="button"
          >
            本地已安装
          </button>
        </div>
        <label className="library-search">
          <MagnifyingGlass aria-hidden="true" />
          <input
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索插件名称或能力"
            value={query}
          />
        </label>
        <div className="library-categories" aria-label="插件分类">
          {categories.map((item) => (
            <button
              className={category === item ? "active" : ""}
              key={item}
              onClick={() => setCategory(item)}
              type="button"
            >
              {item}
            </button>
          ))}
        </div>
      </section>

      {batch.total > 0 ? (
        <section className="batch-progress-panel" aria-live="polite">
          <div>
            <strong>
              {batch.active ? "正在装配 Codex 插件" : "插件装配完成"}
            </strong>
            <span>
              {batch.active && batch.current
                ? `当前：${batch.current}`
                : `成功处理 ${batch.completed} 个插件`}
            </span>
          </div>
          <span>
            {batch.completed} / {batch.total}
          </span>
          <div
            aria-valuemax={batch.total}
            aria-valuemin={0}
            aria-valuenow={batch.completed}
            className="batch-progress-track"
            role="progressbar"
          >
            <span
              style={{
                transform: `scaleX(${
                  batch.total ? batch.completed / batch.total : 0
                })`,
              }}
            />
          </div>
        </section>
      ) : null}

      <section className="library-grid" aria-live="polite">
        {filteredPlugins.length ? (
          filteredPlugins.map((plugin) => {
            const isInstalled = installed.has(plugin.id);
            const isInstalling = installingId === plugin.id;
            const isExpanded = expandedId === plugin.id;
            return (
              <article className="library-card" key={plugin.id}>
                <header className="library-card-header">
                  <span
                    className="library-mark"
                    style={
                      {
                        "--library-color": plugin.brandColor ?? "#52647d",
                      } as CSSProperties
                    }
                  >
                    {plugin.iconUrl ? (
                      <img alt="" src={plugin.iconUrl} />
                    ) : (
                      <PuzzlePiece aria-hidden="true" />
                    )}
                  </span>
                  <button
                    className={isInstalled ? "secondary-action" : "primary-action"}
                    disabled={isInstalling || batch.active}
                    onClick={() => void install(plugin.id)}
                    type="button"
                  >
                    {isInstalling ? (
                      <SpinnerGap className="spin" aria-hidden="true" />
                    ) : isInstalled ? (
                      <Check aria-hidden="true" />
                    ) : (
                      <Plus aria-hidden="true" />
                    )}
                    {isInstalling ? "安装中" : isInstalled ? "已安装" : "安装"}
                  </button>
                </header>
                <div className="library-copy">
                  <div className="library-title-line">
                    <strong>{plugin.displayName}</strong>
                    {isInstalled ? <em>已安装</em> : null}
                  </div>
                  <p>{plugin.shortDescription}</p>
                  <small>
                    {plugin.category} · v{plugin.version} ·{" "}
                    {plugin.authenticationLabel}
                  </small>
                </div>
                {isExpanded ? (
                  <p className="library-detail-copy">{plugin.longDescription}</p>
                ) : null}
                <button
                  aria-expanded={isExpanded}
                  className="library-expand"
                  onClick={() =>
                    setExpandedId((current) =>
                      current === plugin.id ? null : plugin.id,
                    )
                  }
                  type="button"
                >
                  {isExpanded ? "收起" : "展开"}
                  <CaretDown
                    aria-hidden="true"
                    className={isExpanded ? "expanded" : ""}
                  />
                </button>
              </article>
            );
          })
        ) : (
          <div className="library-empty">
            <MagnifyingGlass aria-hidden="true" />
            <strong>没有找到匹配的插件</strong>
            <span>可以缩短关键词或切换到“全部”分类。</span>
          </div>
        )}
      </section>

      <footer className="library-footer-note">
        <ShieldCheck aria-hidden="true" />
        <span>插件库仅在 Codex 模式显示，安装后由 Codex 完成第三方账号连接。</span>
      </footer>
    </div>
  );
}

function LibraryHeading({
  action,
  description,
  engine,
  title,
}: {
  action?: ReactNode;
  description: string;
  engine: Engine;
  title: string;
}) {
  return (
    <header className="library-heading">
      <div>
        <h1>{title}</h1>
        <p>{description}</p>
      </div>
      <span className="library-heading-tools">
        <span className="library-target">
          <EngineMark engine={engine} />
          <span>
            <small>当前目标</small>
            <strong>{engine.name}</strong>
          </span>
        </span>
        {action}
      </span>
    </header>
  );
}

const tutorialSelectionMemory = new Map<EngineId, string>();
const tutorialScrollMemory = new Map<string, number>();

function TutorialPage({ engine }: { engine: Engine }) {
  const [entries, setEntries] = useState<TutorialEntry[]>([]);
  const [document, setDocument] = useState<TutorialDocument | null>(null);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState("");
  const readerRef = useRef<HTMLElement | null>(null);
  const requestIdRef = useRef(0);

  useEffect(() => {
    let cancelled = false;
    const requestId = ++requestIdRef.current;
    setLoading(true);
    setEntries([]);
    setDocument(null);
    setMessage("");
    void (async () => {
      try {
        const items = await loadTutorials(engine.id);
        if (cancelled || requestId !== requestIdRef.current) return;
        setEntries(items);
        const rememberedFile = tutorialSelectionMemory.get(engine.id);
        const selected =
          items.find((item) => item.fileName === rememberedFile) ?? items[0];
        if (!selected) return;
        tutorialSelectionMemory.set(engine.id, selected.fileName);
        const nextDocument = await readTutorial(engine.id, selected.fileName);
        if (cancelled || requestId !== requestIdRef.current) return;
        setDocument(nextDocument);
        restoreTutorialScroll(readerRef.current, engine.id, selected.fileName);
      } catch (error) {
        if (!cancelled && requestId === requestIdRef.current) {
          setMessage(errorMessage(error));
        }
      } finally {
        if (!cancelled && requestId === requestIdRef.current) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [engine.id]);

  const openTutorial = async (entry: TutorialEntry) => {
    const requestId = ++requestIdRef.current;
    rememberTutorialScroll(readerRef.current, engine.id, document?.entry.fileName);
    tutorialSelectionMemory.set(engine.id, entry.fileName);
    setLoading(true);
    setDocument(null);
    setMessage("");
    try {
      const nextDocument = await readTutorial(engine.id, entry.fileName);
      if (requestId !== requestIdRef.current) return;
      setDocument(nextDocument);
      restoreTutorialScroll(readerRef.current, engine.id, entry.fileName);
    } catch (error) {
      if (requestId === requestIdRef.current) setMessage(errorMessage(error));
    } finally {
      if (requestId === requestIdRef.current) setLoading(false);
    }
  };

  return (
    <div className="tutorial-page">
      <section className="tutorial-browser">
        <aside className="tutorial-catalog">
          <header className="tutorial-catalog-header">
            <span className="tutorial-catalog-icon">
              <GraduationCap aria-hidden="true" weight="fill" />
            </span>
            <span>
              <strong>{engine.name} 使用教程</strong>
              <small>{entries.length} 篇教程</small>
            </span>
          </header>

          <nav aria-label={`${engine.name} 教程目录`} className="tutorial-list">
            {entries.map((tutorial, index) => {
              const active = document?.entry.fileName === tutorial.fileName;
              return (
                <button
                  aria-current={active ? "page" : undefined}
                  className={`tutorial-row ${active ? "active" : ""}`}
                  key={tutorial.id}
                  onClick={() => void openTutorial(tutorial)}
                  type="button"
                >
                  <span className="tutorial-index">
                    {String(index + 1).padStart(2, "0")}
                  </span>
                  <span className="tutorial-row-copy">
                    <strong>{tutorial.title}</strong>
                    <small>{(tutorial.bytes / 1024).toFixed(1)} KB</small>
                  </span>
                  <CaretRight aria-hidden="true" />
                </button>
              );
            })}
            {loading && entries.length === 0 ? (
              <span className="tutorial-catalog-loading">
                <SpinnerGap className="spin" aria-hidden="true" />
                正在读取课程目录
              </span>
            ) : null}
          </nav>

        </aside>

        <article
          className="tutorial-reader"
          onScroll={(event) =>
            rememberTutorialScroll(
              event.currentTarget,
              engine.id,
              document?.entry.fileName,
            )
          }
          ref={readerRef}
        >
          {document ? (
            <>
              <header className="tutorial-reader-header">
                <div>
                  <span>{engine.name} 教程</span>
                  <h1>{document.entry.title}</h1>
                </div>
                <EngineMark engine={engine} />
              </header>
              <Suspense
                fallback={
                  <div className="tutorial-markdown-loading" role="status">
                    <SpinnerGap className="spin" aria-hidden="true" />
                    正在准备教程排版
                  </div>
                }
              >
                <MarkdownTutorial
                  engine={engine.id}
                  markdown={document.markdown}
                />
              </Suspense>
            </>
          ) : loading ? (
            <div className="tutorial-reader-state" role="status">
              <SpinnerGap className="spin" aria-hidden="true" />
              <strong>正在读取移动盘教程</strong>
              <span>正文加载完成后会自动显示。</span>
            </div>
          ) : message ? (
            <div className="tutorial-reader-state tutorial-reader-state--error">
              <Info aria-hidden="true" />
              <strong>教程读取失败</strong>
              <span>{message}</span>
              {entries[0] ? (
                <button onClick={() => void openTutorial(entries[0])} type="button">
                  重新读取
                </button>
              ) : null}
            </div>
          ) : (
            <div className="tutorial-reader-state">
              <GraduationCap aria-hidden="true" />
              <strong>当前智能体暂无教程</strong>
              <span>正式教程加入移动盘后会自动显示在这里。</span>
            </div>
          )}
        </article>
      </section>
    </div>
  );
}

function tutorialScrollKey(engine: EngineId, fileName: string): string {
  return `${engine}:${fileName}`;
}

function rememberTutorialScroll(
  reader: HTMLElement | null,
  engine: EngineId,
  fileName?: string,
) {
  if (!reader || !fileName) return;
  tutorialScrollMemory.set(tutorialScrollKey(engine, fileName), reader.scrollTop);
}

function restoreTutorialScroll(
  reader: HTMLElement | null,
  engine: EngineId,
  fileName: string,
) {
  const top = tutorialScrollMemory.get(tutorialScrollKey(engine, fileName)) ?? 0;
  window.requestAnimationFrame(() => reader?.scrollTo({ top }));
}

function SettingsPage({
  activeEngine,
  onOpenProvider,
  runtimeStates,
  savedEngines,
  workstationRoot,
}: {
  activeEngine: Engine;
  onOpenProvider: (engineId: EngineId) => void;
  runtimeStates: Partial<Record<EngineId, EngineState>>;
  savedEngines: Set<EngineId>;
  workstationRoot: string;
}) {
  const [diagnosing, setDiagnosing] = useState(false);
  const [diagnosisMessage, setDiagnosisMessage] = useState("");
  const [localeStatus, setLocaleStatus] =
    useState<CodexLocaleInitializationStatus | null>(null);
  const [localeWorking, setLocaleWorking] = useState(false);
  const [localeMessage, setLocaleMessage] = useState("");
  const [proxyOpen, setProxyOpen] = useState(false);
  const [factorySeed, setFactorySeed] = useState<FactorySeedStatus | null>(null);
  const [factoryPreview, setFactoryPreview] =
    useState<FactoryResetPreview | null>(null);
  const [factoryStatus, setFactoryStatus] =
    useState<FactoryResetStatus | null>(null);
  const [factoryDialogStep, setFactoryDialogStep] = useState<
    "closed" | "preview" | "confirm" | "progress"
  >("closed");
  const [factoryAcknowledged, setFactoryAcknowledged] = useState(false);
  const [factoryWorking, setFactoryWorking] = useState(false);
  const [factoryMessage, setFactoryMessage] = useState("");
  const [preferences, setPreferences] = useState<UiPreferences>({
    reduceMotion: false,
    offlineFirst: false,
    compactMode: false,
  });

  useEffect(() => {
    void loadUiPreferences().then(setPreferences).catch(() => undefined);
    void loadCodexLocaleStatus()
      .then(setLocaleStatus)
      .catch((error) => setLocaleMessage(errorMessage(error)));
    void verifyFactorySeed().then(setFactorySeed).catch(() => undefined);
    void loadFactoryResetStatus().then(setFactoryStatus).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (factoryDialogStep !== "progress") return;
    const timer = window.setInterval(() => {
      void loadFactoryResetStatus().then(setFactoryStatus).catch(() => undefined);
    }, 500);
    return () => window.clearInterval(timer);
  }, [factoryDialogStep]);

  useEffect(() => {
    document.documentElement.dataset.reduceMotion = String(
      preferences.reduceMotion,
    );
    document.documentElement.dataset.compactMode = String(
      preferences.compactMode,
    );
  }, [preferences.compactMode, preferences.reduceMotion]);

  const updatePreference = async (
    field: keyof UiPreferences,
    value: boolean,
  ) => {
    const next = { ...preferences, [field]: value };
    setPreferences(next);
    try {
      setPreferences(await saveUiPreferences(next));
    } catch (error) {
      setDiagnosisMessage(errorMessage(error));
    }
  };

  const diagnose = async () => {
    setDiagnosing(true);
    setDiagnosisMessage("");
    try {
      const diagnosis = await runDiagnosis();
      const detections =
        (diagnosis as { detections?: Array<{ ready: boolean }> }).detections ?? [];
      const ready = detections.filter((item) => item.ready).length;
      setDiagnosisMessage(`检查完成：${ready}/${detections.length} 个智能体主体完整`);
    } catch (error) {
      setDiagnosisMessage(errorMessage(error));
    } finally {
      setDiagnosing(false);
    }
  };

  const initializeCodexLocale = async () => {
    setLocaleWorking(true);
    setLocaleMessage("");
    try {
      const next = await applyCodexLocaleSeed();
      setLocaleStatus(next);
      setLocaleMessage(codexLocaleStateLabel(next.current.nativeI18nState));
    } catch (error) {
      setLocaleMessage(errorMessage(error));
    } finally {
      setLocaleWorking(false);
    }
  };

  const openFactoryReset = async () => {
    setFactoryWorking(true);
    setFactoryMessage("");
    setFactoryPreview(null);
    setFactoryAcknowledged(false);
    try {
      const preview = await previewFactoryReset();
      setFactoryPreview(preview);
      setFactoryDialogStep("preview");
    } catch (error) {
      setFactoryMessage(errorMessage(error));
      setFactoryDialogStep("preview");
    } finally {
      setFactoryWorking(false);
    }
  };

  const executeReset = async () => {
    if (!factoryPreview || !factoryAcknowledged) return;
    setFactoryWorking(true);
    setFactoryStatus(null);
    setFactoryDialogStep("progress");
    setFactoryMessage("");
    try {
      setFactoryStatus(await executeFactoryReset(factoryPreview.previewId));
      setFactorySeed(await verifyFactorySeed());
    } catch (error) {
      setFactoryMessage(errorMessage(error));
      setFactoryStatus(await loadFactoryResetStatus().catch(() => null));
    } finally {
      setFactoryWorking(false);
    }
  };

  const rollbackReset = async () => {
    if (!factoryStatus?.operationId) return;
    setFactoryWorking(true);
    setFactoryMessage("");
    try {
      setFactoryStatus(await rollbackFactoryReset(factoryStatus.operationId));
    } catch (error) {
      setFactoryMessage(errorMessage(error));
    } finally {
      setFactoryWorking(false);
    }
  };

  const factoryCanRollback =
    factoryStatus?.state === "failed" && factoryStatus.canRollback;
  const factoryEntryDescription = factoryCanRollback
    ? "存在未完成恢复，点击查看并回滚"
    : factoryMessage ||
      (factorySeed?.state === "ready"
        ? "清除个人数据并恢复默认配置"
        : factorySeed?.message ?? "正在检查恢复资源");

  return (
    <div className="settings-page">
      <section className="settings-intro">
        <h1>设置</h1>
        <p>
          当前智能体是 {activeEngine.name}。供应商、数据目录、环境检查和维护功能统一放在这里。
        </p>
      </section>

      <section className="settings-card content-card">
        <header className="settings-card-title">
          <span className="settings-icon">
            <Key aria-hidden="true" />
          </span>
          <div>
            <h2>智能体与供应商</h2>
            <p>三套配置彼此独立，切换不会覆盖。</p>
          </div>
        </header>
        <div className="settings-engine-list">
          {ENGINES.map((engine) => (
            <button
              key={engine.id}
              onClick={() => onOpenProvider(engine.id)}
              type="button"
            >
              <EngineMark engine={engine} />
              <span>
                <strong>{engine.name}</strong>
                <small>
                  {runtimeStates[engine.id]?.status === "running"
                    ? "正在运行"
                    : savedEngines.has(engine.id)
                      ? "配置已保存"
                      : "等待配置"}
                </small>
              </span>
              <CaretRight aria-hidden="true" />
            </button>
          ))}
        </div>
      </section>

      <section className="settings-sections">
        <SettingsDetails
          description="随盘中文缓存与仅限 Codex 子进程的备用网络初始化"
          icon={Cloud}
          title="Codex 中文初始化"
        >
          <div className="settings-summary-row">
            <span>当前中文状态</span>
            <strong>
              {localeStatus
                ? codexLocaleStateLabel(localeStatus.current.nativeI18nState)
                : "正在检查"}
            </strong>
          </div>
          <div className="settings-summary-row">
            <span>随盘中文缓存</span>
            <strong>{localeStatus?.seed.message ?? "正在检查"}</strong>
          </div>
          <div className="settings-summary-row">
            <span>备用网络初始化</span>
            <strong>
              {localeStatus?.proxy.configured ? "已保存配置" : "未配置"}
            </strong>
          </div>
          {localeMessage ? (
            <div className="inline-operation-message">{localeMessage}</div>
          ) : null}
          <div className="codex-locale-actions">
            <button
              className="secondary-action"
              disabled={localeWorking || localeStatus?.seed.state !== "ready"}
              onClick={() => void initializeCodexLocale()}
              type="button"
            >
              {localeWorking ? (
                <SpinnerGap className="spin" aria-hidden="true" />
              ) : (
                <ArrowsClockwise aria-hidden="true" />
              )}
              {localeStatus?.seed.state === "ready"
                ? "本地中文初始化"
                : "中文缓存尚未准备"}
            </button>
            <button
              className="secondary-action"
              onClick={() => setProxyOpen(true)}
              type="button"
            >
              <Cloud aria-hidden="true" />
              配置 Codex 网络初始化代理
            </button>
          </div>
        </SettingsDetails>
        <SettingsDetails
          description="移动盘数据、工作区与备份位置"
          icon={Database}
          title="数据与目录"
        >
          <div className="settings-summary-row">
            <span>项目与会话</span>
            <strong>跟随移动盘保存</strong>
          </div>
          <div className="settings-summary-row">
            <span>当前工作站</span>
            <strong title={workstationRoot}>
              {workstationRoot || "浏览器预览"}
            </strong>
          </div>
          <div className="settings-summary-row">
            <span>备份策略</span>
            <strong>安全弹出前检查</strong>
          </div>
          <button
            className="secondary-action"
            onClick={() => void openBackups()}
            type="button"
          >
            打开备份目录
          </button>
        </SettingsDetails>
        <SettingsDetails
          description="运行依赖和首次使用检查"
          icon={ShieldCheck}
          title="环境与依赖"
        >
          <div className="settings-summary-row">
            <span>依赖检查</span>
            <strong>{diagnosisMessage || "可立即检查"}</strong>
          </div>
          <button
            className="secondary-action"
            disabled={diagnosing}
            onClick={() => void diagnose()}
            type="button"
          >
            {diagnosing ? <SpinnerGap className="spin" aria-hidden="true" /> : null}
            {diagnosing ? "检查中" : "开始检查"}
          </button>
        </SettingsDetails>
        <SettingsDetails
          description="界面、离线优先与高级显示"
          icon={GearSix}
          title="界面与高级选项"
        >
          <label className="settings-summary-row">
            <span>减少动画</span>
            <input
              checked={preferences.reduceMotion}
              onChange={(event) =>
                void updatePreference("reduceMotion", event.target.checked)
              }
              type="checkbox"
            />
          </label>
          <label className="settings-summary-row">
            <span>离线缓存优先</span>
            <input
              checked={preferences.offlineFirst}
              onChange={(event) =>
                void updatePreference("offlineFirst", event.target.checked)
              }
              type="checkbox"
            />
          </label>
          <label className="settings-summary-row">
            <span>紧凑信息密度</span>
            <input
              checked={preferences.compactMode}
              onChange={(event) =>
                void updatePreference("compactMode", event.target.checked)
              }
              type="checkbox"
            />
          </label>
        </SettingsDetails>
        <SettingsDetails
          description="诊断、日志和恢复工具"
          icon={Wrench}
          title="日志与维护"
        >
          <div className="settings-summary-row">
            <span>诊断日志</span>
            <strong>仅记录脱敏信息</strong>
          </div>
          <button
            className="secondary-action"
            onClick={() => void openLogs()}
            type="button"
          >
            打开日志目录
          </button>
        </SettingsDetails>
        <button
          className="factory-reset-entry content-card"
          data-state={factoryCanRollback ? "failed" : factorySeed?.state ?? "checking"}
          disabled={
            factoryWorking ||
            (!factoryCanRollback && factorySeed?.state !== "ready")
          }
          onClick={() => {
            if (factoryCanRollback) {
              setFactoryDialogStep("progress");
              return;
            }
            void openFactoryReset();
          }}
          type="button"
        >
          <span className="factory-reset-icon">
            <Trash aria-hidden="true" />
          </span>
          <span>
            <strong>恢复出厂设置</strong>
            <small>{factoryEntryDescription}</small>
          </span>
          {factoryWorking ? (
            <SpinnerGap className="spin" aria-hidden="true" />
          ) : (
            <CaretRight aria-hidden="true" />
          )}
        </button>
      </section>
      {proxyOpen ? (
        <CodexInitProxyDrawer
          initialStatus={localeStatus?.proxy}
          onClose={() => setProxyOpen(false)}
          onStatus={(proxy) =>
            setLocaleStatus((current) =>
              current ? { ...current, proxy } : current,
            )
          }
        />
      ) : null}
      {factoryDialogStep !== "closed" ? (
        <FactoryResetDialog
          acknowledged={factoryAcknowledged}
          dialogStep={factoryDialogStep}
          message={factoryMessage}
          preview={factoryPreview}
          status={factoryStatus}
          working={factoryWorking}
          onAcknowledge={setFactoryAcknowledged}
          onBack={() => setFactoryDialogStep("preview")}
          onClose={() => setFactoryDialogStep("closed")}
          onConfirm={() => void executeReset()}
          onContinue={() => setFactoryDialogStep("confirm")}
          onReload={() => window.location.reload()}
          onRollback={() => void rollbackReset()}
        />
      ) : null}
    </div>
  );
}

const FACTORY_RESET_STEPS: Array<{
  id: FactoryResetStep;
  label: string;
}> = [
  { id: "stopServices", label: "停止服务" },
  { id: "createRestorePoint", label: "生成恢复点" },
  { id: "cleanUserData", label: "清理用户数据" },
  { id: "restoreFactoryDefaults", label: "恢复出厂默认数据" },
  { id: "integrityCheck", label: "完整性检查" },
];

function FactoryResetDialog({
  acknowledged,
  dialogStep,
  message,
  preview,
  status,
  working,
  onAcknowledge,
  onBack,
  onClose,
  onConfirm,
  onContinue,
  onReload,
  onRollback,
}: {
  acknowledged: boolean;
  dialogStep: "preview" | "confirm" | "progress";
  message: string;
  preview: FactoryResetPreview | null;
  status: FactoryResetStatus | null;
  working: boolean;
  onAcknowledge: (value: boolean) => void;
  onBack: () => void;
  onClose: () => void;
  onConfirm: () => void;
  onContinue: () => void;
  onReload: () => void;
  onRollback: () => void;
}) {
  const terminal = status?.state === "completed" || status?.state === "failed" || status?.state === "rolledBack";
  const currentIndex = status?.currentStep
    ? FACTORY_RESET_STEPS.findIndex((step) => step.id === status.currentStep)
    : -1;
  return (
    <div className="factory-reset-overlay" role="presentation">
      <section
        aria-describedby="factory-reset-description"
        aria-labelledby="factory-reset-title"
        aria-modal="true"
        className="factory-reset-dialog"
        role="dialog"
      >
        <header>
          <div>
            <strong id="factory-reset-title">恢复出厂设置</strong>
            <p id="factory-reset-description">不会格式化移动盘，也不会影响工作站目录之外的文件。</p>
          </div>
          {dialogStep !== "progress" || terminal ? (
            <button aria-label="关闭" onClick={onClose} type="button"><X aria-hidden="true" /></button>
          ) : null}
        </header>

        {dialogStep === "preview" && preview ? (
          <>
            <div className="factory-reset-lists">
              <section>
                <h3>将清除</h3>
                <ul>{preview.clean.map((item) => <li key={item}>{item}</li>)}</ul>
              </section>
              <section>
                <h3>将保留</h3>
                <ul>{preview.preserve.map((item) => <li key={item}>{item}</li>)}</ul>
              </section>
            </div>
            <div className="factory-reset-space">
              预计需要 {formatFactoryBytes(preview.requiredBytes)}，当前可用 {formatFactoryBytes(preview.availableBytes)}
            </div>
            {preview.blockers.length ? (
              <div className="factory-reset-error">{preview.blockers.join("；")}</div>
            ) : null}
            <footer>
              <button className="secondary-action" onClick={onClose} type="button">取消</button>
              <button className="danger-action" disabled={!preview.canExecute} onClick={onContinue} type="button">继续</button>
            </footer>
          </>
        ) : null}

        {dialogStep === "preview" && !preview && message ? (
          <>
            <div className="factory-reset-error">{message}</div>
            <footer>
              <button className="secondary-action" onClick={onClose} type="button">关闭</button>
            </footer>
          </>
        ) : null}

        {dialogStep === "confirm" ? (
          <>
            <div className="factory-reset-confirmation">
              <strong>最后确认</strong>
              <p>执行后将清除上述个人数据；失败时可使用本次恢复点回滚。</p>
              <label>
                <input checked={acknowledged} onChange={(event) => onAcknowledge(event.target.checked)} type="checkbox" />
                我已了解清理与保留范围，并确认恢复
              </label>
            </div>
            <footer>
              <button className="secondary-action" onClick={onBack} type="button">返回上一步</button>
              <button className="danger-action" disabled={!acknowledged || working} onClick={onConfirm} type="button">
                {working ? <SpinnerGap className="spin" aria-hidden="true" /> : null}
                确认恢复
              </button>
            </footer>
          </>
        ) : null}

        {dialogStep === "progress" ? (
          <>
            <ol className="factory-reset-progress">
              {FACTORY_RESET_STEPS.map((step, index) => {
                const completed = (status?.completedSteps ?? 0) > index;
                const active = currentIndex === index && status?.state === "running";
                return (
                  <li className={completed ? "is-complete" : active ? "is-active" : ""} key={step.id}>
                    <span>{completed ? <Check aria-hidden="true" /> : index + 1}</span>
                    <strong>{step.label}</strong>
                    {active ? <SpinnerGap className="spin" aria-hidden="true" /> : null}
                  </li>
                );
              })}
            </ol>
            <div className={status?.state === "failed" ? "factory-reset-error" : "factory-reset-status"}>
              {status?.error || message || status?.message || "正在准备恢复事务"}
            </div>
            <footer>
              {status?.state === "failed" && status.canRollback ? (
                <button className="secondary-action" disabled={working} onClick={onRollback} type="button">从恢复点回滚</button>
              ) : null}
              {status?.state === "completed" ? (
                <button className="primary-action" onClick={onReload} type="button">重新进入工作站</button>
              ) : null}
              {status?.state === "rolledBack" ? (
                <button className="secondary-action" onClick={onClose} type="button">关闭</button>
              ) : null}
            </footer>
          </>
        ) : null}
      </section>
    </div>
  );
}

function formatFactoryBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes >= Number.MAX_SAFE_INTEGER) return "充足";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

function CodexInitProxyDrawer({
  initialStatus,
  onClose,
  onStatus,
}: {
  initialStatus?: CodexLocaleInitializationStatus["proxy"];
  onClose: () => void;
  onStatus: (status: CodexLocaleInitializationStatus["proxy"]) => void;
}) {
  const emptyStatus: CodexLocaleInitializationStatus["proxy"] = {
    configured: false,
    nodeCount: 0,
    nodes: [],
    persistentEnabled: false,
    pendingOnce: false,
    active: false,
    mihomoVersion: "1.19.29",
  };
  const [status, setStatus] = useState(initialStatus ?? emptyStatus);
  const [subscriptionUrl, setSubscriptionUrl] = useState("");
  const [region, setRegion] = useState<CodexProxyRegion>("all");
  const [working, setWorking] = useState("");
  const [message, setMessage] = useState("");
  const [disclaimerOpen, setDisclaimerOpen] = useState(() => {
    try {
      return window.localStorage.getItem(CODEX_PROXY_DISCLAIMER_STORAGE_KEY) !== "1";
    } catch {
      return true;
    }
  });
  const [disclaimerAcknowledged, setDisclaimerAcknowledged] = useState(false);
  const [skipFutureDisclaimer, setSkipFutureDisclaimer] = useState(false);

  const continueFromDisclaimer = () => {
    if (!disclaimerAcknowledged) return;
    if (skipFutureDisclaimer) {
      try {
        window.localStorage.setItem(CODEX_PROXY_DISCLAIMER_STORAGE_KEY, "1");
      } catch {
        // 本机存储不可用时仅在下次继续显示提示，不影响代理配置。
      }
    }
    setDisclaimerOpen(false);
  };

  const commitStatus = (next: CodexLocaleInitializationStatus["proxy"]) => {
    setStatus(next);
    onStatus(next);
  };

  const run = async (
    action: "scan" | "once" | "persistent" | "clear",
  ) => {
    setWorking(action);
    setMessage("");
    try {
      if (action === "scan") {
        if (!subscriptionUrl.trim() && !status.subscriptionMasked) {
          setMessage("请先粘贴 Clash/Mihomo 兼容订阅链接。");
          return;
        }
        const next = await scanCodexProxySubscription(
          subscriptionUrl.trim() || undefined,
        );
        setSubscriptionUrl("");
        commitStatus(next);
        setMessage(`扫描完成：共读取 ${next.nodeCount} 个节点。`);
      } else if (action === "once") {
        await startCodexWithInitProxy();
        commitStatus(await loadCodexProxyStatus());
        setMessage(
          "Codex 已通过本次专用节点启动，中文初始化服务连通性已验证；请以 Codex 实际界面为准。退出 Codex 后代理自动关闭。",
        );
      } else if (action === "persistent") {
        const next = status.persistentEnabled
          ? await disableCodexProxy()
          : await enablePersistentCodexProxy();
        commitStatus(next);
        setMessage(
          next.persistentEnabled
            ? "Codex 专用代理已启用，以后启动 Codex 时按需自动启动。"
            : "Codex 专用代理已停用。",
        );
      } else {
        const next = await clearCodexProxy();
        setSubscriptionUrl("");
        setRegion("all");
        commitStatus(next);
        setMessage("订阅链接、节点缓存和选择已清除。");
      }
    } catch (error) {
      setMessage(errorMessage(error));
    } finally {
      setWorking("");
    }
  };

  const selectNode = async (nodeName: string) => {
    setWorking(`select:${nodeName}`);
    setMessage("");
    try {
      commitStatus(await selectCodexProxyNode(nodeName));
    } catch (error) {
      setMessage(errorMessage(error));
    } finally {
      setWorking("");
    }
  };

  const regionOptions: Array<{ id: CodexProxyRegion; label: string }> = [
    { id: "all", label: "全部" },
    { id: "hong_kong", label: "香港" },
    { id: "japan", label: "日本" },
    { id: "singapore", label: "新加坡" },
    { id: "united_states", label: "美国" },
    { id: "other", label: "其他" },
  ];
  const visibleNodes = status.nodes.filter(
    (node) => region === "all" || node.region === region,
  );
  const updatedAt = status.updatedAtUnixMs
    ? new Date(status.updatedAtUnixMs).toLocaleString("zh-CN", { hour12: false })
    : "尚未扫描";

  return (
    <aside
      aria-label="Codex 网络初始化代理"
      aria-modal="false"
      className="image-provider-drawer"
      role="dialog"
    >
      <div className="image-provider-panel content-card">
        <header className="provider-workspace-header">
          <div>
            <h2>Codex 网络初始化代理</h2>
            <p>仅供 Codex 子进程使用，不修改 Windows 系统代理。</p>
          </div>
          <button
            className="icon-button"
            onClick={onClose}
            title="关闭网络初始化配置"
            type="button"
          >
            <X aria-hidden="true" />
          </button>
        </header>
        <div className="chat-provider-body codex-proxy-body">
          <div
            aria-hidden={disclaimerOpen}
            className="codex-proxy-content"
            inert={disclaimerOpen}
          >
            <div className="codex-proxy-subscription">
            <label className="form-field">
              <span>Clash/Mihomo 订阅链接</span>
              <input
                autoComplete="off"
                onChange={(event) => setSubscriptionUrl(event.target.value)}
                placeholder={status.subscriptionMasked ?? "粘贴订阅链接"}
                spellCheck={false}
                type="password"
                value={subscriptionUrl}
              />
            </label>
            <button
              className="codex-proxy-clear"
              disabled={working !== "" || !status.subscriptionMasked}
              onClick={() => void run("clear")}
              type="button"
            >
              <Trash aria-hidden="true" />
              清除订阅
            </button>
          </div>
          <div className="codex-proxy-meta">
            <span><small>当前订阅</small><strong>{status.subscriptionName ?? "未配置"}</strong></span>
            <span><small>更新时间</small><strong>{updatedAt}</strong></span>
            <span><small>节点数量</small><strong>{status.nodeCount}</strong></span>
            <span><small>核心版本</small><strong>v{status.mihomoVersion}</strong></span>
          </div>
          <div className="codex-proxy-regions" aria-label="节点地区">
            {regionOptions.map((option) => (
              <button
                aria-pressed={region === option.id}
                className={region === option.id ? "active" : ""}
                key={option.id}
                onClick={() => setRegion(option.id)}
                type="button"
              >
                {option.label}
              </button>
            ))}
          </div>
          <div className="codex-proxy-nodes">
            {visibleNodes.map((node) => {
              const selected = status.selectedNode === node.name;
              return (
                <button
                  aria-pressed={selected}
                  className={`codex-proxy-node ${selected ? "selected" : ""}`}
                  disabled={working !== ""}
                  key={node.name}
                  onClick={() => void selectNode(node.name)}
                  type="button"
                >
                  <span>
                    <strong>{node.name}</strong>
                    <small>{regionOptions.find((item) => item.id === node.region)?.label ?? "其他"}</small>
                  </span>
                  <span className={node.available ? "available" : "timeout"}>
                    {working === `select:${node.name}` ? (
                      <SpinnerGap className="spin" aria-hidden="true" />
                    ) : selected ? (
                      <CheckCircle aria-hidden="true" weight="fill" />
                    ) : null}
                    {node.available && node.latencyMs ? `${node.latencyMs} ms` : "超时"}
                  </span>
                </button>
              );
            })}
            {status.nodeCount === 0 ? (
              <div className="codex-proxy-empty">粘贴订阅后点击“扫描节点”。</div>
            ) : visibleNodes.length === 0 ? (
              <div className="codex-proxy-empty">当前地区没有节点。</div>
            ) : null}
          </div>
          <div className="codex-proxy-note">
            Mihomo 仅监听 127.0.0.1 随机端口，不开启 TUN、系统代理或局域网访问。Claude Code、OpenClaw、AI 对话和 AI 生图不使用该节点。
          </div>
          {message ? <div className="inline-operation-message">{message}</div> : null}
            <footer className="chat-provider-actions codex-proxy-actions">
            <button className="secondary-action" disabled={working !== ""} onClick={() => void run("scan")} type="button">
              {working === "scan" ? <SpinnerGap className="spin" aria-hidden="true" /> : <ArrowsClockwise aria-hidden="true" />}
              扫描节点
            </button>
            <button
              className="primary-action"
              disabled={working !== "" || !status.configured || status.persistentEnabled}
              onClick={() => void run("once")}
              type="button"
            >
              {working === "once" ? <SpinnerGap className="spin" aria-hidden="true" /> : <Play aria-hidden="true" />}
              仅本次启动 Codex 中文初始化
            </button>
            <button
              className={status.persistentEnabled ? "danger-action" : "secondary-action"}
              disabled={working !== "" || (!status.configured && !status.persistentEnabled)}
              onClick={() => void run("persistent")}
              type="button"
            >
              {working === "persistent" ? <SpinnerGap className="spin" aria-hidden="true" /> : status.persistentEnabled ? <Stop aria-hidden="true" /> : <Cloud aria-hidden="true" />}
              {status.persistentEnabled ? "停用 Codex 专用代理" : "启用 Codex 专用代理"}
            </button>
            </footer>
          </div>
          {disclaimerOpen ? (
            <div className="codex-proxy-disclaimer-overlay">
              <section
                aria-labelledby="codex-proxy-disclaimer-title"
                className="codex-proxy-disclaimer"
                role="dialog"
              >
                <div className="codex-proxy-disclaimer-copy">
                  <h3 id="codex-proxy-disclaimer-title">
                    Codex 网络初始化代理使用提示
                  </h3>
                  <p>
                    此功能仅将您自行提供的 Clash/Mihomo 兼容订阅用于 Codex
                    的网络连接。云枢 AI
                    便携工作站不提供、销售或推荐任何代理节点，也无法保证第三方节点的可用性、安全性及隐私保护效果。
                  </p>
                  <p>
                    使用者应确保订阅及节点来源合法，并遵守所在地法律法规、网络服务商规则及相关服务条款。请勿将本功能用于任何违法违规用途；因第三方节点、不当配置或不当使用产生的风险与损失，由使用者自行承担。
                  </p>
                  <p>
                    本功能仅作用于 Codex，不会开启电脑全局代理，也不会修改
                    Windows 系统代理。
                  </p>
                </div>
                <label className="codex-proxy-disclaimer-choice">
                  <input
                    checked={disclaimerAcknowledged}
                    onChange={(event) =>
                      setDisclaimerAcknowledged(event.target.checked)
                    }
                    type="checkbox"
                  />
                  <span>我已阅读并理解上述内容</span>
                </label>
                <label className="codex-proxy-disclaimer-choice">
                  <input
                    checked={skipFutureDisclaimer}
                    onChange={(event) =>
                      setSkipFutureDisclaimer(event.target.checked)
                    }
                    type="checkbox"
                  />
                  <span>本设备后续不再提示</span>
                </label>
                <footer className="codex-proxy-disclaimer-actions">
                  <button
                    className="secondary-action"
                    onClick={onClose}
                    type="button"
                  >
                    取消
                  </button>
                  <button
                    className="primary-action"
                    disabled={!disclaimerAcknowledged}
                    onClick={continueFromDisclaimer}
                    type="button"
                  >
                    继续配置
                  </button>
                </footer>
              </section>
            </div>
          ) : null}
        </div>
      </div>
    </aside>
  );
}

function SettingsDetails({
  children,
  description,
  icon: Icon,
  title,
}: {
  children: ReactNode;
  description: string;
  icon: IconComponent;
  title: string;
}) {
  return (
    <details className="settings-details content-card">
      <summary>
        <span className="settings-icon">
          <Icon aria-hidden="true" />
        </span>
        <span>
          <strong>{title}</strong>
          <small>{description}</small>
        </span>
        <CaretDown className="details-caret" aria-hidden="true" />
      </summary>
      <div className="settings-details-body">{children}</div>
    </details>
  );
}

function ProviderPanel({
  claudeRuntimeDraft,
  credentialPresent,
  draft,
  engine,
  launching,
  stopping,
  onChange,
  onClaudeRuntimeChange,
  onLaunch,
  onSave,
  onStop,
  runtimeState,
  saved,
  workstationRoot,
}: {
  claudeRuntimeDraft?: ClaudeRuntimeDraft;
  credentialPresent: boolean;
  draft: ProviderDraft;
  engine: Engine;
  launching: boolean;
  stopping: boolean;
  onChange: (draft: ProviderDraft) => void;
  onClaudeRuntimeChange: (draft: ClaudeRuntimeDraft) => void;
  onLaunch: () => Promise<void>;
  onSave: () => Promise<void>;
  onStop: () => Promise<void>;
  runtimeState?: EngineState;
  saved: boolean;
  workstationRoot: string;
}) {
  const [showKey, setShowKey] = useState(false);
  const [editingKey, setEditingKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testPassed, setTestPassed] = useState(false);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const discoveryRevision = useRef(0);
  const latestProviderDraft = useRef(draft);
  latestProviderDraft.current = draft;
  const displayedModels = engine.id === "codex" ? draft.availableModels ?? [] : availableModels;
  const [modelMessage, setModelMessage] = useState("");
  const [workspaceMessage, setWorkspaceMessage] = useState("");

  useEffect(() => {
    setShowKey(false);
    setEditingKey(false);
    setTesting(false);
    setTestPassed(false);
    setFetchingModels(false);
    setAvailableModels([]);
    discoveryRevision.current += 1;
    setModelMessage("");
    setWorkspaceMessage("");
    return () => { discoveryRevision.current += 1; };
  }, [engine.id]);

  useEffect(() => {
    if (saved && credentialPresent && !draft.apiKey) {
      setEditingKey(false);
      setShowKey(false);
    }
  }, [credentialPresent, draft.apiKey, saved]);

  const changeField = (field: keyof ProviderDraft, value: string) => {
    const invalidatesModels = field === "baseUrl" || field === "apiKey" || field === "protocol" || field === "provider";
    onChange({ ...draft, [field]: value, ...(engine.id === "codex" && invalidatesModels ? { availableModels: [] } : {}) });
    setTestPassed(false);
    if (invalidatesModels) {
      discoveryRevision.current += 1;
      setFetchingModels(false);
      setAvailableModels([]);
      setModelMessage("");
    }
  };

  const testConnection = async () => {
    setTesting(true);
    setTestPassed(false);
    try {
      if (!saved) await onSave();
      await testProvider(engine.id);
      setTestPassed(true);
    } catch (error) {
      setModelMessage(errorMessage(error));
    } finally {
      setTesting(false);
    }
  };

  const fetchModels = async () => {
    if (
      fetchingModels ||
      !draft.baseUrl ||
      (!draft.apiKey && !credentialPresent)
    ) return;
    try {
      new URL(draft.baseUrl);
    } catch {
      setModelMessage("API 地址格式不正确，请检查后重试。");
      return;
    }

    setFetchingModels(true);
    setModelMessage("");
    const revision = ++discoveryRevision.current;
    try {
      const nextModels = await discoverModels({
        engine: engine.id,
        baseUrl: draft.baseUrl,
        apiKey: draft.apiKey || undefined,
        protocol: protocolToWire(draft.protocol),
      });
      if (revision !== discoveryRevision.current) return;
      const findModel = (family: string) =>
        nextModels.find((model) => model.toLowerCase().includes(family));
      const preferredDefaultModel =
        engine.id === "claude"
          ? findModel("sonnet") ?? nextModels[0]
          : nextModels[0];
      setAvailableModels(nextModels);
      setModelMessage(`已获取 ${nextModels.length} 个可用模型`);
      if (engine.id === "codex") {
        const current = latestProviderDraft.current;
        onChange({ ...current, availableModels: nextModels,
          model: nextModels.includes(current.model) ? current.model : preferredDefaultModel });
      } else if (!nextModels.includes(draft.model)) {
        onChange({ ...draft, model: preferredDefaultModel });
      }
      if (engine.id === "claude" && claudeRuntimeDraft) {
        onClaudeRuntimeChange({
          ...claudeRuntimeDraft,
          opusModel:
            findModel("opus") ??
            claudeRuntimeDraft.opusModel ??
            nextModels[0],
          sonnetModel:
            findModel("sonnet") ??
            claudeRuntimeDraft.sonnetModel ??
            nextModels[0],
          haikuModel:
            findModel("haiku") ??
            claudeRuntimeDraft.haikuModel ??
            nextModels[0],
          subagentModel:
            findModel("sonnet") ??
            claudeRuntimeDraft.subagentModel ??
            nextModels[0],
        });
      }
    } catch (error) {
      if (revision === discoveryRevision.current) setModelMessage(errorMessage(error));
    } finally {
      if (revision === discoveryRevision.current) setFetchingModels(false);
    }
  };

  const changeClaudeRuntime = (
    field: keyof ClaudeRuntimeDraft,
    value: string | boolean,
  ) => {
    if (!claudeRuntimeDraft) return;
    onClaudeRuntimeChange({
      ...claudeRuntimeDraft,
      [field]: value,
    });
  };

  const chooseClaudeWorkspace = async () => {
    try {
      const selected = await choosePortableWorkspace(workstationRoot);
      if (!selected) return;
      changeClaudeRuntime("workspacePath", selected);
      setWorkspaceMessage("已选择移动工作站内的工作目录");
    } catch (error) {
      setWorkspaceMessage(errorMessage(error));
    }
  };

  const recommendedModels = displayedModels.length
    ? displayedModels
    : ["opus", "sonnet", "haiku"];
  const claudeConfigurationReady =
    !claudeRuntimeDraft ||
    (claudeRuntimeDraft.opusModel.trim().length > 0 &&
      claudeRuntimeDraft.sonnetModel.trim().length > 0 &&
      claudeRuntimeDraft.haikuModel.trim().length > 0 &&
      claudeRuntimeDraft.subagentModel.trim().length > 0 &&
      (!claudeRuntimeDraft.chooseWorkspaceOnLaunch ||
        claudeRuntimeDraft.workspacePath.trim().length > 0));
  const savedCredentialMasked = isSavedCredentialMasked(
    draft.apiKey,
    credentialPresent,
    editingKey,
  );
  const displayedApiKey = credentialDisplayValue(
    draft.apiKey,
    credentialPresent,
    editingKey,
  );
  const openClawRunning =
    engine.id === "openclaw" && runtimeState?.status === "running";
  const stopSupported = engineSupportsStop(engine.id);
  const savingPortableData = runtimeState?.status === "saving";
  const stopInProgress = stopping || runtimeState?.status === "stopping";
  const stopEnabled = engineCanStop(engine.id, runtimeState);
  const launchDisabled =
    launching ||
    stopping ||
    savingPortableData ||
    runtimeState?.status === "stopping" ||
    (!openClawRunning &&
      (!draft.baseUrl ||
        (!draft.apiKey && !credentialPresent) ||
        !draft.model ||
        !claudeConfigurationReady));

  return (
    <section
      aria-label={`${engine.name} 供应商配置`}
      className="provider-workspace content-card"
    >
      <header className="provider-workspace-header">
        <div>
          <h2>供应商配置</h2>
          <p>
            当前只编辑 {engine.name}。保存后不会影响另外两个智能体。
          </p>
        </div>
        <span className={`provider-save-state ${saved ? "saved" : ""}`}>
          {saved ? (
            <>
              <CheckCircle aria-hidden="true" weight="fill" />
              已保存
            </>
          ) : (
            "尚未保存"
          )}
        </span>
      </header>

      <div className="provider-workspace-body">
        <div className="provider-form provider-form--inline">
          <label className="form-field">
            <span>供应商名称</span>
            <input
              onChange={(event) => changeField("provider", event.target.value)}
              placeholder="例如 云枢 API"
              value={draft.provider}
            />
          </label>
          <label className="form-field provider-field--wide">
            <span>API 地址</span>
            <input
              onChange={(event) => changeField("baseUrl", event.target.value)}
              placeholder="https://example.com/v1"
              spellCheck={false}
              value={draft.baseUrl}
            />
            <small>填写服务商提供的 Base URL</small>
          </label>
          <label className="form-field provider-field--wide">
            <span>API Key</span>
            <div
              className={`secret-input ${
                savedCredentialMasked ? "secret-input--saved" : ""
              }`}
            >
              <input
                aria-label="API Key"
                onBlur={() => {
                  if (!draft.apiKey) setEditingKey(false);
                }}
                onChange={(event) => {
                  setEditingKey(true);
                  changeField("apiKey", event.target.value);
                }}
                onFocus={() => {
                  if (savedCredentialMasked) {
                    setEditingKey(true);
                    setShowKey(false);
                  }
                }}
                placeholder={credentialPresent ? "输入新密钥" : "粘贴密钥"}
                readOnly={savedCredentialMasked}
                spellCheck={false}
                type={savedCredentialMasked || showKey ? "text" : "password"}
                value={displayedApiKey}
              />
              <button
                aria-label={
                  savedCredentialMasked
                    ? "API Key 已安全保存"
                    : showKey
                      ? "隐藏 API Key"
                      : "显示 API Key"
                }
                disabled={savedCredentialMasked}
                onClick={() => setShowKey((current) => !current)}
                title={
                  savedCredentialMasked
                    ? "已保存的密钥不会从加密存储中回显"
                    : undefined
                }
                type="button"
              >
                {savedCredentialMasked ? (
                  <ShieldCheck aria-hidden="true" weight="fill" />
                ) : showKey ? (
                  <EyeSlash aria-hidden="true" />
                ) : (
                  <Eye aria-hidden="true" />
                )}
              </button>
            </div>
            <small
              className={
                credentialPresent ? "credential-state credential-state--saved" : "credential-state"
              }
            >
              {credentialPresent
                ? "已安全保存；点击输入框可更换密钥"
                : "密钥只会保存到移动盘加密存储中"}
            </small>
          </label>
          <label className="form-field">
            <span>接口协议</span>
            <select
              onChange={(event) => changeField("protocol", event.target.value)}
              value={draft.protocol}
            >
              <option>自动识别（推荐）</option>
              <option>OpenAI Responses</option>
              <option>OpenAI Chat Completions</option>
              <option>Anthropic Messages</option>
            </select>
          </label>
          <div className="form-field model-field">
            <span>默认模型</span>
            <div className="model-picker-row">
              {displayedModels.length ? (
                <select
                  aria-label="默认模型"
                  onChange={(event) => changeField("model", event.target.value)}
                  value={draft.model}
                >
                  {displayedModels.map((model) => (
                    <option key={model} value={model}>
                      {model}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  aria-label="默认模型"
                  onChange={(event) => changeField("model", event.target.value)}
                  placeholder="输入模型名称"
                  value={draft.model}
                />
              )}
              <button
                className="model-fetch-button"
                disabled={
                  fetchingModels ||
                  !draft.baseUrl.trim() ||
                  (!draft.apiKey.trim() && !credentialPresent)
                }
                onClick={() => void fetchModels()}
                type="button"
              >
                {fetchingModels ? (
                  <SpinnerGap className="spin" aria-hidden="true" />
                ) : (
                  <ArrowsClockwise aria-hidden="true" />
                )}
                {fetchingModels ? "获取中" : "自动获取模型"}
              </button>
            </div>
            <small
              className={
                modelMessage.startsWith("已获取")
                  ? "model-message success"
                  : "model-message"
              }
            >
              {modelMessage || "也可以直接手动填写模型名称"}
            </small>
          </div>

          {engine.id === "claude" && claudeRuntimeDraft ? (
            <>
              <section className="claude-model-settings provider-field--wide">
                <header className="claude-section-heading">
                  <span className="claude-section-icon">
                    <Sparkle aria-hidden="true" weight="fill" />
                  </span>
                  <div>
                    <strong>Claude 模型分工</strong>
                    <p>按任务类型指定模型，自动获取后可以直接从列表选择。</p>
                  </div>
                </header>

                <div
                  aria-label="Claude 推荐模型"
                  className="claude-recommended-models"
                >
                  <span>推荐模型</span>
                  <div>
                    {recommendedModels.slice(0, 5).map((model) => (
                      <button
                        className={draft.model === model ? "active" : ""}
                        key={model}
                        onClick={() => changeField("model", model)}
                        type="button"
                      >
                        {model}
                      </button>
                    ))}
                  </div>
                </div>

                <div className="claude-model-grid">
                  <ClaudeModelField
                    availableModels={availableModels}
                    description="复杂推理、规划与高难度任务"
                    label="Opus 模型"
                    onChange={(value) =>
                      changeClaudeRuntime("opusModel", value)
                    }
                    value={claudeRuntimeDraft.opusModel}
                  />
                  <ClaudeModelField
                    availableModels={availableModels}
                    description="日常编码与主要执行任务"
                    label="Sonnet 模型"
                    onChange={(value) =>
                      changeClaudeRuntime("sonnetModel", value)
                    }
                    value={claudeRuntimeDraft.sonnetModel}
                  />
                  <ClaudeModelField
                    availableModels={availableModels}
                    description="轻量、快速和低成本任务"
                    label="Haiku 模型"
                    onChange={(value) =>
                      changeClaudeRuntime("haikuModel", value)
                    }
                    value={claudeRuntimeDraft.haikuModel}
                  />
                  <ClaudeModelField
                    allowInherit
                    availableModels={availableModels}
                    description="Claude 调用子智能体时使用"
                    label="Subagent 模型"
                    onChange={(value) =>
                      changeClaudeRuntime("subagentModel", value)
                    }
                    value={claudeRuntimeDraft.subagentModel}
                  />
                </div>

                <div className="claude-model-hint">
                  <Info aria-hidden="true" weight="fill" />
                  <span>
                    如果供应商只提供一个 Claude
                    兼容模型，四个模型字段可以填写一致；Subagent
                    选择“继承主模型”时跟随默认模型。
                  </span>
                </div>
              </section>

              <section className="claude-workspace-settings provider-field--wide">
                <div className="claude-workspace-copy">
                  <span className="claude-section-icon">
                    <FolderOpen aria-hidden="true" weight="fill" />
                  </span>
                  <div>
                    <strong>工作目录</strong>
                    <p>Claude Code 启动后将在这个目录中读取和修改项目。</p>
                  </div>
                  <label className="claude-workspace-toggle">
                    <input
                      checked={claudeRuntimeDraft.chooseWorkspaceOnLaunch}
                      onChange={(event) =>
                        changeClaudeRuntime(
                          "chooseWorkspaceOnLaunch",
                          event.target.checked,
                        )
                      }
                      type="checkbox"
                    />
                    <span>启动前选择目录</span>
                  </label>
                </div>
                <div className="claude-workspace-picker">
                  <input
                    aria-label="Claude Code 工作目录"
                    readOnly
                    value={claudeRuntimeDraft.workspacePath}
                  />
                  <button
                    disabled={!claudeRuntimeDraft.chooseWorkspaceOnLaunch}
                    onClick={chooseClaudeWorkspace}
                    type="button"
                  >
                    <FolderOpen aria-hidden="true" />
                    选择目录
                  </button>
                </div>
                <small className={workspaceMessage ? "success" : ""}>
                  {workspaceMessage ||
                    (claudeRuntimeDraft.chooseWorkspaceOnLaunch
                      ? "目录必须位于当前移动工作站内，保存时自动转换为相对路径。"
                      : "已关闭目录选择，启动时使用工作站默认目录。")}
                </small>
              </section>
            </>
          ) : null}
        </div>

        <aside
          className={`provider-action-panel ${
            engine.id === "claude" ? "provider-action-panel--compact" : ""
          }`}
        >
          <div className="provider-action-copy">
            <EngineMark engine={engine} large />
            <div>
              <strong>启动 {engine.name}</strong>
              <p>保存当前配置后，从移动盘启动这个智能体。</p>
            </div>
          </div>

          <button
            className={`connection-test ${testPassed ? "passed" : ""}`}
            disabled={
              testing ||
              !draft.baseUrl ||
              (!draft.apiKey && !credentialPresent)
            }
            onClick={() => void testConnection()}
            type="button"
          >
            {testing ? (
              <SpinnerGap className="spin" aria-hidden="true" />
            ) : testPassed ? (
              <CheckCircle aria-hidden="true" weight="fill" />
            ) : (
              <LinkSimple aria-hidden="true" />
            )}
            {testing ? "正在测试连接" : testPassed ? "连接测试通过" : "测试连接"}
          </button>
          <div
            className={`provider-actions ${
              stopSupported ? "provider-actions--with-stop" : ""
            }`}
          >
            <button
              className="secondary-action"
              disabled={stopping}
              onClick={() => void onSave()}
              type="button"
            >
              {saved ? <Check aria-hidden="true" /> : null}
              {saved ? "已保存" : "保存配置"}
            </button>
            <button
              className="primary-action"
              disabled={launchDisabled}
              onClick={() => void onLaunch()}
              type="button"
            >
              {launching ? (
                <SpinnerGap className="spin" aria-hidden="true" />
              ) : (
                <Play aria-hidden="true" weight="fill" />
              )}
              {runtimeActionLabel(engine.id, runtimeState, launching)}
            </button>
            {stopSupported ? (
              <button
              className="danger-action"
                disabled={stopInProgress || savingPortableData || !stopEnabled}
                onClick={() => void onStop()}
                type="button"
              >
                {stopInProgress ? (
                  <SpinnerGap className="spin" aria-hidden="true" />
                ) : savingPortableData ? (
                  <CheckCircle aria-hidden="true" weight="fill" />
                ) : (
                  <Stop aria-hidden="true" weight="fill" />
                )}
                {stopInProgress
                  ? "正在停止"
                  : savingPortableData
                    ? "Codex 已关闭"
                    : engine.id === "codex"
                      ? "停止 Codex"
                      : "停止网关"}
              </button>
            ) : null}
          </div>
          <small>
            {engine.id === "codex" &&
            (launching || runtimeState?.status === "starting") ? (
              <>
                首次使用、恢复出厂后首次启动或更换电脑时，Codex
                需要准备本机运行缓存，启动时间可能较长。请耐心等待，请勿重复点击启动或拔出移动盘；准备完成后，后续启动会明显加快。
                <br />
              </>
            ) : null}
            {engine.id === "openclaw" &&
            (launching || runtimeState?.status === "starting") ? (
              <>
                首次使用、恢复出厂后首次启动或更换电脑时，小龙虾需要准备本机运行缓存，启动可能需要
                1–3 分钟。请勿重复点击启动或拔出移动盘；准备完成后，后续启动会明显加快。
                <br />
              </>
            ) : null}
            {runtimeStatusHint(engine.id, runtimeState, saved)}
          </small>
        </aside>
      </div>
    </section>
  );
}

function ClaudeModelField({
  allowInherit = false,
  availableModels,
  description,
  label,
  onChange,
  value,
}: {
  allowInherit?: boolean;
  availableModels: string[];
  description: string;
  label: string;
  onChange: (value: string) => void;
  value: string;
}) {
  const options = Array.from(
    new Set([
      ...(allowInherit ? ["inherit"] : []),
      ...(value ? [value] : []),
      ...availableModels,
    ]),
  );

  return (
    <label className="claude-model-field">
      <span>
        <strong>{label}</strong>
        <small>{description}</small>
      </span>
      {availableModels.length ? (
        <select onChange={(event) => onChange(event.target.value)} value={value}>
          {options.map((model) => (
            <option key={model} value={model}>
              {model === "inherit" ? "继承主模型" : model}
            </option>
          ))}
        </select>
      ) : (
        <input
          onChange={(event) => onChange(event.target.value)}
          placeholder={allowInherit ? "inherit 或模型名称" : "输入模型名称"}
          value={value}
        />
      )}
    </label>
  );
}

function EngineMark({ engine, large = false }: { engine: Engine; large?: boolean }) {
  return (
    <span
      className={`engine-mark ${large ? "engine-mark--large" : ""}`}
      style={{ "--engine-accent": engine.accent } as CSSProperties}
    >
      <img alt="" src={engine.iconSrc} />
    </span>
  );
}
