import { Database, FileText, PlugZap, Unplug } from "lucide-react";
import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { t } from "@/i18n";
import type { WorkstationSnapshot } from "@/lib/workstation-api";
import {
  EnvironmentRow,
  SectionCard,
} from "@/features/workstation/components";

export function DataScreen({
  snapshot,
}: {
  snapshot: WorkstationSnapshot;
}) {
  const dbPaths = Array.from(
    new Set(snapshot.sessions.map((session) => session.dbPath).filter(Boolean)),
  );
  const scriptCount =
    snapshot.settings?.user_scripts.scripts?.filter((script) => script.enabled)
      .length ?? 0;
  const extensionCount =
    snapshot.contextCounts.mcpServers +
    snapshot.contextCounts.skills +
    snapshot.contextCounts.plugins;

  return (
    <div className="workstation-page workstation-two-column">
      <SectionCard
        detail={t("当前阶段只展示 Codex++ 真实路径；阶段 C 再迁移到移动盘目录")}
        title={t("数据与备份")}
      >
        <div className="workstation-data-list">
          <DataItem
            detail={snapshot.settings?.settings_path || t("未读取")}
            icon={<FileText className="h-4 w-4" />}
            label={t("Codex++ 设置")}
            value={snapshot.settings ? t("已加载") : t("不可用")}
          />
          <DataItem
            detail={dbPaths.join("\n") || t("尚未发现会话数据库")}
            icon={<Database className="h-4 w-4" />}
            label={t("会话数据库")}
            value={String(dbPaths.length)}
          />
          <DataItem
            detail={t("MCP、Skills、Plugins 的本地启用状态")}
            icon={<PlugZap className="h-4 w-4" />}
            label={t("扩展配置")}
            value={String(extensionCount)}
          />
          <DataItem
            detail={t("来自 Codex++ 用户脚本清单")}
            icon={<FileText className="h-4 w-4" />}
            label={t("已启用脚本")}
            value={String(scriptCount)}
          />
        </div>
      </SectionCard>

      <SectionCard
        detail={t("安全弹出需要阶段 C 的路径布局和阶段 D 的进程状态机")}
        title={t("安全弹出")}
      >
        <div className="workstation-environment-list">
          <EnvironmentRow
            detail={t("阶段 D 实现受控停止和 PID 树等待")}
            status={t("待开发")}
            title={t("关闭工作站进程")}
            tone="warning"
          />
          <EnvironmentRow
            detail={t("阶段 C/D 确认 SQLite WAL、SHM 和文件句柄释放")}
            status={t("待开发")}
            title={t("刷新并释放数据")}
            tone="warning"
          />
          <EnvironmentRow
            detail={t("阶段 G 才在真实测试移动盘执行")}
            status={t("不可用")}
            title={t("弹出移动盘")}
            tone="neutral"
          />
        </div>
        <Button className="workstation-disabled-action" disabled>
          <Unplug className="h-4 w-4" />
          {t("安全弹出 U盘")}
        </Button>
      </SectionCard>
    </div>
  );
}

function DataItem({
  icon,
  label,
  detail,
  value,
}: {
  icon: ReactNode;
  label: string;
  detail: string;
  value: string;
}) {
  return (
    <article>
      <span>{icon}</span>
      <div>
        <strong>{label}</strong>
        <small title={detail}>{detail}</small>
      </div>
      <b>{value}</b>
    </article>
  );
}
