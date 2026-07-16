import { CircleArrowUp, CloudOff, RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { t } from "@/i18n";
import {
  EnvironmentRow,
  SectionCard,
} from "@/features/workstation/components";

type UpdateSnapshot = {
  currentVersion: string;
  latestVersion?: string | null;
  updateAvailable?: boolean;
  message: string;
  status: string;
} | null;

export function UpdatesScreen({
  update,
  ignoredVersion,
  onCheck,
  onOpenLegacyDetails,
  onRestoreReminder,
}: {
  update: UpdateSnapshot;
  ignoredVersion: string;
  onCheck: () => void;
  onOpenLegacyDetails: () => void;
  onRestoreReminder: () => void;
}) {
  const currentVersionIgnored =
    Boolean(ignoredVersion) && update?.latestVersion === ignoredVersion;

  return (
    <div className="workstation-page">
      <SectionCard
        action={
          <div className="workstation-action-row compact">
            <Button onClick={onCheck} size="sm" variant="outline">
              <RefreshCw className="h-4 w-4" />
              {t("检查 Codex++ 更新")}
            </Button>
            <Button onClick={onOpenLegacyDetails} size="sm">
              {t("查看原更新详情")}
            </Button>
          </div>
        }
        detail={t("工作站正式版只连接自己的签名更新源，不直接拉取社区仓库")}
        title={t("更新中心")}
      >
        <div className="workstation-environment-list">
          <EnvironmentRow
            detail={t("阶段 H 实现 stable.json、SHA-256、Ed25519 和回滚")}
            status={t("未接入")}
            title={t("便携工作站签名更新源")}
            tone="warning"
          />
          <EnvironmentRow
            detail={
              update
                ? `${t("当前")} ${update.currentVersion} · ${
                    update.latestVersion
                      ? `${t("最新")} ${update.latestVersion}`
                      : update.message
                  }`
                : t("尚未执行 Codex++ 原更新检查")
            }
            status={
              update?.updateAvailable
                ? t("发现更新")
                : update
                  ? t("已检查")
                  : t("未检查")
            }
            title={t("Codex++ 上游更新参考")}
            tone={update?.updateAvailable ? "warning" : update ? "ok" : "neutral"}
          />
          {currentVersionIgnored ? (
            <EnvironmentRow
              detail={t("该版本不会在启动时再次弹出提醒，手动检查仍然可用")}
              status={t("已忽略")}
              title={ignoredVersion}
              tone="neutral"
            />
          ) : null}
          <EnvironmentRow
            detail={t("当前阶段不下载、不替换工作站组件")}
            status={t("禁用")}
            title={t("自动安装与回滚")}
            tone="neutral"
          />
        </div>
        <div className="workstation-update-placeholder">
          <CloudOff className="h-5 w-5" />
          <div>
            <strong>{t("没有连接工作站 Release 服务")}</strong>
            <span>
              {t(
                "这里不会显示演示版本号或假更新成功。正式接口提供后再接入下载、签名验证和回滚。",
              )}
            </span>
          </div>
          <CircleArrowUp className="h-5 w-5" />
        </div>
        {currentVersionIgnored ? (
          <Button
            className="workstation-restore-update"
            onClick={onRestoreReminder}
            size="sm"
            variant="outline"
          >
            {t("恢复此版本提醒")}
          </Button>
        ) : null}
      </SectionCard>
    </div>
  );
}
