# ADR-0007：两套业务服务器使用端口接口和 NotConnected Adapter

## 状态

Accepted

## 背景

客户端需要先完成，但中转站网站服务器和工作站卡密/网络控制服务器由用户后续提供。与此同时，Codex 官方服务、第三方 API、GitHub、插件市场和远程项目必须保持正常联网。

## 决策

客户端预留两个版本化端口：

- `RelayServicePort`
- `WorkstationControlPort`

当前使用 `NotConfigured`/`ServerNotConnected` Adapter。GitHub Release 更新使用独立 `GitHubReleaseUpdateSource`，不依赖两套业务服务器。

## 正面影响

- 客户端不被服务器开发阻塞。
- 后续接入服务器时不修改页面业务结构。
- 未接入状态真实可见，不产生假余额、假套餐或假卡密。
- 普通联网能力不受影响。

## 负面影响

- RC1 中相关页面不能展示真实业务数据。
- 需要维护 DTO 版本和契约测试。
- 后续服务器字段与当前合同不一致时需要 adapter 转换。

## 备选

- 在前端直接写死未来 URL：拒绝，因为域名和接口尚未提供。
- 使用假成功 Mock：拒绝，因为容易把演示状态误认为真实服务。
- 等服务器完成后再开发客户端：拒绝，因为会阻塞 U 盘启动器主体。
