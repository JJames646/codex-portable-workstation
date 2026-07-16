import { FolderKanban } from "lucide-react";

import { t } from "@/i18n";
import {
  groupSessionsByProject,
  type WorkstationSnapshot,
} from "@/lib/workstation-api";
import {
  SectionCard,
  StatusPill,
} from "@/features/workstation/components";

export function ProjectsScreen({
  snapshot,
}: {
  snapshot: WorkstationSnapshot;
}) {
  const projects = groupSessionsByProject(snapshot.sessions);

  return (
    <div className="workstation-page">
      <SectionCard
        detail={t("项目根据 Codex 本地会话中的真实 cwd 自动聚合，不显示演示项目")}
        title={t("项目管理")}
      >
        {projects.length ? (
          <div className="workstation-project-table">
            <div className="workstation-project-head">
              <span>{t("项目")}</span>
              <span>{t("会话")}</span>
              <span>{t("供应商")}</span>
              <span>{t("最近活动")}</span>
            </div>
            {projects.map((project) => (
              <article key={project.cwd}>
                <div>
                  <span className="workstation-project-icon">
                    <FolderKanban className="h-4 w-4" />
                  </span>
                  <div>
                    <strong>{project.name}</strong>
                    <small title={project.cwd}>{project.cwd}</small>
                  </div>
                </div>
                <strong>{project.sessionCount}</strong>
                <span>
                  {project.providers.length
                    ? project.providers.join(" / ")
                    : t("未记录")}
                </span>
                <StatusPill tone="neutral">
                  {formatActivity(project.latestActivityMs)}
                </StatusPill>
              </article>
            ))}
          </div>
        ) : (
          <div className="workstation-empty-state">
            <FolderKanban className="h-8 w-8" />
            <h3>{t("尚未发现可聚合项目")}</h3>
            <p>
              {snapshot.status === "unavailable"
                ? t("请在 Tauri 客户端中重新检测本地会话。")
                : t("Codex 会话中出现 cwd 后，项目会自动显示在这里。")}
            </p>
          </div>
        )}
      </SectionCard>
    </div>
  );
}

function formatActivity(value: number | null): string {
  return value ? new Date(value).toLocaleString("zh-CN") : t("未记录");
}
