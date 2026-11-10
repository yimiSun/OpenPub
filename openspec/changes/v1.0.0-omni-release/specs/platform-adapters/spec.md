# Spec Delta

## MODIFIED Requirements

### Requirement: platform registry single source of truth
前端注册表 SHALL 是平台元数据的唯一来源，且 SHALL 覆盖 v1.0.0 的全部 19 个国内平台；每条记录 SHALL 声明执行通道 `adapterChannel` 与发布能力 `publishCapability`，`available` SHALL 仅在真机验证通过后置为 `true`。

#### Scenario: 注册表覆盖 19 个平台
- **WHEN** 应用启动加载注册表
- **THEN** 注册表包含 19 条平台记录
- **AND** 每条记录的 `id` 与参考实现 bundle 的平台标识一致

#### Scenario: 每条记录声明执行通道
- **WHEN** 读取任一平台记录
- **THEN** 该记录带有 `adapterChannel`，取值为 `http` / `webview` / `metaweblog` 之一
- **AND** `credentialKind` 与 `adapterChannel` 各自独立表达「凭据怎么收」与「请求怎么发」
- **AND** 需要页面 token 预检的平台不另设通道值，仍为 `http`

#### Scenario: available 只在真机验证后为 true
- **WHEN** 某平台适配器已实现但尚未完成真机验证
- **THEN** 该平台 `available` 为 `false`
- **AND** 前端不把它计入可同步平台数量

#### Scenario: 注册表与实现清单双向一致
- **WHEN** 执行注册表审计
- **THEN** `available: true` 的平台集合与已实现适配器 id 集合完全一致
- **AND** 任一方向不一致都产生可读的审计错误

#### Scenario: 平台 id 与参考实现一致
- **WHEN** 注册表声明腾讯云开发者社区
- **THEN** 其 id 为 `tencentcloud`
- **AND** 旧 id `tencent` 不再出现在注册表、路由与能力清单中

## ADDED Requirements

### Requirement: 适配器通道声明与路由
每个平台适配器 SHALL 声明其执行通道，`dispatch` SHALL 按通道选择执行方式，而不是按平台 id 硬编码分支；通道与平台 id 的组合 SHALL 在注册表中可查。

#### Scenario: 按通道分派
- **WHEN** 收到某平台的发布请求
- **THEN** `dispatch` 先按平台 id 找到注册记录，再按其 `adapterChannel` 选择执行路径
- **AND** 新增平台只需增加一条注册记录与一个适配器，不需要改 `dispatch` 的匹配结构

#### Scenario: 通道与实现不匹配时报错
- **WHEN** 某平台声明 `webview` 通道但不存在对应页面脚本
- **THEN** 本次同步返回明确错误而不是 panic
- **AND** 错误信息指明缺失的脚本标识

#### Scenario: 未实现的平台不被路由
- **WHEN** 请求的平台 id 不在已实现清单中
- **THEN** 返回 `not_implemented` 且不发起任何网络请求
- **AND** 其余平台继续执行

### Requirement: http 直发通道平台适配器
`adapterChannel: "http"` 的适配器 SHALL 用 Cookie 加静态请求头直接调用平台接口，请求与结果 SHALL 映射到统一的 `PublishRequest` / `PublishResult`；SHALL 遵守「先草稿、后发布」与「绝不静默降级」的既有约束。

#### Scenario: 简书发布
- **WHEN** 向简书发起同步且勾选发布
- **THEN** 适配器先创建笔记再调用发布接口
- **AND** 返回真实 `post_id` 与编辑地址，不返回占位 ID

#### Scenario: 51CTO 发布
- **WHEN** 向 51CTO 发起同步
- **THEN** 适配器按平台接口提交标题、正文与标签
- **AND** 平台返回的标识写入同步映射

#### Scenario: 开源中国发布
- **WHEN** 向开源中国发起同步
- **THEN** 适配器按平台接口创建或更新内容
- **AND** 失败时归类到明确的失败类别

#### Scenario: 腾讯云开发者社区发布
- **WHEN** 向腾讯云开发者社区发起同步
- **THEN** 适配器按平台接口提交内容
- **AND** 平台返回的标识写入同步映射

#### Scenario: 阿里云开发者社区发布
- **WHEN** 向阿里云开发者社区发起同步
- **THEN** 适配器携带该平台要求的 CSRF 头提交内容
- **AND** 未取到 CSRF 时不发业务请求并归类为 `not_configured`

#### Scenario: 百家号发布
- **WHEN** 向百家号发起同步
- **THEN** 适配器按平台接口保存或提交内容
- **AND** 平台返回的标识写入同步映射

#### Scenario: 网易号发布
- **WHEN** 向网易号发起同步
- **THEN** 适配器按平台接口提交内容
- **AND** 平台返回的标识写入同步映射

