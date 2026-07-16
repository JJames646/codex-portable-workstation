const state = {
  page: "home",
  providerMode: "recommended",
  networkMode: "bundle",
  codexRunning: false,
  envConfigured: false,
  networkConnected: false,
  licenseActive: false,
  quickKeyConfigured: false,
  updateDot: true,
  theme: "dark",
};

const titles = {
  home: "首页",
  providers: "供应商配置",
  projects: "项目管理",
  sessions: "会话与绘画",
  tools: "工具与插件",
  enhance: "页面增强",
  zed: "Zed 远程项目",
  data: "数据与备份",
  network: "网络环境配置",
  updates: "更新中心",
  maintenance: "安装维护",
  settings: "设置",
  sub2Detail: "我的中转站",
};

const content = document.querySelector("#content");
const pageTitle = document.querySelector("#pageTitle");
const toast = document.querySelector("#toast");
const launchButton = document.querySelector("#launchCodex");
const stopButton = document.querySelector("#stopCodex");
const safeEjectTopButton = document.querySelector("#safeEjectTop");

function showToast(message) {
  toast.textContent = message;
  toast.classList.add("show");
  clearTimeout(showToast.timer);
  showToast.timer = setTimeout(() => toast.classList.remove("show"), 2300);
}

function setPage(page) {
  state.page = page;
  pageTitle.textContent = titles[page] || "Codex 便携工作站";
  document.querySelectorAll(".nav-item").forEach((item) => {
    item.classList.toggle("active", item.dataset.page === page);
  });
  render();
}

function syncRuntimeButtons() {
  launchButton.disabled = state.codexRunning;
  stopButton.disabled = !state.codexRunning;
  safeEjectTopButton.disabled = state.codexRunning;
}

function panel(title, body, action = "") {
  return `
    <section class="panel">
      <div class="panel-header">
        <h2 class="panel-title">${title}</h2>
        ${action}
      </div>
      <div class="panel-body">${body}</div>
    </section>
  `;
}

function statusRow(name, detail, stateText, dot = "green", action = "") {
  const clickable = Boolean(action);
  const badgeClass = dot === "green" ? "green" : dot === "yellow" ? "yellow" : dot === "red" ? "red" : "";
  return `
    <div class="status-row ${clickable ? "clickable-row" : ""}" ${clickable ? `data-action="${action}"` : ""}>
      <span class="status-dot ${dot}"></span>
      <div>
        <strong>${name}</strong>
        <div class="muted small">${detail}</div>
      </div>
      <span class="badge ${badgeClass}">${stateText}${clickable ? " ›" : ""}</span>
    </div>
  `;
}

function environmentRows(compact = false) {
  const rows = [
    ["Windows 版本", "Windows 10/11 64-bit", "已安装", "green"],
    ["WebView2 Runtime", "内嵌网站仪表盘和详情页", "已安装", "green"],
    ["Codex 主程序", "portable/App/Codex.exe", "已安装", "green"],
    ["U盘数据目录", "portable/data/profile", "已挂载", "green"],
    [
      "网络环境配置",
      "便携工作站代理服务连接状态",
      state.envConfigured ? "已配置" : "未配置",
      state.envConfigured ? "green" : "red",
      state.envConfigured ? "" : "goNetworkConfig",
    ],
  ];
  const visibleRows = compact ? rows.slice(0, 5) : rows;
  return visibleRows.map(([name, detail, stateText, dot, action]) => statusRow(name, detail, stateText, dot, action)).join("");
}

function websiteDashboardFrame() {
  const chart = [52, 61, 74, 66, 88, 72, 96, 81, 68, 86, 74, 92]
    .map((height) => `<div class="bar" style="height:${height}%"></div>`)
    .join("");

  return `
    <div class="web-dashboard">
      <div class="web-dashboard-top">
        <div>
          <strong>Codex++ 概览</strong>
          <div class="muted small">本地数据：Codex++ Overview + Workstation Health</div>
        </div>
        <span class="badge green">U盘状态正常</span>
      </div>
      <div class="grid-3">
        <div class="metric-card"><span class="muted small">Codex 版本</span><strong>1.18.0</strong><div class="mini">便携运行时 · Win11 x64</div></div>
        <div class="metric-card"><span class="muted small">项目与会话</span><strong>12 / 64</strong><div class="mini">全部从当前移动盘读取</div></div>
        <div class="metric-card"><span class="muted small">供应商</span><strong>${state.quickKeyConfigured ? "已配置" : "待配置"}</strong><div class="mini">${state.quickKeyConfigured ? "官方中转站连接检测通过" : "可粘贴中转站 API Key"}</div></div>
      </div>
      <div class="usage-strip">
        <div><span>插件</span><strong>8 已启用</strong></div>
        <div><span>Skills</span><strong>26 可用</strong></div>
        <div><span>会话索引</span><strong>正常</strong></div>
        <div><span>剩余空间</span><strong>41.8GB</strong></div>
      </div>
      <div class="chart">${chart}</div>
    </div>
  `;
}

