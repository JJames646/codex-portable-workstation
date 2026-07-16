# 本地中转站预览接入

## 用途

当前用于把局域网 Sub2API 用户仪表盘接入 Codex 便携工作站，验证登录、个人用量、套餐额度和详情窗口。管理员后台仍是独立管理入口，不进入客户端。

## 构建配置

服务器根地址通过 Rust 编译时环境变量注入，不写入普通前端配置：

```powershell
$env:CODEX_WORKSTATION_RELAY_BASE_URL = "http://192.168.1.181:8080"
cargo build -p codex-plus-manager --release
```

未设置该变量时，首页会显示中转站服务器未配置。

## 用户流程

1. 未登录时，首页中转站仪表盘显示登录表单，不读取用户数据。
2. 登录成功后，工作站通过用户级 API 读取 profile、dashboard stats、trend、models 和 subscriptions summary。
3. 首页显示当前账号自己的余额、今日请求、Token、实际消耗、套餐额度和趋势。
4. “查看详情”打开 `/dashboard` 用户网页，并注入同一短期会话，避免再次登录。
5. 管理员 `/admin/*` 页面和管理员 API 不参与该流程。

## 安全边界

- 远程用户页面使用独立 `relay-preview` WebviewWindow。
- `capabilities/default.json` 只授权 `main` 窗口；远程窗口不拥有 Tauri IPC 权限。
- 顶层导航只允许停留在配置地址的相同 scheme、host 和 port。
- URL 不允许包含账号或密码。
- 公网地址必须使用 HTTPS；HTTP 仅允许 localhost 或私有局域网 IP。
- 密码只用于登录请求，不进入状态、日志或配置。
- 当前预览的 access/refresh token 只保存在客户端进程内；退出程序后需要重新登录。
- 远程窗口使用临时浏览数据，由工作站在打开时注入短期用户会话。
- 客户端不调用 Sub2API 管理员 API，也不保存管理员 Key。

## 正式接入时替换

正式用户版应把预览地址替换为用户网站入口，并由签名的服务器配置提供允许域名。首页用量数据仍通过 `activation-bridge` 和用户级 adapter 接口读取，不得调用管理员接口。
