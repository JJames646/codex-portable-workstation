import { describe, expect, it } from "vitest";

import {
  engineCanStop,
  engineSupportsStop,
  codexLocaleStateLabel,
  normalizeSkillItems,
  shouldSaveBeforeLaunch,
  runtimeActionLabel,
  runtimeStatusHint,
  type EngineState,
} from "./workstationApi";

describe("Codex 中文状态提示", () => {
  it("区分未初始化、待验证、已验证和版本不匹配", () => {
    expect(codexLocaleStateLabel("missing")).toBe("官方中文功能状态尚未初始化");
    expect(codexLocaleStateLabel("writtenPendingVerification")).toBe(
      "中文配置已写入，请启动 Codex 验证",
    );
    expect(codexLocaleStateLabel("verified")).toBe("中文缓存已生效");
    expect(codexLocaleStateLabel("versionMismatch")).toBe(
      "中文种子与当前 Codex 版本不匹配",
    );
  });
});

describe("SkillHub 中文介绍映射", () => {
  it("保留原始英文标题，将中文详情直接作为卡片介绍", () => {
    const [skill] = normalizeSkillItems({
      items: [
        {
          namespace: "root",
          slug: "find-skills",
          name: "Find Skills",
          description: "Discover useful skills for a task.",
          description_zh: "根据任务目标查找并推荐合适的 Skill。",
          version: "1.0.0",
        },
      ],
    });

    expect(skill.name).toBe("Find Skills");
    expect(skill.description).toBe("根据任务目标查找并推荐合适的 Skill。");
  });

  it("原本就是中文的标题和介绍保持不变", () => {
    const [skill] = normalizeSkillItems({
      items: [
        {
          namespace: "root",
          slug: "documents",
          name: "文档处理",
          description_zh: "创建、读取和修改 Word 文档。",
        },
      ],
    });

    expect(skill.name).toBe("文档处理");
    expect(skill.description).toBe("创建、读取和修改 Word 文档。");
  });

  it("supports the object namespace returned by the current SkillHub API", () => {
    const [skill] = normalizeSkillItems({
      items: [
        {
          namespace: {
            canonicalName:
              "@org-28ib33ph/lingyi-wx-viral-script-generator",
            displayName: "org-28ib33ph",
            handle: "org-28ib33ph",
            publicSlug: "lingyi-wx-viral-script-generator",
          },
          slug: "lingyi-wx-viral-script-generator",
          name: "灵易微信爆款脚本生成器",
          version: "0.3.0",
        },
      ],
    });

    expect(skill.reference).toBe(
      "skillhub:@org-28ib33ph/lingyi-wx-viral-script-generator@0.3.0",
    );
  });

  it("does not expose an installable item when the namespace is missing", () => {
    const skills = normalizeSkillItems({
      items: [
        {
          slug: "missing-namespace",
          name: "Missing Namespace",
          version: "1.0.0",
        },
      ],
    });

    expect(skills).toEqual([]);
  });

  it("keeps a canonical reference supplied by SkillHub", () => {
    const [skill] = normalizeSkillItems({
      items: [
        {
          slug: "find-skills",
          reference: "skillhub:@root/find-skills@1.0.0",
          version: "1.0.0",
        },
      ],
    });

    expect(skill.reference).toBe("skillhub:@root/find-skills@1.0.0");
  });
});

function codexState(overrides: Partial<EngineState> = {}): EngineState {
  return {
    engine: "codex",
    status: "starting",
    windowReady: false,
    ...overrides,
  };
}

describe("Codex 真实窗口启动状态", () => {
  it("does not report started while only the background process exists", () => {
    const state = codexState({ processPid: 321 });

    expect(runtimeActionLabel("codex", state, false)).toBe("等待 Codex 窗口");
    expect(runtimeStatusHint("codex", state, true)).toContain("等待可见窗口");
  });

  it("reports started only after a responsive window is detected", () => {
    const state = codexState({
      status: "running",
      processPid: 321,
      windowReady: true,
      windowHandle: 456,
    });

    expect(runtimeActionLabel("codex", state, false)).toBe("已启动");
    expect(runtimeStatusHint("codex", state, true)).toContain("窗口已经出现");
  });

  it("offers to restore a background process without a visible window", () => {
    const state = codexState({
      status: "failed",
      processPid: 321,
      message: "Codex进程已经运行，但没有检测到可见窗口",
    });

    expect(runtimeActionLabel("codex", state, false)).toBe("重新唤起");
    expect(runtimeStatusHint("codex", state, true)).toContain("没有检测到可见窗口");
  });
});

describe("OpenClaw 网关操作状态", () => {
  it("offers to reopen the control UI only after the gateway is running", () => {
    const state: EngineState = {
      engine: "openclaw",
      status: "running",
      processPid: 654,
      loopbackPort: 18789,
      windowReady: true,
    };

    expect(runtimeActionLabel("openclaw", state, false)).toBe(
      "打开 OpenClaw",
    );
    expect(runtimeStatusHint("openclaw", state, true)).toContain("停止网关");
  });

  it("keeps the normal launch label while the gateway is stopped", () => {
    const state: EngineState = {
      engine: "openclaw",
      status: "stopped",
      windowReady: false,
    };

    expect(runtimeActionLabel("openclaw", state, false)).toBe("启动");
  });
});

describe("Codex 与 OpenClaw 安全停止入口", () => {
  it("shows stop support only for Codex and OpenClaw", () => {
    expect(engineSupportsStop("codex")).toBe(true);
    expect(engineSupportsStop("openclaw")).toBe(true);
    expect(engineSupportsStop("claude")).toBe(false);
  });

  it("enables stop while a managed Codex process is starting or running", () => {
    expect(
      engineCanStop("codex", codexState({ status: "starting", processPid: 321 })),
    ).toBe(true);
    expect(
      engineCanStop("codex", codexState({ status: "running", processPid: 321 })),
    ).toBe(true);
  });

  it("disables repeated stop while Codex data is being saved", () => {
    const state = codexState({
      status: "saving",
      message: "Codex已关闭，正在保存随盘数据",
    });

    expect(engineCanStop("codex", state)).toBe(false);
    expect(runtimeActionLabel("codex", state, false)).toBe("保存数据中");
    expect(runtimeStatusHint("codex", state, true)).toContain("正在保存随盘数据");
  });

  it("enables gateway stop only while OpenClaw is active", () => {
    expect(
      engineCanStop("openclaw", {
        engine: "openclaw",
        status: "running",
        processPid: 654,
        windowReady: true,
      }),
    ).toBe(true);
    expect(
      engineCanStop("openclaw", {
        engine: "openclaw",
        status: "stopped",
        windowReady: false,
      }),
    ).toBe(false);
  });
});

describe("启动前自动保存供应商配置", () => {
  it("saves a dirty configuration before starting an engine", () => {
    expect(shouldSaveBeforeLaunch("codex", undefined, false)).toBe(true);
    expect(shouldSaveBeforeLaunch("openclaw", undefined, false)).toBe(true);
  });

  it("does not rewrite configuration when reopening a running OpenClaw dashboard", () => {
    expect(
      shouldSaveBeforeLaunch(
        "openclaw",
        {
          engine: "openclaw",
          status: "running",
          processPid: 654,
          windowReady: true,
        },
        false,
      ),
    ).toBe(false);
  });
});