function renderHome() {
  content.innerHTML = `
    <div class="page">
      <section class="grid-3">
        <div class="metric-card accent">
          <span class="muted small">当前工作区</span>
          <strong>U:\\CodexPortable</strong>
          <div class="mini">项目、会话、插件、Scale 记忆从 U 盘直接读取</div>
        </div>
        <div class="metric-card">
          <span class="muted small">Codex 状态</span>
          <strong>${state.codexRunning ? "运行中" : "未启动"}</strong>
          <div class="mini">必须由便携工作站统一启动和停止</div>
        </div>
        <div class="metric-card">
          <span class="muted small">网络环境</span>
          <strong>${state.networkConnected ? "已接入" : "待配置"}</strong>
          <div class="mini">便携工作站代理服务待连接</div>
        </div>
      </section>

      <section class="grid-2">
        ${panel(
          "Codex++ 概览仪表盘",
          `
            <div class="toolbar" style="justify-content: space-between; margin-bottom: 12px;">
              <div>
                <h3>Codex++ 能力与工作站状态</h3>
                <p class="muted small">无需登录账号，直接读取 U 盘中的 Codex、项目、会话、供应商、插件和环境状态。</p>
              </div>
              <button class="primary-button" data-action="openSub2">查看详情</button>
            </div>
            ${websiteDashboardFrame()}
          `
        )}

        ${panel(
          "运行状态与环境校验",
          `
            <p class="muted small">首次插入新电脑时自动核对环境。绿色代表已安装/已挂载，红色代表需要点击开始环境配置。</p>
            <div class="status-grid">
              ${environmentRows(true)}
              ${statusRow("更新中心", "检测到 Codex 与插件市场更新", "可更新", "yellow")}
            </div>
            <button class="primary-button" data-action="startEnvConfig" style="margin-top:14px;">开始环境配置</button>
          `
        )}
      </section>
    </div>
  `;
}

function renderSub2Detail() {
  content.innerHTML = `
    <div class="page">
      <div class="toolbar" style="justify-content: space-between;">
        <button class="soft-button" data-action="backHome">← 返回启动器</button>
        <span class="badge blue">我的中转站</span>
      </div>
      <section class="empty-webview">
        <div class="empty-webview-inner">
          <div class="big-icon">□</div>
          <h2>我的中转站</h2>
          <p class="muted">
            后续这里直接跳转并嵌入我的中转站完整网页。启动器只负责承载，不重新设计网站内部内容。
          </p>
        </div>
      </section>
    </div>
  `;
}

