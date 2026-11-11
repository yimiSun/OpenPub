# Spec Delta

## MODIFIED Requirements

### Requirement: platform registry single source of truth
前端注册表 SHALL 是平台元数据的唯一来源，且 SHALL 覆盖 v1.0.0 的全部 18 个国内平台；每条记录 SHALL 声明执行通道 `channelType`，`available` SHALL 仅在真机验证通过后置为 `true`；注册表 SHALL NOT 声明发布能力字段，能力由适配器的实际返回值表达。

#### Scenario: 注册表覆盖 18 个平台
- **WHEN** 应用启动加载注册表
- **THEN** 注册表包含 18 条平台记录
- **AND** 每条记录的 `id` 与参考实现 bundle 的平台标识一致

#### Scenario: 每条记录声明执行通道
- **WHEN** 读取任一平台记录
- **THEN** 该记录带有 `channelType`，取值为 `direct` / `webview` 之一
- **AND** `credentialKind` 与 `channelType` 各自独立表达「凭据怎么收」与「请求怎么发」
- **AND** 需要页面 token 或 CSRF 预检的平台不另设通道值，仍为 `direct`
- **AND** 博客园的 MetaWeblog 由 `credentialKind: "metaweblog"` 表达，`channelType` 仍为 `direct`

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

#### Scenario: Registry and dispatcher agree
- **WHEN** a consistency check compares the shared registry ids against the backend dispatcher's covered ids
- **THEN** the two sets are equal
- **AND** removing a dispatcher branch makes the check fail

#### Scenario: Unregistered platform id is rejected
- **WHEN** a publish request targets an id absent from the registry
- **THEN** the result has `success: false`
- **AND** the error message contains that unknown id

#### Scenario: Available platform must have an adapter
- **WHEN** a platform is marked available in the registry
- **THEN** that platform id is present in the implemented-adapter list
- **AND** the consistency check fails if it is missing

#### Scenario: Implemented adapter must be marked available
- **WHEN** a platform id appears in the implemented-adapter list but is marked unavailable in the registry
- **THEN** the consistency check reports that platform as mislabeled

#### Scenario: Publish capability is declared per platform
- **WHEN** 需要判断某平台能否直接发布
- **THEN** 该判断来自该平台适配器在 `publish_status = true` 时的实际返回值
- **AND** 注册表不新增 `publishCapability` 一类声明字段，避免出现第二份事实

#### Scenario: Draft-only platform suppresses live-publish expectations
- **WHEN** 仅草稿平台被勾选发布
- **THEN** 适配器返回 `not_implemented`，结果文案说明内容仅保存为草稿、需到平台后台手动发布
- **AND** 该文案由适配器返回的结果携带，前端不再按平台 id 字面量硬编码能力提示

## ADDED Requirements

### Requirement: 适配器通道声明与路由
每个平台适配器 SHALL 声明其执行通道，`dispatch` SHALL 按平台 id 路由到对应适配器，而不是按通道分派；通道与平台 id 的组合 SHALL 在注册表中可查。

#### Scenario: 按平台 id 路由
- **WHEN** 收到某平台的发布请求
- **THEN** `dispatch` 按平台 id 命中对应适配器分支
- **AND** 新增平台只需增加一条注册记录与一个适配器分支，不需要改请求/结果结构

#### Scenario: 通道与实现不匹配时报错
- **WHEN** 某平台声明 `webview` 通道但不存在对应页面脚本
- **THEN** 本次同步返回明确错误而不是 panic
- **AND** 错误信息指明缺失的脚本标识

#### Scenario: 未实现的平台不被路由
- **WHEN** 请求的平台 id 不在已实现清单中
- **THEN** 返回 `not_implemented` 且不发起任何网络请求
- **AND** 其余平台继续执行

### Requirement: direct 直发通道平台适配器
`channelType: "direct"` 的适配器 SHALL 用 Cookie 加静态请求头直接调用平台接口，请求与结果 SHALL 映射到统一的 `PublishRequest` / `PublishResult`；SHALL 遵守「先草稿、后发布」与「绝不静默降级」的既有约束。

#### Scenario: 博客园发布
- **WHEN** 向博客园发起同步且勾选发布
- **THEN** 适配器通过 MetaWeblog XML-RPC 提交内容
- **AND** 返回真实文章标识与编辑地址

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

