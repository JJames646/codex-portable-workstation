import {
  ArrowLeft,
  CircleAlert,
  CircleCheck,
  CloudOff,
  ExternalLink,
  FileCode2,
  Info,
  Network,
  RefreshCw,
} from "lucide-react";
import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { t } from "@/i18n";
import type {
  BusinessServiceAdapter,
  WorkstationCommandStatus,
} from "@/lib/workstation-api";

import "./workstation.css";

export type WorkstationNavigationTarget =
  | "home"
  | "relay"
  | "projects"
  | "sessions"
  | "context"
  | "enhance"
  | "zedRemote"
  | "data"
  | "network"
  | "updates"
  | "maintenance"
  | "settings"
  | "serviceDetail"
  | "userScripts"
  | "recommendations"
  | "relayEnvironment"
  | "about";

export function MetricCard({
  label,
  value,
  detail,
  accent = false,
}: {
  label: string;
  value: string;
  detail: string;
  accent?: boolean;
}) {
  return (
    <article className={`workstation-metric ${accent ? "accent" : ""}`}>
      <span>{label}</span>
      <strong title={value}>{value}</strong>
      <small>{detail}</small>
    </article>
  );
}

export function SectionCard({
  title,
  detail,
  action,
  children,
  className = "",
}: {
  title: string;
  detail?: string;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={`workstation-card ${className}`}>
      <header className="workstation-card-header">
        <div>
          <h2>{title}</h2>
          {detail ? <p>{detail}</p> : null}
        </div>
        {action}
      </header>
      <div className="workstation-card-body">{children}</div>
    </section>
  );
}

export function EnvironmentRow({
  title,
  detail,
  status,
  tone,
  onClick,
}: {
  title: string;
  detail: string;
  status: string;
  tone: "ok" | "warning" | "error" | "neutral";
  onClick?: () => void;
}) {
  const content = (
    <>
      <span className={`workstation-status-dot ${tone}`} />
      <div className="workstation-environment-copy">
        <strong>{title}</strong>
        <span>{detail}</span>
      </div>
      <StatusPill tone={tone}>{status}</StatusPill>
    </>
  );

  return onClick ? (
    <button
      className="workstation-environment-row clickable"
      onClick={onClick}
      type="button"
    >
      {content}
    </button>
  ) : (
    <div className="workstation-environment-row">{content}</div>
  );
}

export function StatusPill({
  tone,
  children,
}: {
  tone: "ok" | "warning" | "error" | "neutral";
  children: ReactNode;
}) {
  return (
    <span className={`workstation-status-pill ${tone}`}>{children}</span>
  );
}

export function SnapshotStatus({
  status,
  loading,
  onRefresh,
}: {
  status: WorkstationCommandStatus;
  loading: boolean;
  onRefresh: () => void;
}) {
  const healthy = status === "ok";
  return (
    <div className={`workstation-snapshot-state ${healthy ? "ok" : "warning"}`}>
      {healthy ? (
        <CircleCheck className="h-4 w-4" />
      ) : (
        <CircleAlert className="h-4 w-4" />
      )}
      <span>
        {healthy
          ? t("本地 Codex++ 数据已加载")
          : t("部分本地数据暂不可用")}
      </span>
      <button disabled={loading} onClick={onRefresh} type="button">
        <RefreshCw className={`h-3.5 w-3.5 ${loading ? "spinning" : ""}`} />
        {loading ? t("检测中") : t("重新检测")}
      </button>
    </div>
  );
}

export function ServiceAdapterRow({
  adapter,
  onOpen,
}: {
  adapter: BusinessServiceAdapter;
  onOpen?: () => void;
}) {
  const content = (
    <>
      <span className="workstation-service-icon">
        <CloudOff className="h-4 w-4" />
      </span>
      <div>
        <strong>{adapter.label}</strong>
        <span>{adapter.message}</span>
      </div>
      <StatusPill tone="warning">{t("未接入")}</StatusPill>
    </>
  );
  return onOpen ? (
    <button
      className="workstation-service-row clickable"
      onClick={onOpen}
      type="button"
    >
      {content}
    </button>
  ) : (
    <div className="workstation-service-row">{content}</div>
  );
}

export function WorkstationModuleLinks({
  onNavigate,
}: {
  onNavigate: (route: WorkstationNavigationTarget) => void;
}) {
  const modules: Array<{
    route: WorkstationNavigationTarget;
    title: string;
    detail: string;
    icon: typeof FileCode2;
  }> = [
    {
      route: "userScripts",
      title: t("脚本市场"),
      detail: t("保留 Codex++ 本地脚本和市场能力"),
      icon: FileCode2,
    },
    {
      route: "recommendations",
      title: t("推荐内容"),
      detail: t("保留 Codex++ 推荐内容页面"),
      icon: ExternalLink,
    },
    {
      route: "relayEnvironment",
      title: t("环境冲突检测"),
      detail: t("检查代理变量、TUN 和 Codex .env"),
      icon: Network,
    },
    {
      route: "about",
      title: t("日志与诊断"),
      detail: t("查看版本、更新参考、日志和诊断"),
      icon: Info,
    },
  ];

  return (
    <section className="workstation-module-links">
      {modules.map((module) => {
        const Icon = module.icon;
        return (
          <button
            key={module.route}
            onClick={() => onNavigate(module.route)}
            type="button"
          >
            <span>
              <Icon className="h-4 w-4" />
            </span>
            <div>
              <strong>{module.title}</strong>
              <small>{module.detail}</small>
            </div>
          </button>
        );
      })}
    </section>
  );
}

export function ServiceDetailScreen({
  onBack,
}: {
  onBack: () => void;
}) {
  return (
    <div className="workstation-page">
      <div className="workstation-detail-toolbar">
        <Button onClick={onBack} variant="outline">
          <ArrowLeft className="h-4 w-4" />
          {t("返回启动器")}
        </Button>
        <StatusPill tone="warning">{t("服务器未接入")}</StatusPill>
      </div>
      <section className="workstation-empty-webview">
        <div>
          <CloudOff className="h-9 w-9" />
          <h2>{t("中转站详情页预留区")}</h2>
          <p>
            {t(
              "阶段 B 不加载远程网站。后续提供正式服务器地址后，这里将使用隔离 WebView 或系统浏览器打开完整网站。",
            )}
          </p>
        </div>
      </section>
    </div>
  );
}
