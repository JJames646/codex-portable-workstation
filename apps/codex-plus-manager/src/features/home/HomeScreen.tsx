import {
  Activity,
  CircleDollarSign,
  Gauge,
  KeyRound,
  LogIn,
  LogOut,
  RefreshCw,
  WalletCards,
} from "lucide-react";
import { useState, type FormEvent, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { t } from "@/i18n";
import type { WorkstationSnapshot } from "@/lib/workstation-api";
import {
  EnvironmentRow,
  MetricCard,
  SectionCard,
  StatusPill,
  type WorkstationNavigationTarget,
} from "@/features/workstation/components";

export type RelayDashboardStats = {
  total_api_keys: number;
  active_api_keys: number;
  total_requests: number;
  total_tokens: number;
  total_actual_cost: number;
  today_requests: number;
  today_tokens: number;
  today_actual_cost: number;
  average_duration_ms: number;
  rpm: number;
  tpm: number;
};

export type RelayTrendPoint = {
  date: string;
  requests: number;
  total_tokens: number;
  actual_cost: number;
};

export type RelaySubscriptionItem = {
  group_name: string;
  status: string;
  daily_used_usd: number;
  daily_limit_usd: number;
  weekly_used_usd: number;
  weekly_limit_usd: number;
  monthly_used_usd: number;
  monthly_limit_usd: number;
  expires_at: string | null;
};

export type RelayDashboardResult = {
  status: string;
  message: string;
  configured: boolean;
  authStatus: "required" | "authenticated" | "unavailable" | string;
  accountLabel: string | null;
  balance: number;
  stats: RelayDashboardStats | null;
  trend: RelayTrendPoint[];
  models: Array<{
    model: string;
    requests: number;
    total_tokens: number;
    actual_cost: number;
  }>;
  subscription: {
    active_count: number;
    total_used_usd: number;
    subscriptions: RelaySubscriptionItem[];
  } | null;
};

export function HomeScreen({
  snapshot,
  onNavigate,
  relayDashboard,
  relayLoading,
  onRelayLogin,
  onRelayLogout,
  onRelayRefresh,
}: {
  snapshot: WorkstationSnapshot;
  onNavigate: (route: WorkstationNavigationTarget) => void;
  relayDashboard: RelayDashboardResult | null;
  relayLoading: boolean;
  onRelayLogin: (email: string, password: string) => Promise<boolean>;
  onRelayLogout: () => void;
  onRelayRefresh: () => void;
}) {
  const [relayEmail, setRelayEmail] = useState("");
  const [relayPassword, setRelayPassword] = useState("");
  const launch = snapshot.overview?.latest_launch;
  const codexRunning = launch?.status === "running";
  const workspace = snapshot.sessions.find((session) => session.cwd.trim())?.cwd;
  const relayAuthenticated =
    relayDashboard?.authStatus === "authenticated" &&
    relayDashboard.stats !== null;
  const relayStats = relayDashboard?.stats;
  const quota = selectRelayQuota(relayDashboard);
  const usageBars = buildRelayUsageBars(relayDashboard?.trend ?? []);
  const relayEnvironmentHealthy =
    snapshot.relayEnvironment !== null &&
    !snapshot.relayEnvironment.clashVergeTun.enabled &&
    snapshot.relayEnvironment.proxyEnvironment.variables.length === 0 &&
    !snapshot.relayEnvironment.codexEnvFile.exists;
  const submitRelayLogin = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const loggedIn = await onRelayLogin(relayEmail, relayPassword);
    if (loggedIn) setRelayPassword("");
  };

  return (
    <div className="workstation-page">
      <section className="workstation-metric-grid">
        <MetricCard
          accent
          detail={t("便携根目录将在阶段 C 从当前 EXE 自动推导")}
          label={t("当前工作区")}
          value={workspace || snapshot.settings?.settings_path || t("等待本地数据")}
        />
        <MetricCard
          detail={launch?.message || t("通过工作站统一启动 Codex")}
          label={t("Codex 状态")}
          value={codexRunning ? t("运行中") : t("未启动")}
        />
        <MetricCard
          detail={snapshot.services.control.message}
          label={t("网络环境")}
          value={t("未接入")}
        />
      </section>

      <section className="workstation-home-grid">
        <SectionCard
          action={
            <Button
              disabled={!relayAuthenticated}
              onClick={() => onNavigate("serviceDetail")}
              size="sm"
            >
              {t("查看详情")}
            </Button>
          }
          className="workstation-dashboard-card"
          detail={t("登录后自动读取当前中转站账号的余额、用量和套餐额度")}
          title={t("中转站用量仪表盘")}
        >
          <div className="workstation-dashboard-frame">
            <div className="workstation-dashboard-head">
              <div>
                <span className="workstation-dashboard-icon">
                  <Gauge className="h-5 w-5" />
                </span>
                <div>
                  <strong>
                    {relayAuthenticated
                      ? relayDashboard?.accountLabel || t("当前账号")
                      : t("需要登录中转站")}
                  </strong>
                  <small>
                    {relayAuthenticated
                      ? t("数据来自当前账号的 Sub2API 用户接口。")
                      : t("登录后才显示个人消耗额度，未登录不会读取任何用户数据。")}
                  </small>
                </div>
              </div>
              <StatusPill tone={relayAuthenticated ? "ok" : "warning"}>
                {relayLoading
                  ? t("读取中")
                  : relayAuthenticated
                    ? t("已登录")
                    : t("需要登录")}
              </StatusPill>
            </div>
            {relayAuthenticated ? (
              <>
                <div className="workstation-dashboard-metrics">
                  <DashboardMetric
                    icon={<WalletCards className="h-4 w-4" />}
                    label={t("可用余额")}
                    value={formatMoney(relayDashboard?.balance ?? 0)}
                  />
                  <DashboardMetric
                    icon={<Activity className="h-4 w-4" />}
                    label={t("今日请求")}
                    value={formatCount(relayStats?.today_requests ?? 0)}
                  />
                  <DashboardMetric
                    icon={<CircleDollarSign className="h-4 w-4" />}
                    label={t("今日实际消耗")}
                    value={formatMoney(relayStats?.today_actual_cost ?? 0)}
                  />
                </div>
                <div className="workstation-usage-strip">
                  <div>
                    <Gauge className="h-4 w-4" />
                    <span>{t("今日 Tokens")}</span>
                    <strong>{formatCompact(relayStats?.today_tokens ?? 0)}</strong>
                  </div>
                  <div>
                    <CircleDollarSign className="h-4 w-4" />
                    <span>{t("累计消耗")}</span>
                    <strong>{formatMoney(relayStats?.total_actual_cost ?? 0)}</strong>
                  </div>
                  <div>
                    <KeyRound className="h-4 w-4" />
                    <span>{t("API Keys")}</span>
                    <strong>
                      {relayStats?.active_api_keys ?? 0}/{relayStats?.total_api_keys ?? 0}
                    </strong>
                  </div>
                  <div>
                    <RefreshCw className="h-4 w-4" />
                    <span>{t("近 5 分钟")}</span>
                    <strong>{relayStats?.rpm ?? 0} RPM</strong>
                  </div>
                </div>
                <div className="workstation-relay-quota">
                  <div>
                    <span>{quota.label}</span>
                    <strong>
                      {formatMoney(quota.used)} / {quota.limit > 0 ? formatMoney(quota.limit) : t("不限额")}
                    </strong>
                  </div>
                  <div className="workstation-relay-quota-track">
                    <i style={{ width: `${quota.percent}%` }} />
                  </div>
                  <small>
                    {quota.expiresAt
                      ? `${t("到期时间")}：${formatDate(quota.expiresAt)}`
                      : t("当前套餐未返回到期时间")}
                  </small>
                </div>
                <div
                  aria-label={t("个人消耗趋势")}
                  className="workstation-activity-chart workstation-relay-usage-chart"
                >
                  {usageBars.map((bar) => (
                    <i
                      key={bar.label}
                      style={{ height: `${bar.height}%` }}
                      title={`${bar.label} · ${formatMoney(bar.cost)} · ${formatCompact(bar.tokens)} Tokens`}
                    />
                  ))}
                </div>
                <div className="workstation-relay-dashboard-actions">
                  <Button onClick={onRelayRefresh} size="sm" variant="outline">
                    <RefreshCw className="h-4 w-4" />
                    {t("刷新用量")}
                  </Button>
                  <Button onClick={onRelayLogout} size="sm" variant="outline">
                    <LogOut className="h-4 w-4" />
                    {t("退出中转站")}
                  </Button>
                </div>
              </>
            ) : (
              <form
                className="workstation-relay-login"
                onSubmit={submitRelayLogin}
              >
                <span className="workstation-relay-login-icon">
                  <LogIn className="h-6 w-6" />
                </span>
                <div>
                  <strong>{t("登录后查看个人消耗额度")}</strong>
                  <p>{t("账号密码只用于本次登录，密码不会保存到工作站。")}</p>
                </div>
                <label>
                  <span>{t("邮箱")}</span>
                  <input
                    autoComplete="username"
                    onChange={(event) => setRelayEmail(event.target.value)}
                    placeholder={t("请输入中转站邮箱")}
                    type="email"
                    value={relayEmail}
                  />
                </label>
                <label>
                  <span>{t("密码")}</span>
                  <input
                    autoComplete="current-password"
                    onChange={(event) => setRelayPassword(event.target.value)}
                    placeholder={t("请输入中转站密码")}
                    type="password"
                    value={relayPassword}
                  />
                </label>
                <Button
                  disabled={relayLoading || !relayEmail || !relayPassword}
                  type="submit"
                >
                  <LogIn className="h-4 w-4" />
                  {relayLoading ? t("登录中") : t("登录中转站")}
                </Button>
              </form>
            )}
          </div>
        </SectionCard>

        <SectionCard
          detail={t("只有未配置或异常的项目可以点击进入对应页面")}
          title={t("运行状态与环境校验")}
        >
          <div className="workstation-environment-list">
            <EnvironmentRow
              detail={snapshot.overview?.codex_app.path || t("尚未配置 Codex 应用路径")}
              onClick={
                snapshot.overview?.codex_app.path
                  ? undefined
                  : () => onNavigate("maintenance")
              }
              status={snapshot.overview?.codex_app.path ? t("已安装") : t("未配置")}
              title={t("Codex 主程序")}
              tone={snapshot.overview?.codex_app.path ? "ok" : "error"}
            />
            <EnvironmentRow
              detail={snapshot.settings?.settings_path || t("设置接口暂不可用")}
              status={snapshot.settings ? t("已加载") : t("未加载")}
              title={t("Codex++ 本地设置")}
              tone={snapshot.settings ? "ok" : "error"}
            />
            <EnvironmentRow
              detail={t("阶段 C 实现 EXE 根目录发现和 CODEXPP_HOME")}
              onClick={() => onNavigate("data")}
              status={t("待开发")}
              title={t("便携数据目录")}
              tone="warning"
            />
            <EnvironmentRow
              detail={
                relayEnvironmentHealthy
                  ? t("未发现代理变量、TUN 或 .env 冲突")
                  : t("需要查看 Codex++ 环境冲突检测")
              }
              onClick={
                relayEnvironmentHealthy
                  ? undefined
                  : () => onNavigate("relayEnvironment")
              }
              status={relayEnvironmentHealthy ? t("正常") : t("需检查")}
              title={t("本机网络环境")}
              tone={relayEnvironmentHealthy ? "ok" : "warning"}
            />
            <EnvironmentRow
              detail={snapshot.services.control.message}
              onClick={() => onNavigate("network")}
              status={t("未接入")}
              title={t("工作站服务连接")}
              tone="error"
            />
          </div>
          {snapshot.errors.length ? (
            <div className="workstation-error-list">
              {snapshot.errors.map((error) => (
                <p key={error}>{error}</p>
              ))}
            </div>
          ) : null}
          <Button
            className="workstation-primary-wide"
            onClick={() => onNavigate("network")}
          >
            {t("开始环境配置")}
          </Button>
        </SectionCard>
      </section>
    </div>
  );
}