#### Scenario: 阿里云开发者社区草稿
- **WHEN** 向阿里云开发者社区发起同步
- **THEN** 适配器携带该平台要求的 CSRF 头提交内容
- **AND** 未取到 CSRF 时不发业务请求并归类为 `not_configured`
- **AND** 勾选发布时返回 `not_implemented` 并提示到平台后台手动发布

#### Scenario: 百家号草稿
- **WHEN** 向百家号发起同步
- **THEN** 适配器按平台接口保存内容并解析 JSONP 响应
- **AND** 平台返回的标识写入同步映射
- **AND** 勾选发布时返回 `not_implemented` 并提示到平台后台手动发布

#### Scenario: 网易号草稿
- **WHEN** 向网易号发起同步
- **THEN** 适配器先取得 `wemediaId` 与 `realUserId` 再提交草稿
- **AND** 预检失败时不发业务请求并归类为 `not_configured`
- **AND** 勾选发布时返回 `not_implemented`，因为正式发布依赖页面脚本产出的 `ursToken`

#### Scenario: 哔哩哔哩草稿
- **WHEN** 向哔哩哔哩发起同步
- **THEN** 适配器从 Cookie 中读取 `bili_jct` 作为 CSRF 值
- **AND** 缺失该 Cookie 键时不发业务请求并归类为 `not_configured`
- **AND** 勾选发布时返回 `not_implemented`，因为正式发布依赖编辑器页面内点击按钮

#### Scenario: CSDN 发布
- **WHEN** 向 CSDN 发起同步且勾选发布
- **THEN** 适配器以 `status = 0` 调用保存接口并本地计算网关 HMAC 签名
- **AND** `is_published` 取请求的 `publish_status` 值

#### Scenario: 掘金草稿
- **WHEN** 向掘金发起同步
- **THEN** 适配器先执行 CSRF 探针再创建或更新草稿
- **AND** 勾选发布时返回 `not_implemented` 并在文案中提示到掘金后台手动发布

#### Scenario: 微信公众号草稿
- **WHEN** 向微信公众号发起同步
- **THEN** 适配器先取得后台 URL 中的 `token` 再提交纯文字草稿
- **AND** 勾选发布或覆盖已发布文章时返回 `not_implemented`

#### Scenario: 微博发布
- **WHEN** 向微博发起同步
- **THEN** 适配器先从编辑器页面抽取配置值再提交
- **AND** 抽取失败时不发起保存请求并归类为 `not_configured`

#### Scenario: 草稿与发布状态如实反映
- **WHEN** 适配器在草稿阶段成功后发布阶段失败
- **THEN** `is_published` 为 `false` 且保留草稿标识
- **AND** 不把该次同步标记为已发布

#### Scenario: 仅草稿平台不静默降级
- **WHEN** 用户对仅草稿平台勾选发布
- **THEN** 适配器返回 `not_implemented`，不忽略 `publish_status` 一律存草稿
- **AND** 结果文案明确提示需到平台后台手动发布
- **AND** `is_published` 为 `false`

### Requirement: direct 通道的页面 token 与 CSRF 预检
`channelType: "direct"` 且需要页面 token 或 CSRF 的平台 SHALL 在业务请求前先取得该值，取得失败 SHALL 在发请求前终止；预检不是独立通道，枚举值仍为 `direct`。

#### Scenario: 微博从编辑器页面取得配置
- **WHEN** 向微博发起同步
- **THEN** 适配器先请求编辑器页面并从中抽取配置值
- **AND** 抽取失败时不发起保存请求并归类为 `not_configured`

#### Scenario: 掘金取得 CSRF 探针值
- **WHEN** 向掘金发起同步
- **THEN** 适配器先执行探针取得 `x-ware-csrf-token`（取逗号第 2 段作为 `x-secsdk-csrf-token` 发往业务请求）
- **AND** 探针失败时不发起保存请求并归类为 `not_configured`

#### Scenario: 微信公众号取得页面 token
- **WHEN** 向微信公众号发起同步
- **THEN** 适配器先请求后台页面并抽取 URL 中的 `token`
- **AND** 抽取失败时不发起保存请求并归类为 `not_configured`

