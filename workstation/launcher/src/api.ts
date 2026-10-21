import { mockCatalog } from "./mockCatalog";
import type {
  ApiEnvelope,
  PluginCatalog,
  PluginInstallResult,
} from "./types";

const useMocks = import.meta.env.VITE_PLUGIN_API_MODE === "mock";

async function request<T>(
  url: string,
  init?: RequestInit,
  signal?: AbortSignal,
): Promise<T> {
  const response = await fetch(url, { ...init, signal });
  const payload = (await response.json()) as ApiEnvelope<T>;
  if (!response.ok || !payload.ok) {
    throw new Error(payload.error || `请求失败（HTTP ${response.status}）`);
  }
  return payload.data;
}

export async function loadPlugins(
  query: string,
  category: string,
  page: number,
  signal?: AbortSignal,
): Promise<PluginCatalog> {
  if (useMocks) {
    await new Promise((resolve) => window.setTimeout(resolve, 180));
    const normalizedQuery = query.trim().toLowerCase();
    const items = mockCatalog.items.filter(
      (item) =>
        (!normalizedQuery ||
          item.id.toLowerCase().includes(normalizedQuery) ||
          item.displayName.toLowerCase().includes(normalizedQuery) ||
          item.shortDescription.toLowerCase().includes(normalizedQuery)) &&
        (!category || item.category === category),
    );
    return { ...mockCatalog, items, total: items.length, page };
  }
  const search = new URLSearchParams({
    query,
    category,
    page: String(page),
    pageSize: "24",
  });
  return request<PluginCatalog>(`/api/plugins?${search.toString()}`, undefined, signal);
}

export async function installPlugin(
  pluginId: string,
): Promise<PluginInstallResult> {
  if (useMocks) {
    await new Promise((resolve) => window.setTimeout(resolve, 620));
    const plugin = mockCatalog.items.find((item) => item.id === pluginId);
    if (!plugin) throw new Error("没有找到这个插件");
    return {
      id: plugin.id,
      version: plugin.version,
      installed: true,
      reinstalled: plugin.installed,
      installPath: `移动盘/data/.codex/plugins/cache/yunshu_plugins/${plugin.id}/${plugin.version}`,
      packageSha256: "mock",
      authentication: plugin.authentication,
      nextAction:
        plugin.authentication === "ON_INSTALL"
          ? "已安装；请启动 Codex 后在插件页面连接对应第三方账号。"
          : "已安装；重新进入 Codex 会话后即可使用。",
    };
  }
  return request<PluginInstallResult>(`/api/plugins/${pluginId}/install`, {
    method: "POST",
  });
}
