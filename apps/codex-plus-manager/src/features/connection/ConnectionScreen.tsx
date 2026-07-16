import { CloudOff, Power, PowerOff, ShieldCheck } from "lucide-react";

import { Button } from "@/components/ui/button";
import { t } from "@/i18n";
import type { WorkstationSnapshot } from "@/lib/workstation-api";
import {
  SectionCard,
  ServiceAdapterRow,
} from "@/features/workstation/components";

export function ConnectionScreen({
  snapshot,
}: {
  snapshot: WorkstationSnapshot;
}) {
  return (
    <div className="workstation-page workstation-two-column">
      <SectionCard
        detail={t("只作用于本工作站启动的 Codex，不修改 Windows 全局代理")}
        title={t("便携工作站服务连接")}
      >
        <div className="workstation-connection-flow">
          <span className="active">Codex</span>
          <i />
          <span>{t("工作站连接")}</span>
          <i />
          <span className="not-connected">
            <CloudOff className="h-4 w-4" />
            {t("未接入")}
          </span>
        </div>
        <div className="workstation-service-list">
          <ServiceAdapterRow adapter={snapshot.services.relay} />
          <ServiceAdapterRow adapter={snapshot.services.control} />
        </div>
        <div className="workstation-action-row">
          <Button disabled>
            <Power className="h-4 w-4" />
            {t("启动服务")}
          </Button>
          <Button className="workstation-stop-button" disabled variant="outline">
            <PowerOff className="h-4 w-4" />
            {t("停止服务")}
          </Button>
        </div>
        <p className="workstation-phase-note">
          {t("阶段 B 不提供假连接、假延迟、假卡密或假到期时间。")}
        </p>
      </SectionCard>

      <SectionCard title={t("合规使用声明")}>
        <div className="workstation-compliance">
          <ShieldCheck className="h-5 w-5" />
          <p>
            {t(
              "本服务仅用于提升 Codex 便携工作站在合法场景下连接本产品自有 API 服务的稳定性，不会修改电脑系统网络设置，也不会影响电脑上的其他软件。用户应遵守所在地法律法规、平台规则及第三方服务协议，不得将本服务用于破解、攻击、非法侵入、绕过权限限制、窃取数据、传播违法信息、侵犯他人隐私或知识产权等违法违规用途。因用户自行配置或滥用产生的法律责任、账号风险和数据风险由用户自行承担。",
            )}
          </p>
        </div>
      </SectionCard>
    </div>
  );
}
