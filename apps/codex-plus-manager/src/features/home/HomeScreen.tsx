import {
  Box,
  Boxes,
  FolderKanban,
  Gauge,
  KeyRound,
  PlugZap,
} from "lucide-react";
import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { t } from "@/i18n";
import type { WorkstationSnapshot } from "@/lib/workstation-api";
import {
  EnvironmentRow,
  MetricCard,
  SectionCard,
  ServiceAdapterRow,
  StatusPill,
  type WorkstationNavigationTarget,
} from "@/features/workstation/components";

export function HomeScreen({
  snapshot,
  loading,
  onNavigate,
}: {
  snapshot: WorkstationSnapshot;
  loading: boolean;
  onNavigate: (route: WorkstationNavigationTarget) => void;
}) {
  const launch = snapshot.overview?.latest_launch;
  const codexRunning = launch?.status === "running";
  const activeProvider = activeProviderName(snapshot);
  const workspace = snapshot.sessions.find((session) => session.cwd.trim())?.cwd;
  const projectCount = new Set(
    snapshot.sessions
      .map((session) => session.cwd.trim())
      .filter(Boolean),
  ).size;
  const activityBars = buildActivityBars(snapshot);
  const relayEnvironmentHealthy =
    snapshot.relayEnvironment !== null &&
    !snapshot.relayEnvironment.clashVergeTun.enabled &&
    snapshot.relayEnvironment.proxyEnvironment.variables.length === 0 &&
    !snapshot.relayEnvironment.codexEnvFile.exists;

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
            <Button onClick={() => onNavigate("serviceDetail")} size="sm">
              {t("查看详情")}
            </Button>
          }
          className="workstation-dashboard-card"
          detail={t("数据直接来自 Codex++ 本地 Overview、设置、会话和扩展接口")}
          title={t("Codex++ 概览仪表盘")}
        >
          <div className="workstation-dashboard-frame">
            <div className="workstation-dashboard-head">
              <div>
                <span className="workstation-dashboard-icon">
                  <Gauge className="h-5 w-5" />
                </span>
                <div>
                  <strong>{t("Codex++ 能力与工作站状态")}</strong>
                  <small>{t("无需登录账号，直接读取当前 Codex++ 本地数据。")}</small>
                </div>
              </div>
              <StatusPill tone={snapshot.status === "ok" ? "ok" : "warning"}>
                {loading
                  ? t("检测中")
                  : snapshot.status === "ok"
                    ? t("本地数据正常")
                    : t("部分数据不可用")}
              </StatusPill>
            </div>
            <div className="workstation-dashboard-metrics">
              <DashboardMetric
                icon={<Box className="h-4 w-4" />}
                label={t("Codex 版本")}
                value={snapshot.overview?.codex_version || t("未检测到")}
              />
              <DashboardMetric
                icon={<FolderKanban className="h-4 w-4" />}
                label={t("项目与会话")}
                value={`${projectCount} / ${snapshot.sessions.length}`}
              />
              <DashboardMetric
                icon={<KeyRound className="h-4 w-4" />}
                label={t("供应商")}
                value={activeProvider}
              />
            </div>
            <div className="workstation-usage-strip">
              <div>
                <PlugZap className="h-4 w-4" />
                <span>{t("插件")}</span>
                <strong>{snapshot.contextCounts.plugins}</strong>
              </div>
              <div>
                <Boxes className="h-4 w-4" />
                <span>{t("Skills")}</span>
                <strong>{snapshot.contextCounts.skills}</strong>
              </div>
              <div>
                <FolderKanban className="h-4 w-4" />
                <span>{t("会话索引")}</span>
                <strong>{snapshot.sessions.length ? t("正常") : t("暂无")}</strong>
              </div>
              <div>
                <FolderKanban className="h-4 w-4" />
                <span>{t("远程项目")}</span>
                <strong>{snapshot.zedProjectCount}</strong>
              </div>
            </div>
            <div
              aria-label={t("最近十二天会话活动")}
              className="workstation-activity-chart"
            >
              {activityBars.map((bar) => (
                <i
                  key={bar.label}
                  style={{ height: `${bar.height}%` }}
                  title={`${bar.label} · ${bar.count}`}
                />
              ))}
            </div>
          </div>
          <div className="workstation-service-list">
            <ServiceAdapterRow
              adapter={snapshot.services.relay}
              onOpen={() => onNavigate("serviceDetail")}
            />
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

function buildActivityBars(snapshot: WorkstationSnapshot) {
  const dayMs = 24 * 60 * 60 * 1000;
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const counts = Array.from({ length: 12 }, () => 0);

  for (const session of snapshot.sessions) {
    if (!session.updatedAtMs) continue;
    const age = Math.floor(
      (today.getTime() - new Date(session.updatedAtMs).setHours(0, 0, 0, 0)) /
        dayMs,
    );
    if (age >= 0 && age < counts.length) {
      counts[counts.length - 1 - age] += 1;
    }
  }

  const max = Math.max(...counts, 1);
  return counts.map((count, index) => {
    const day = new Date(today.getTime() - (counts.length - 1 - index) * dayMs);
    return {
      count,
      height: count ? Math.max(18, Math.round((count / max) * 100)) : 8,
      label: day.toLocaleDateString("zh-CN"),
    };
  });
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

function activeProviderName(snapshot: WorkstationSnapshot): string {
  const settings = snapshot.settings?.settings;
  if (!settings) return t("未加载");
  const active = settings.relayProfiles.find(
    (profile) => profile.id === settings.activeRelayId,
  );
  return active?.name || t("未配置");
}