function renderProviders() {
  const modes = [
    ["recommended", "官方中转站", "默认推荐，原版官方 Codex 最新模型满血接入", "官", "推荐"],
    ["third", "第三方 API 项目", "用户自己添加 Base URL、API Key、协议和模型", "API", "可选"],
    ["deepseek", "DeepSeek 纯 API", "本地直连 DeepSeek API，不走中转", "D", "直连"],
    ["local", "本地模型", "Ollama / LM Studio / LocalAI", "LM", "本机"],
  ];

  const rows = modes
    .map(
      ([id, name, desc, avatar, badge]) => `
        <div class="provider-row ${state.providerMode === id ? "selected" : ""}" data-provider="${id}">
          <div class="drag">⋮⋮</div>
          <div class="avatar ${id === "recommended" ? "blue" : ""}">${avatar}</div>
          <div>
            <strong>${name}</strong>
            <div class="muted small">${desc}</div>
          </div>
          <div class="toolbar">
            <span class="badge ${id === "recommended" ? "green" : ""}">${badge}</span>
            <button class="soft-button" data-action="selectProvider" data-provider="${id}">选择</button>
          </div>
        </div>
      `
    )
    .join("");

  const details = {
    recommended: `
      <div class="quick-key-card">
        <div>
          <strong>复制密钥，粘贴后自动配置</strong>
          <div class="muted small">从中转站网站复制 API Key。Base URL、Responses 协议和推荐模型已预设，无需逐项填写。</div>
        </div>
        <div class="quick-key-row">
          <input id="recommendedKeyInput" type="password" autocomplete="off" placeholder="在这里粘贴 API Key" />
          <button class="primary-button" data-action="importRecommendedKey">${state.quickKeyConfigured ? "重新检测" : "自动填写并检测"}</button>
        </div>
        <div class="muted small">${state.quickKeyConfigured ? "密钥已保存为安全引用，连接检测通过。" : "原型不会保存真实密钥，请勿在演示页面输入生产 Key。"}</div>
      </div>
      <div class="field-grid">
        <label class="field"><span>官方项目</span><select><option>Codex 便携工作站官方中转站</option><option>个人套餐项目</option></select></label>
        <label class="field"><span>协议</span><select><option>Responses API</option><option>Chat Completions</option></select></label>
        <label class="field"><span>默认模型</span><input value="auto-router / recommended-gateway" /></label>
        <label class="field"><span>套餐权益</span><input value="官方中转站用户免费使用便携工作站代理服务" /></label>
      </div>
    `,
    third: `
      <div class="field-grid">
        <label class="field"><span>Base URL</span><input placeholder="https://api.example.com/v1" /></label>
        <label class="field"><span>API Key</span><input placeholder="sk-..." /></label>
        <label class="field"><span>协议</span><select><option>OpenAI Compatible</option><option>Responses API</option><option>Anthropic Compatible</option></select></label>
        <label class="field"><span>模型列表</span><input placeholder="gpt-4.1, claude-sonnet, deepseek-chat" /></label>
      </div>
    `,
    deepseek: `
      <div class="field-grid">
        <label class="field"><span>Base URL</span><input value="https://api.deepseek.com" /></label>
        <label class="field"><span>API Key</span><input placeholder="DeepSeek API Key" /></label>
        <label class="field"><span>默认模型</span><input value="deepseek-chat" /></label>
        <label class="field"><span>转发模式</span><input value="Chat Completions 转 Responses" /></label>
      </div>
    `,
    local: `
      <div class="field-grid">
        <label class="field"><span>本地服务</span><select><option>Ollama</option><option>LM Studio</option><option>LocalAI</option></select></label>
        <label class="field"><span>地址</span><input value="http://127.0.0.1:11434" /></label>
        <label class="field"><span>模型</span><input value="qwen3-coder:latest" /></label>
        <label class="field"><span>状态</span><input value="等待检测" /></label>
      </div>
    `,
  };

  content.innerHTML = `
    <div class="page">
      ${panel(
        "供应商列表",
        `
          <div class="status-grid">${rows}</div>
        `,
        `<div class="toolbar"><span class="badge green">已启用切换</span><button class="soft-button">+ 添加供应商</button></div>`
      )}
      ${panel(
        "当前供应商详情",
        `
          <div class="toolbar" style="justify-content: space-between; margin-bottom: 14px;">
            <p class="muted small" style="margin:0;">保留 Codex++ 供应商能力；本产品中转站采用预设配置和便捷粘贴，第三方 API 保留完整高级表单。</p>
            <label class="check-line" style="margin:0;">
              <input type="checkbox" checked />
              <span>启动时自动写入 Codex 配置</span>
            </label>
          </div>
          ${details[state.providerMode]}
        `
      )}
    </div>
  `;
}

