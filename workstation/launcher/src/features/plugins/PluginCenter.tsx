import {
  type CSSProperties,
  type FormEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { installPlugin, loadPlugins } from "../../api";
import type { InstallState, PluginCatalog, PluginItem } from "../../types";

const PAGE_SIZE = 24;

interface BatchState {
  active: boolean;
  completed: number;
  total: number;
  logs: string[];
}

const idleBatch: BatchState = {
  active: false,
  completed: 0,
  total: 0,
  logs: [],
};

export function PluginCenter() {
  const [queryInput, setQueryInput] = useState("");
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("");
  const [page, setPage] = useState(1);
  const [catalog, setCatalog] = useState<PluginCatalog | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [refreshKey, setRefreshKey] = useState(0);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [installStates, setInstallStates] = useState<
    Record<string, InstallState>
  >({});
  const [batch, setBatch] = useState<BatchState>(idleBatch);
  const stopBatchRef = useRef(false);

  useEffect(() => {
    const controller = new AbortController();
    const timer = window.setTimeout(async () => {
      setLoading(true);
      setError("");
      try {
        const result = await loadPlugins(
          query,
          category,
          page,
          controller.signal,
        );
        setCatalog(result);
      } catch (reason) {
        if (!controller.signal.aborted) {
          setError(
            reason instanceof Error
              ? reason.message
              : "插件目录暂时无法读取，请重试。",
          );
        }
      } finally {
        if (!controller.signal.aborted) setLoading(false);
      }
    }, 180);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [category, page, query, refreshKey]);

  const updateInstallState = useCallback(
    (id: string, state: InstallState) => {
      setInstallStates((current) => ({ ...current, [id]: state }));
    },
    [],
  );

  const installOne = useCallback(
    async (plugin: PluginItem) => {
      updateInstallState(plugin.id, {
        kind: "installing",
        message: plugin.installed ? "正在重新校验并安装" : "正在从移动盘安装",
      });
      try {
        const result = await installPlugin(plugin.id);
        updateInstallState(plugin.id, {
          kind: "success",
          message: result.nextAction,
        });
        setRefreshKey((value) => value + 1);
        return result.nextAction;
      } catch (reason) {
        const message =
          reason instanceof Error ? reason.message : "安装失败，请稍后重试。";
        updateInstallState(plugin.id, { kind: "error", message });
        throw new Error(message);
      }
    },
    [updateInstallState],
  );

  const runBatch = async () => {
    if (!catalog || batch.active) return;
    const queue = catalog.items.filter((item) => !item.installed);
    if (queue.length === 0) {
      setBatch({
        active: false,
        completed: 0,
        total: 0,
        logs: ["当前页面的插件都已经安装。"],
      });
      return;
    }
    stopBatchRef.current = false;
    setBatch({
      active: true,
      completed: 0,
      total: queue.length,
      logs: [`开始顺序安装 ${queue.length} 个插件。`],
    });
    let completed = 0;
    for (const plugin of queue) {
      if (stopBatchRef.current) break;
      setBatch((current) => ({
        ...current,
        logs: [...current.logs, `正在安装 ${plugin.displayName}…`],
      }));
      try {
        const nextAction = await installOne(plugin);
        completed += 1;
        setBatch((current) => ({
          ...current,
          completed,
          logs: [
            ...current.logs,
            `已完成 ${plugin.displayName}。${nextAction}`,
          ],
        }));
      } catch (reason) {
        setBatch((current) => ({
          ...current,
          logs: [
            ...current.logs,
            `${plugin.displayName} 安装失败：${
              reason instanceof Error ? reason.message : "未知错误"
            }`,
          ],
        }));
      }
    }
    setBatch((current) => ({
      ...current,
      active: false,
      logs: [
        ...current.logs,
        stopBatchRef.current ? "已停止后续安装。" : "当前页安装队列已处理完成。",
      ],
    }));
  };

  const submitSearch = (event: FormEvent) => {
    event.preventDefault();
    setPage(1);
    setQuery(queryInput.trim());
  };

  const totalPages = Math.max(
    1,
    Math.ceil((catalog?.total ?? 0) / PAGE_SIZE),
  );
  const availableCount = useMemo(
    () => catalog?.items.filter((item) => !item.installed).length ?? 0,
    [catalog],
  );

  return (
    <main className="plugin-shell">
      <header className="plugin-header">
        <div>
          <h1>Codex 插件中心</h1>
          <p>
            插件主体已预置在移动盘。安装只写入便携 Codex，不会修改电脑上的个人
            Codex 配置。
          </p>
        </div>
        <div className="source-status" aria-label="插件来源状态">
          <StatusIcon />
          <span>
            <strong>移动盘本地目录</strong>
            <small>离线可浏览与安装</small>
          </span>
        </div>
      </header>

      <section className="catalog-toolbar" aria-label="插件筛选">
        <form className="search-box" onSubmit={submitSearch}>
          <SearchIcon />
          <label className="sr-only" htmlFor="plugin-search">
            搜索插件
          </label>
          <input
            id="plugin-search"
            value={queryInput}
            onChange={(event) => setQueryInput(event.target.value)}
            placeholder="搜索插件名称或能力"
          />
          {queryInput && (
            <button
              className="clear-search"
              type="button"
              aria-label="清空搜索"
              onClick={() => {
                setQueryInput("");
                setQuery("");
                setPage(1);
              }}
            >
              ×
            </button>
          )}
          <button className="search-submit" type="submit">
            搜索
          </button>
        </form>
        <button
          className="batch-button"
          type="button"
          disabled={loading || batch.active || !catalog}
          onClick={runBatch}
        >
          <DownloadIcon />
          安装本页未安装项
          {availableCount > 0 && <span>{availableCount}</span>}
        </button>
      </section>

      <nav className="category-tabs" aria-label="插件分类">
        <button
          type="button"
          className={!category ? "active" : ""}
          aria-pressed={!category}
          onClick={() => {
            setCategory("");
            setPage(1);
          }}
        >
          全部
        </button>
        {catalog?.categories.map((item) => (
          <button
            key={item}
            type="button"
            className={category === item ? "active" : ""}
            aria-pressed={category === item}
            onClick={() => {
              setCategory(item);
              setPage(1);
            }}
          >
            {item}
          </button>
        ))}
      </nav>

      <section className="catalog-content" aria-live="polite">
        <div className="catalog-summary">
          <p>
            {loading
              ? "正在读取移动盘目录…"
              : `找到 ${catalog?.total ?? 0} 个插件`}
          </p>
          <span>仅适用于 Codex</span>
        </div>

        {error && (
          <div className="state-panel error-panel" role="alert">
            <AlertIcon />
            <div>
              <strong>插件目录读取失败</strong>
              <p>{error}</p>
            </div>
            <button type="button" onClick={() => setRefreshKey((key) => key + 1)}>
              重新读取
            </button>
          </div>
        )}

        {loading && !catalog && <PluginListSkeleton />}

        {!loading && !error && catalog?.items.length === 0 && (
          <div className="state-panel empty-panel">
            <SearchIcon />
            <div>
              <strong>没有找到匹配的插件</strong>
              <p>试试更短的关键词，或者切换到“全部”分类。</p>
            </div>
          </div>
        )}

        {catalog && catalog.items.length > 0 && (
          <div className={loading ? "plugin-list refreshing" : "plugin-list"}>
            {catalog.items.map((plugin) => (
              <PluginRow
                key={plugin.id}
                plugin={plugin}
                expanded={expandedId === plugin.id}
                installState={installStates[plugin.id] ?? { kind: "idle" }}
                onToggle={() =>
                  setExpandedId((current) =>
                    current === plugin.id ? null : plugin.id,
                  )
                }
                onInstall={() => void installOne(plugin)}
              />
            ))}
          </div>
        )}

        {catalog && catalog.total > PAGE_SIZE && (
          <div className="pagination" aria-label="插件目录分页">
            <button
              type="button"
              disabled={page <= 1}
              onClick={() => setPage((value) => Math.max(1, value - 1))}
            >
              上一页
            </button>
            <span>
              第 {page} / {totalPages} 页
            </span>
            <button
              type="button"
              disabled={page >= totalPages}
              onClick={() =>
                setPage((value) => Math.min(totalPages, value + 1))
              }
            >
              下一页
            </button>
          </div>
        )}
      </section>

      {(batch.active || batch.logs.length > 0) && (
        <aside className="batch-console" aria-live="polite">
          <div className="batch-console-header">
            <div>
              <strong>当前页安装队列</strong>
              <span>
                {batch.total > 0
                  ? `${batch.completed} / ${batch.total}`
                  : "无需安装"}
              </span>
            </div>
            <div className="batch-actions">
              {batch.active ? (
                <button
                  type="button"
                  onClick={() => {
                    stopBatchRef.current = true;
                  }}
                >
                  停止后续安装
                </button>
              ) : (
                <button type="button" onClick={() => setBatch(idleBatch)}>
                  收起
                </button>
              )}
            </div>
          </div>
          {batch.total > 0 && (
            <div
              className="batch-progress"
              role="progressbar"
              aria-valuenow={batch.completed}
              aria-valuemin={0}
              aria-valuemax={batch.total}
            >
              <span
                style={{
                  transform: `scaleX(${batch.completed / batch.total})`,
                }}
              />
            </div>
          )}
          <div className="batch-logs">
            {batch.logs.map((log, index) => (
              <p key={`${index}-${log}`}>{log}</p>
            ))}
          </div>
        </aside>
      )}
    </main>
  );
}

interface PluginRowProps {
  plugin: PluginItem;
  expanded: boolean;
  installState: InstallState;
  onToggle: () => void;
  onInstall: () => void;
}

function PluginRow({
  plugin,
  expanded,
  installState,
  onToggle,
  onInstall,
}: PluginRowProps) {
  const installing = installState.kind === "installing";
  const style = {
    "--plugin-color": safeBrandColor(plugin.brandColor),
  } as CSSProperties;
  return (
    <article className="plugin-row" style={style}>
      <button
        className="plugin-main"
        type="button"
        aria-expanded={expanded}
        onClick={onToggle}
      >
        <span className="plugin-mark" aria-hidden="true">
          {pluginInitials(plugin.displayName)}
        </span>
        <span className="plugin-copy">
          <span className="plugin-title-line">
            <strong>{plugin.displayName}</strong>
            {plugin.installed && <em>已安装</em>}
          </span>
          <span className="plugin-description">{plugin.shortDescription}</span>
          <span className="plugin-metadata">
            {plugin.category} · v{plugin.version} · {plugin.authenticationLabel}
          </span>
        </span>
        <ChevronIcon expanded={expanded} />
      </button>
      <div className="plugin-action">
        <button type="button" disabled={installing} onClick={onInstall}>
          {installing && <SpinnerIcon />}
          {installing
            ? "安装中"
            : plugin.installed
              ? "重新安装"
              : "安装"}
        </button>
      </div>
      {expanded && (
        <div className="plugin-detail">
          <p>{plugin.longDescription}</p>
          <dl>
            <div>
              <dt>开发者</dt>
              <dd>{plugin.developerName}</dd>
            </div>
            <div>
              <dt>载荷来源</dt>
              <dd>移动盘预置快照</dd>
            </div>
            <div>
              <dt>能力</dt>
              <dd>
                {plugin.capabilities.length > 0
                  ? plugin.capabilities.join("、")
                  : "由插件清单决定"}
              </dd>
            </div>
          </dl>
          <p className="auth-note">
            工作站不会收集第三方账号密码。
            {plugin.authentication === "ON_INSTALL"
              ? "安装完成后，请在 Codex 内按官方流程连接账号。"
              : "此插件无需额外账号连接。"}
          </p>
        </div>
      )}
      {installState.kind !== "idle" && installState.kind !== "installing" && (
        <div className={`install-message ${installState.kind}`} role="status">
          {installState.kind === "success" ? <CheckIcon /> : <AlertIcon />}
          <span>{installState.message}</span>
        </div>
      )}
    </article>
  );
}

export function pluginInitials(name: string): string {
  const normalized = name.trim();
  if (!normalized) return "插件";
  const words = normalized.split(/\s+/).filter(Boolean);
  if (words.length > 1) {
    return `${words[0][0]}${words[1][0]}`.toUpperCase();
  }
  return Array.from(normalized).slice(0, 2).join("").toUpperCase();
}

export function safeBrandColor(value?: string): string {
  return value && /^#[0-9a-f]{6}$/i.test(value) ? value : "#356B5B";
}

function PluginListSkeleton() {
  return (
    <div className="plugin-list skeleton-list" aria-hidden="true">
      {[1, 2, 3, 4].map((item) => (
        <div className="skeleton-row" key={item}>
          <span />
          <div>
            <i />
            <i />
            <i />
          </div>
        </div>
      ))}
    </div>
  );
}

function SearchIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="11" cy="11" r="7" />
      <path d="m16.2 16.2 4 4" />
    </svg>
  );
}

function DownloadIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3v12m0 0 4-4m-4 4-4-4M5 20h14" />
    </svg>
  );
}

function StatusIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 7.5h16v11H4zM8 7.5V5h8v2.5" />
      <path d="m9 13 2 2 4-4" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="m5 12 4 4L19 6" />
    </svg>
  );
}

function AlertIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3 2.8 20h18.4L12 3Z" />
      <path d="M12 9v5m0 3v.1" />
    </svg>
  );
}

function SpinnerIcon() {
  return (
    <svg className="spinner" viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="8" />
    </svg>
  );
}

function ChevronIcon({ expanded }: { expanded: boolean }) {
  return (
    <svg
      className={expanded ? "chevron expanded" : "chevron"}
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path d="m8 10 4 4 4-4" />
    </svg>
  );
}