function DashboardMetric({
  icon,
  label,
  value,
}: {
  icon: ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div>
      <span>{icon}</span>
      <small>{label}</small>
      <strong title={value}>{value}</strong>
    </div>
  );
}

function buildRelayUsageBars(trend: RelayTrendPoint[]) {
  const points = trend.slice(-12);
  const normalized = points.length
    ? points
    : Array.from({ length: 12 }, (_, index) => ({
        date: `${index + 1}`,
        requests: 0,
        total_tokens: 0,
        actual_cost: 0,
      }));
  const max = Math.max(
    ...normalized.map((point) => point.actual_cost || point.total_tokens),
    1,
  );
  return normalized.map((point) => {
    const value = point.actual_cost || point.total_tokens;
    return {
      label: point.date,
      cost: point.actual_cost,
      tokens: point.total_tokens,
      height: value ? Math.max(14, Math.round((value / max) * 100)) : 8,
    };
  });
}

function selectRelayQuota(relay: RelayDashboardResult | null) {
  const item = relay?.subscription?.subscriptions[0];
  if (!item) {
    return {
      label: t("当前套餐额度"),
      used: relay?.subscription?.total_used_usd ?? 0,
      limit: 0,
      percent: 0,
      expiresAt: null as string | null,
    };
  }
  const windows = [
    {
      label: t("月度套餐额度"),
      used: item.monthly_used_usd,
      limit: item.monthly_limit_usd,
    },
    {
      label: t("每周套餐额度"),
      used: item.weekly_used_usd,
      limit: item.weekly_limit_usd,
    },
    {
      label: t("每日套餐额度"),
      used: item.daily_used_usd,
      limit: item.daily_limit_usd,
    },
  ];
  const selected = windows.find((window) => window.limit > 0) ?? windows[0];
  return {
    ...selected,
    percent:
      selected.limit > 0
        ? Math.min(100, Math.max(0, (selected.used / selected.limit) * 100))
        : 0,
    expiresAt: item.expires_at,
  };
}

function formatMoney(value: number) {
  return `$${Number.isFinite(value) ? value.toFixed(value >= 100 ? 0 : 2) : "0.00"}`;
}

function formatCount(value: number) {
  return new Intl.NumberFormat("zh-CN").format(value || 0);
}

function formatCompact(value: number) {
  return new Intl.NumberFormat("zh-CN", {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value || 0);
}

function formatDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleDateString("zh-CN");
}