function renderProjects() {
  const rows = [
    ["Codex便携版项目开发", "U:\\projects\\codex-portable", "38", "刚刚", "已打开"],
    ["推荐中转站前端", "U:\\projects\\gateway-web", "12", "昨天", "可继续"],
    ["插件市场实验", "U:\\projects\\plugins", "7", "3 天前", "已索引"],
  ];

  content.innerHTML = `
    <div class="page">
      <section class="grid-2">
        ${panel(
          "U盘项目",
          `
            <div class="table-head projects"><span>项目名称</span><span>路径</span><span>会话</span><span>最近活动</span><span>状态</span></div>
            <div class="status-grid">
              ${rows
                .map(
                  ([name, path, sessions, time, status]) => `
                    <div class="project-row">
                      <div><strong>${name}</strong><div class="muted small">${path}</div></div>
                      <span class="muted small">自动挂载</span>
                      <span>${sessions}</span>
                      <span class="muted small">${time}</span>
                      <span class="badge green">${status}</span>
                    </div>
                  `
                )
                .join("")}
            </div>
          `,
          `<div class="toolbar"><button class="soft-button">导入项目</button><button class="primary-button">新建项目</button></div>`
        )}
        ${panel(
          "当前项目详情",
          `
            <h3>Codex便携版项目开发</h3>
            <p class="muted small">换电脑后会读取 U 盘里的项目路径映射、历史会话、插件配置和 Scale 记忆索引。</p>
            <div class="status-grid">
              ${statusRow("项目文件", "README.md / prototype / portable-config", "正常")}
              ${statusRow("历史会话", "38 条对话记录已关联", "已同步")}
              ${statusRow("绘画记录", "12 个图像任务索引", "已同步")}
              ${statusRow("记忆数据", "尚未确认“Scale 记忆”的真实数据来源", "待确认", "yellow")}
            </div>
          `
        )}
      </section>
    </div>
  `;
}

function renderSessions() {
  content.innerHTML = `
    <div class="page">
      <section class="grid-2">
        ${panel(
          "会话历史",
          `
            <div class="timeline">
              <div class="timeline-item"><div><strong>启动器 UI 方案讨论</strong><div class="muted small">刚刚 · 关联项目 Codex便携版项目开发</div></div></div>
              <div class="timeline-item"><div><strong>Windows U盘便携启动流程</strong><div class="muted small">今天 · 已保存到 U 盘 profile</div></div></div>
              <div class="timeline-item"><div><strong>供应商配置与推荐路由</strong><div class="muted small">昨天 · 供应商草案已标记</div></div></div>
            </div>
          `,
          `<div class="segmented"><button class="active">全部</button><button>收藏</button><button>最近</button></div>`
        )}
        ${panel(
          "绘画管理",
          `
            <div class="grid-2-even">
              <div class="metric-card"><span class="muted small">生成记录</span><strong>12</strong><div class="mini">自动跟随 U 盘</div></div>
              <div class="metric-card"><span class="muted small">素材缓存</span><strong>1.8GB</strong><div class="mini">可手动清理缓存</div></div>
            </div>
            <div class="status-grid" style="margin-top: 12px;">
              ${statusRow("最近设计稿", "Codex 便携工作站首页仪表盘方案", "可查看")}
              ${statusRow("UI 参考图", "IMG_4005.jpg", "已引用")}
              ${statusRow("导出目录", "U:\\data\\images", "正常")}
            </div>
          `
        )}
      </section>
    </div>
  `;
}

function renderTools() {
  const rows = [
    ["文档处理", "DOCX / Markdown / PDF 解析", "已启用", "1.2.0", "管理"],
    ["图片生成", "UI 方案、图像编辑", "已启用", "2.1.4", "管理"],
    ["GitHub", "仓库、Issue、PR", "未登录", "0.9.8", "连接"],
    ["Zed Remote", "远程项目桥接", "可更新", "1.0.2", "更新"],
    ["页面增强", "Codex 页面注入与增强", "已启用", "0.7.6", "设置"],
  ];

  content.innerHTML = `
    <div class="page">
      ${panel(
        "工具链与插件",
        `
          <div class="toolbar" style="justify-content: space-between; margin-bottom: 12px;">
            <div class="segmented"><button class="active">Skill</button><button>插件</button><button>页面增强</button><button>远程</button></div>
            <button class="soft-button">打开插件市场</button>
          </div>
          <div class="table-head tools"><span></span><span>名称</span><span>状态</span><span>版本</span><span>操作</span></div>
          <div class="status-grid">
            ${rows
              .map(
                ([name, desc, status, version, action]) => `
                  <div class="tool-row">
                    <div class="avatar">${name.slice(0, 1)}</div>
                    <div><strong>${name}</strong><div class="muted small">${desc}</div></div>
                    <span class="badge ${status === "已启用" ? "green" : status === "可更新" ? "yellow" : ""}">${status}</span>
                    <span class="muted small">${version}</span>
                    <button class="soft-button">${action}</button>
                  </div>
                `
              )
              .join("")}
          </div>
        `
      )}
    </div>
  `;
}