#### Scenario: 网易号取得身份标识
- **WHEN** 向网易号发起同步
- **THEN** 适配器先从 `navinfo` 取得 `wemediaId` 与 `realUserId`
- **AND** 取得失败时不发起保存请求并归类为 `not_configured`

#### Scenario: 哔哩哔哩从 Cookie 取 CSRF
- **WHEN** 向哔哩哔哩发起同步
- **THEN** 适配器从 Cookie 直接读取 `bili_jct`
- **AND** 缺失时不发起保存请求并归类为 `not_configured`

#### Scenario: token 只用于本次同步
- **WHEN** token 取得成功
- **THEN** 该 token 仅在本次同步内使用
- **AND** 不写入凭据存储与日志

#### Scenario: 预检失败不产生业务请求
- **WHEN** 任一预检步骤失败
- **THEN** 本次同步的请求计数只包含预检请求
- **AND** 不产生任何建稿或发布请求

### Requirement: webview 通道平台适配器
`channelType: "webview"` 的适配器 SHALL 通过通道窗口在平台页面内完成建稿与发布，只搬运文章数据与结果；SHALL NOT 在 Rust 侧计算平台签名。

#### Scenario: 知乎发布
- **WHEN** 向知乎发起同步
- **THEN** 适配器在知乎编辑页内完成建稿，勾选发布时继续在页面内完成发布
- **AND** 返回真实文章标识与编辑地址

#### Scenario: 小红书图文笔记发布
- **WHEN** 向小红书发起同步
- **THEN** 适配器在小红书创作页内完成图文提交
- **AND** 图片与正文按平台要求一并提交
- **AND** 勾选发布时云端发布并返回真实标识

#### Scenario: 头条发布
- **WHEN** 向今日头条发起同步
- **THEN** 适配器在头条编辑页的页面上下文内发起保存与发布请求
- **AND** 不把请求搬回 Rust 侧直发

#### Scenario: 抖音长文草稿
- **WHEN** 向抖音发起同步
- **THEN** 适配器在抖音创作页内完成提交
- **AND** 平台返回的标识写入同步映射
- **AND** 勾选发布时如实提示需手动发布，`is_published` 为 `false`

#### Scenario: 简书草稿
- **WHEN** 向简书发起同步
- **THEN** 适配器在 `/writer` 页面上下文内同源发起建笔记、自动保存与发布请求
- **AND** 不把请求搬回 Rust 侧直发

#### Scenario: 思否发布
- **WHEN** 向思否发起同步
- **THEN** 适配器在思否页面内调用其自身网关提交
- **AND** 不移植该网关的加密参数生成逻辑

#### Scenario: 页面脚本不含签名算法
- **WHEN** 审查任一 `webview` 平台的页面脚本
- **THEN** 脚本中不出现签名函数名、密钥常量或加密串拼接
- **AND** 脚本只调用平台页面已有的全局函数或模块方法

#### Scenario: 页面内未登录
- **WHEN** 通道窗口内判定未登录
- **THEN** 适配器在 `auth` 阶段终止，不发起建稿
- **AND** 本次同步归类为 `auth_error`

### Requirement: 平台能力边界以返回值表达
适配器 SHALL 在 `publish_status` 与 `existing_published` 的每种组合上显式判定；仅草稿平台在勾选发布时 SHALL 返回 `not_implemented`，不得忽略参数静默存草稿。

#### Scenario: 仅草稿平台勾选发布
- **WHEN** 用户对掘金、微信公众号、阿里云、百家号、网易号、哔哩哔哩、抖音或简书勾选发布
- **THEN** 适配器返回 `not_implemented` 且 `is_published` 为 `false`
- **AND** 结果文案提示到平台后台手动发布

#### Scenario: 已发布文章的覆盖确认
- **WHEN** 目标文章已发布且用户未勾选覆盖
- **THEN** 适配器在发请求前拒绝并给出明确提示
- **AND** 平台侧内容保持不变

#### Scenario: 草稿去重更新
- **WHEN** 同一篇文章再次同步且映射中已有草稿标识
- **THEN** 适配器走更新分支而不是重复建稿
- **AND** 同步映射的标识保持不变