#### Scenario: 哔哩哔哩发布
- **WHEN** 向哔哩哔哩发起同步
- **THEN** 适配器从 Cookie 中读取 `bili_jct` 作为 CSRF 值
- **AND** 缺失该 Cookie 键时不发业务请求并归类为 `not_configured`

#### Scenario: 草稿与发布状态如实反映
- **WHEN** 适配器在草稿阶段成功后发布阶段失败
- **THEN** `is_published` 为 `false` 且保留草稿标识
- **AND** 不把该次同步标记为已发布

### Requirement: http 通道的页面 token 预检
`adapterChannel: "http"` 且需要页面 token 的平台 SHALL 在业务请求前先取得 token，token 取得失败 SHALL 在发请求前终止；预检不是独立通道，枚举值仍为 `http`。

#### Scenario: 微博从编辑器页面取得配置
- **WHEN** 向微博发起同步
- **THEN** 适配器先请求编辑器页面并从中抽取配置值
- **AND** 抽取失败时不发起保存请求并归类为 `not_configured`

#### Scenario: 头条取得 CSRF 探针值
- **WHEN** 向头条发起同步
- **THEN** 适配器先执行探针取得 `x-ware-csrf-token`（作为 `x-secsdk-csrf-token` 发往业务请求）
- **AND** 探针失败时不发起保存请求并归类为 `not_configured`

#### Scenario: 掘金取得 CSRF 探针值
- **WHEN** 向掘金发起同步
- **THEN** 适配器先执行探针取得 `x-ware-csrf-token`（作为 `x-secsdk-csrf-token` 发往业务请求）
- **AND** 探针失败时不发起保存请求并归类为 `not_configured`

#### Scenario: 微信公众号取得页面 token
- **WHEN** 向微信公众号发起同步
- **THEN** 适配器先请求后台页面并抽取 URL 中的 `token`
- **AND** 抽取失败时不发起保存请求并归类为 `not_configured`

#### Scenario: token 只用于本次同步
- **WHEN** token 取得成功
- **THEN** 该 token 仅在本次同步内使用
- **AND** 不写入凭据存储与日志

#### Scenario: 预检失败不产生业务请求
- **WHEN** 任一预检步骤失败
- **THEN** 本次同步的请求计数只包含预检请求
- **AND** 不产生任何建稿或发布请求

### Requirement: webview 通道平台适配器
`adapterChannel: "webview"` 的适配器 SHALL 通过通道窗口在平台页面内完成建稿与发布，只搬运文章数据与结果；SHALL NOT 在 Rust 侧计算平台签名。

#### Scenario: 知乎发布
- **WHEN** 向知乎发起同步
- **THEN** 适配器在知乎编辑页内完成建稿，勾选发布时继续在页面内完成发布
- **AND** 返回真实文章标识与编辑地址

#### Scenario: 小红书长文笔记发布
- **WHEN** 向小红书长文笔记发起同步
- **THEN** 适配器在小红书创作页内完成提交
- **AND** 不计算该平台的运行时签名头

#### Scenario: 小红书图文笔记发布
- **WHEN** 向小红书图文笔记发起同步
- **THEN** 适配器在小红书创作页内完成图文提交
- **AND** 图片与正文按平台要求一并提交

#### Scenario: 抖音长文发布
- **WHEN** 向抖音长文发起同步
- **THEN** 适配器在抖音创作页内完成提交
- **AND** 平台返回的标识写入同步映射

#### Scenario: 抖音图文发布
- **WHEN** 向抖音图文发起同步
- **THEN** 适配器在抖音创作页内完成图文提交
- **AND** 平台返回的标识写入同步映射

#### Scenario: 思否发布
- **WHEN** 向思否发起同步
- **THEN** 适配器在思否页面内调用其自身网关提交
- **AND** 不移植该网关的加密参数生成逻辑

#### Scenario: 页面内未登录
- **WHEN** 通道窗口内判定未登录
- **THEN** 适配器在 `auth` 阶段终止，不发起建稿
- **AND** 本次同步归类为 `auth_error`

### Requirement: 掘金直发分支
掘金适配器 SHALL 在 `publish_status = true` 且目标未发布时调用其发布接口，把内容从草稿推进为已发布；`publishCapability` SHALL 为 `live`。

#### Scenario: 草稿推进为发布
- **WHEN** 向掘金同步且勾选发布，目标文章为草稿
- **THEN** 适配器先建稿或更新草稿，再调用发布接口
- **AND** `is_published` 为 `true` 且保留文章标识

#### Scenario: 发布阶段失败保留草稿
- **WHEN** 建稿成功但发布接口失败
- **THEN** 结果保留草稿标识且 `is_published` 为 `false`
- **AND** 同步映射记录该草稿，避免下次重复建稿

#### Scenario: 已发布文章的覆盖确认
- **WHEN** 目标文章已发布且用户未勾选覆盖
- **THEN** 适配器在发请求前拒绝并给出明确提示
- **AND** 平台侧内容保持不变