function renderEnhance() {
  content.innerHTML = `
    <div class="page">
      <section class="grid-2-even">
        ${panel(
          "页面增强",
          `
            <div class="status-grid">
              ${toggleRow("自动保存页面状态", "换电脑后恢复侧栏、主题、面板布局", true)}
              ${toggleRow("注入快捷工具条", "在 Codex 页面右下角显示快速操作", true)}
              ${toggleRow("会话标签增强", "自动按项目、日期、供应商归档", true)}
              ${toggleRow("实验功能", "预览功能默认关闭", false)}
            </div>
          `
        )}
        ${panel(
          "增强预览",
          `
            <div class="empty-webview" style="min-height: 390px;">
              <div class="empty-webview-inner">
                <div class="big-icon">⌗</div>
                <h2>Codex 页面增强区</h2>
                <p class="muted">这里后续对应便携工作站对原始 Codex 桌面端的扩展能力。</p>
              </div>
            </div>
          `
        )}
      </section>
    </div>
  `;
}

function renderZed() {
  content.innerHTML = `
    <div class="page">
      <section class="grid-2">
        ${panel(
          "远程项目",
          `
            <div class="status-grid">
              ${statusRow("Windows 开发主机", "开发环境 · SSH 可连接", "在线")}
              ${statusRow("WSL Ubuntu", "wsl.localhost · 开发环境", "可用")}
              ${statusRow("Mac 本机", "localhost · 当前设计原型", "当前")}
            </div>
          `,
          `<button class="primary-button">添加远程</button>`
        )}
        ${panel(
          "连接配置",
          `
            <div class="field-grid">
              <label class="field"><span>主机</span><input value="windows-dev.example.local" /></label>
              <label class="field"><span>端口</span><input value="22" /></label>
              <label class="field"><span>用户</span><input value="james" /></label>
              <label class="field"><span>默认目录</span><input value="D:\\codex-portable-workspace" /></label>
            </div>
          `
        )}
      </section>
    </div>
  `;
}

function renderData() {
  const rows = [
    ["项目文件", "18.4GB", "实时", "打开目录"],
    ["会话数据库", "324MB", "实时", "查看索引"],
    ["插件与 Skill", "1.2GB", "启动同步", "管理"],
    ["临时缓存", "2.8GB", "可清理", "清理"],
  ];
  content.innerHTML = `
    <div class="page">
      <section class="grid-2">
        ${panel(
          "U盘数据状态",
          `
            <div class="table-head data"><span>数据类型</span><span>占用</span><span>策略</span><span>操作</span></div>
            <div class="status-grid">
              ${rows
                .map(
                  ([name, size, strategy, action]) => `
                    <div class="data-row">
                      <div><strong>${name}</strong><div class="muted small">portable/data/${name}</div></div>
                      <span>${size}</span>
                      <span class="badge green">${strategy}</span>
                      <button class="soft-button">${action}</button>
                    </div>
                  `
                )
                .join("")}
            </div>
          `
        )}
        ${panel(
          "弹出 U盘",
          `
            <p class="muted small">因为项目和应用都在 U 盘里，正常情况下不需要额外“恢复”。弹出时只做关闭进程、刷新数据库、写快照、释放占用。</p>
            <div class="status-grid">
              ${statusRow("关闭 Codex 进程", "防止文件被占用", "准备")}
              ${statusRow("刷新 SQLite", "会话和插件状态落盘", "准备")}
              ${statusRow("生成快速快照", "只记录配置索引，不复制整盘", "准备")}
            </div>
            <button class="primary-button" data-action="safeEject" style="margin-top:14px;">安全弹出 U盘</button>
          `
        )}
      </section>
    </div>
  `;
}

function renderNetwork() {
  content.innerHTML = `
    <div class="page">
      <section class="grid-2">
        ${panel(
          "便携工作站代理服务",
          `
            <p class="muted small">该服务由便携工作站统一启动，只服务于从这里打开的 Codex，不修改电脑系统网络设置。</p>
            <div class="network-diagram">
              <div class="diagram-node active">Codex</div>
              <div class="diagram-line"></div>
              <div class="diagram-node">便携工作站代理</div>
              <div class="diagram-line"></div>
              <div class="diagram-node ${state.networkConnected ? "active" : ""}">服务连接</div>
              <div class="diagram-line"></div>
              <div class="diagram-node">模型服务</div>
            </div>
            <div class="status-grid" style="margin-top:14px;">
              ${statusRow("生效范围", "仅对从便携工作站启动的 Codex 生效", "应用级")}
              ${statusRow("系统设置", "不修改 Windows 全局网络设置", "不影响")}
              ${statusRow("服务连接状态", state.networkConnected ? "当前服务可用 · 延迟 32ms" : "服务未连接", state.networkConnected ? "已连接" : "未连接", state.networkConnected ? "green" : "red")}
            </div>
            <div class="service-note">
              <strong>合规使用声明</strong>
              <p>
                本服务仅用于提升 Codex 便携工作站在合法场景下的服务连接稳定性，不会修改电脑系统网络设置，也不会影响电脑上的其他软件。用户应遵守所在地法律法规、平台规则及第三方服务协议，不得将本服务用于破解、攻击、非法侵入、绕过权限限制、窃取数据、传播违法信息、侵犯他人隐私或知识产权等任何违法违规用途。因用户自行使用、配置或滥用本服务产生的法律责任、账号风险、数据风险及其他后果，由用户自行承担。平台有权对异常、违规或高风险使用行为采取限制、暂停或终止服务等措施，并依法配合相关监管要求。
              </p>
            </div>
            <div class="toolbar" style="margin-top:14px;">
              <button class="primary-button" data-action="connectNetwork">${state.networkConnected ? "重新连接服务" : "启动代理服务"}</button>
              <button class="stop-button" data-action="stopNetwork" ${state.networkConnected ? "" : "disabled"}>停止代理服务</button>
            </div>
          `
        )}
        ${panel(
          "套餐与卡密",
          `
            <div class="billing-choice ${state.networkMode === "bundle" ? "selected" : ""}" data-action="setNetworkMode" data-mode="bundle">
              <div>
                <strong>本产品中转站套餐</strong>
                <div class="muted small">不要求登录工作站账号。套餐有效时附带便携工作站服务连接权益。</div>
              </div>
              <span class="badge green">免费</span>
            </div>
            <div class="billing-choice ${state.networkMode === "license" ? "selected" : ""}" data-action="setNetworkMode" data-mode="license">
              <div>
                <strong>独立代理服务</strong>
                <div class="muted small">不使用本产品中转站时，输入闲鱼卡密启用，暂定 ¥10/月，从首次激活开始计时。</div>
              </div>
              <span class="badge yellow">¥10/月</span>
            </div>
            <div class="field-grid" style="margin-top:14px;">
              <label class="field"><span>卡密</span><input id="licenseInput" placeholder="请输入闲鱼购买的卡密" /></label>
              <label class="field"><span>剩余时间</span><input value="${state.licenseActive ? "29天 23小时" : "未激活"}" readonly /></label>
            </div>
            <button class="primary-button" data-action="redeemLicense" style="margin-top:14px;">验证卡密</button>
            <p class="muted small" style="margin:12px 0 0;">无需注册账号；同一卡密可绑定多台移动工作站，具体数量由管理员后台配置。数字商品暂定不支持退款。</p>
          `
        )}
      </section>
    </div>
  `;
}

function renderUpdates() {
  content.innerHTML = `
    <div class="page">
      ${panel(
        "更新中心",
        `
          <p class="muted small">每次插入并启动后异步检查自己的 GitHub Release 签名清单。不会对社区源码仓库执行 git pull；检测到更新只显示小点，不强制升级。</p>
          <div class="status-grid">
            ${updateRow("Codex 官方桌面端", "当前 1.18.0", "发现 1.18.2", "可更新")}
            ${updateRow("便携工作站管理层", "当前 0.9.4", "发现 0.9.5", "可更新")}
            ${updateRow("Skill 与插件索引", "当前 2026.07.12", "最新", "已是最新")}
            ${updateRow("便携启动器", "当前 0.1.0", "最新", "已是最新")}
          </div>
        `,
        `<div class="toolbar"><button class="soft-button" data-action="ignoreUpdates">稍后提醒</button><button class="primary-button" data-action="applyUpdates">更新选中项</button></div>`
      )}
    </div>
  `;
}

function updateRow(name, current, latest, status) {
  return `
    <div class="status-row">
      <span class="status-dot ${status === "可更新" ? "yellow" : "green"}"></span>
      <div><strong>${name}</strong><div class="muted small">${current} · ${latest}</div></div>
      <span class="badge ${status === "可更新" ? "yellow" : "green"}">${status}</span>
    </div>
  `;
}

function renderMaintenance() {
  content.innerHTML = `
    <div class="page">
      <section class="grid-2-even">
        ${panel(
          "环境检测",
          `
            <div class="status-grid" id="envRows">
              ${environmentRows(false)}
              ${statusRow("Git / SSH", "远程项目与开发环境", "建议安装", "yellow")}
            </div>
          `,
          `<button class="primary-button" data-action="startEnvConfig">${state.envConfigured ? "重新检测" : "开始环境配置"}</button>`
        )}
        ${panel(
          "安装维护",
          `
            <div class="status-grid">
              ${toggleRow("首次配置向导", "新电脑插入 U 盘时自动检测缺失依赖", true)}
              ${toggleRow("启动前锁定检测", "防止 U 盘盘符变化导致路径失效", true)}
              ${toggleRow("静默修复", "只修复启动器内部索引，不动用户项目", true)}
              ${toggleRow("开发者模式", "显示日志、端口、配置文件位置", false)}
            </div>
          `
        )}
      </section>
    </div>
  `;
}

function renderSettings() {
  content.innerHTML = `
    <div class="page">
      <section class="grid-2">
        ${panel(
          "启动器设置",
          `
            <div class="status-grid">
              ${toggleRow("开机自动识别 U 盘模式", "插入后直接显示上次项目与会话", true)}
              ${toggleRow("启动后检查更新", "只提示，不强制升级", true)}
              ${toggleRow("退出时生成快照", "弹出 U 盘时刷新内部索引", true)}
              ${toggleRow("隐藏高级日志", "普通用户不显示内部写入细节", true)}
            </div>
          `
        )}
        ${panel(
          "恢复与安全",
          `
            <p class="muted small">不需要账号密码。二次确认后会停止全部工作站进程并永久删除移动盘中的工作站数据，恢复为首次出厂状态。</p>
            <div class="status-grid">
              ${statusRow("配置与授权", "供应商、API Key、卡密、UI 状态", "将删除", "red")}
              ${statusRow("用户项目", "代码、文档、会话、绘画记录", "将删除", "red")}
              ${statusRow("扩展与备份", "插件、Skills、缓存、所有恢复点", "将删除", "red")}
            </div>
            <button class="danger-button" data-action="openFactory" style="margin-top:14px;">恢复出厂设置</button>
          `
        )}
      </section>
    </div>
  `;
}

function toggleRow(title, desc, checked) {
  return `
    <div class="status-row">
      <span class="status-dot ${checked ? "" : "yellow"}"></span>
      <div><strong>${title}</strong><div class="muted small">${desc}</div></div>
      <label class="switch">
        <input type="checkbox" ${checked ? "checked" : ""} />
        <span></span>
      </label>
    </div>
  `;
}

function render() {
  const map = {
    home: renderHome,
    providers: renderProviders,
    projects: renderProjects,
    sessions: renderSessions,
    tools: renderTools,
    enhance: renderEnhance,
    zed: renderZed,
    data: renderData,
    network: renderNetwork,
    updates: renderUpdates,
    maintenance: renderMaintenance,
    settings: renderSettings,
    sub2Detail: renderSub2Detail,
  };
  (map[state.page] || renderHome)();
}

document.querySelectorAll(".nav-item").forEach((item) => {
  item.addEventListener("click", () => setPage(item.dataset.page));
});

document.addEventListener("click", (event) => {
  const target = event.target.closest("[data-action]");
  if (!target) return;

  const action = target.dataset.action;
  if (action === "openSub2") setPage("sub2Detail");
  if (action === "backHome") setPage("home");
  if (action === "selectProvider") {
    state.providerMode = target.dataset.provider;
    renderProviders();
    showToast("已切换供应商配置视图");
  }
  if (action === "importRecommendedKey") {
    const keyInput = document.querySelector("#recommendedKeyInput");
    if (!keyInput?.value.trim()) {
      showToast("请先粘贴从中转站网站复制的 API Key");
      return;
    }
    state.quickKeyConfigured = true;
    renderProviders();
    showToast("已自动填写预设并完成连接检测（UI 原型未保存真实 Key）");
  }
  if (action === "startEnvConfig") {
    setPage("network");
    showToast("环境配置已打开，请在网络环境配置页启动代理服务");
  }
  if (action === "goNetworkConfig") {
    setPage("network");
    showToast("该项未配置，已跳转到网络环境配置");
  }
  if (action === "setNetworkMode") {
    state.networkMode = target.dataset.mode;
    renderNetwork();
    showToast(state.networkMode === "bundle" ? "已选择官方中转站附赠模式" : "已选择独立卡密模式");
  }
  if (action === "connectNetwork") {
    state.networkConnected = true;
    state.envConfigured = true;
    renderNetwork();
    showToast("便携工作站代理服务已启动");
  }
  if (action === "stopNetwork") {
    state.networkConnected = false;
    renderNetwork();
    showToast("便携工作站代理服务已停止");
  }
  if (action === "redeemLicense") {
    state.licenseActive = true;
    state.networkMode = "license";
    renderNetwork();
    showToast("卡密验证通过：代理服务剩余 29天 23小时");
  }
  if (action === "generateKeys") showToast("已模拟生成 100 个 30天卡密，实际版本会写入服务器数据库");
  if (action === "safeEject") {
    if (state.codexRunning) {
      showToast("请先点击停止，Codex 退出后再弹出 U盘");
    } else {
      showToast("已模拟执行：刷新数据库、生成快照、释放 U盘");
    }
  }
  if (action === "ignoreUpdates") {
    state.updateDot = false;
    document.querySelector('[data-page="updates"]').classList.add("dot-hidden");
    showToast("已设置为稍后提醒，本次不再显示更新小点");
  }
  if (action === "applyUpdates") showToast("已模拟更新：实际开发时会先下载到临时目录，再替换便携组件");
  if (action === "envCheck") showToast("依赖重新检测完成：Git / SSH 建议安装，其余通过");
  if (action === "openFactory") openFactoryModal();
});

document.querySelector("#safeEjectFromSide").addEventListener("click", () => {
  if (state.codexRunning) {
    showToast("请先停止 Codex，再弹出 U盘");
    return;
  }
  showToast("已模拟安全弹出流程：先落盘，再释放占用");
});

document.querySelector("#quickCheck").addEventListener("click", () => {
  showToast("运行状态检测完成：Codex、WebView2、U盘数据均正常");
});

document.querySelector("#launchCodex").addEventListener("click", () => {
  const autoStartedNetwork = !state.networkConnected;
  state.networkConnected = true;
  state.envConfigured = true;
  state.codexRunning = true;
  syncRuntimeButtons();
  render();
  showToast(
    autoStartedNetwork
      ? "已自动启动代理服务，并启动 Codex"
      : "代理服务已连接，已启动 Codex"
  );
});

document.querySelector("#stopCodex").addEventListener("click", () => {
  state.codexRunning = false;
  syncRuntimeButtons();
  if (state.page === "home") renderHome();
  showToast("Codex 已停止，现在可以安全弹出 U盘");
});

document.querySelector("#safeEjectTop").addEventListener("click", () => {
  if (state.codexRunning) {
    showToast("请先停止 Codex，再弹出 U盘");
    return;
  }
  showToast("已模拟弹出流程：确认 Codex 停止、刷新数据、释放 U盘占用");
});

document.querySelector("#themeToggle").addEventListener("click", () => {
  document.body.classList.toggle("light-mode");
  state.theme = document.body.classList.contains("light-mode") ? "light" : "dark";
  showToast(state.theme === "dark" ? "已切换为暗色模式" : "已切换为浅色预览");
});

function openFactoryModal() {
  document.querySelector("#factoryModal").classList.add("show");
  document.querySelector("#factoryModal").setAttribute("aria-hidden", "false");
}

function closeFactoryModal() {
  document.querySelector("#factoryModal").classList.remove("show");
  document.querySelector("#factoryModal").setAttribute("aria-hidden", "true");
}

document.querySelector("#closeFactory").addEventListener("click", closeFactoryModal);
document.querySelector("#cancelFactory").addEventListener("click", closeFactoryModal);
document.querySelector("#factoryModal").addEventListener("click", (event) => {
  if (event.target.id === "factoryModal") closeFactoryModal();
});

document.querySelector("#confirmFactory").addEventListener("click", () => {
  const agree = document.querySelector("#factoryAgree").checked;
  const confirmation = document.querySelector("#factoryConfirmText").value.trim();
  if (!agree) {
    showToast("请先勾选手动确认项");
    return;
  }
  if (confirmation !== "恢复出厂设置") {
    showToast("请输入完整确认文字：恢复出厂设置");
    return;
  }
  closeFactoryModal();
  showToast("已确认永久清空：这里是 UI 原型，未真正删除任何数据");
});

render();
syncRuntimeButtons();
