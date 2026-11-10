# Spec Delta

## ADDED Requirements

### Requirement: 通道窗口生命周期
`webview` 通道 SHALL 使用固定 label `channel-{platform}` 的独立 `WebviewWindow`，与登录窗口 `login-{platform}` 分离；同一平台的通道窗口在多次同步之间 SHALL 复用，不得每次同步重建。

#### Scenario: 首次同步创建窗口
- **WHEN** 某 `webview` 通道平台首次发起同步
- **THEN** 创建 label 为 `channel-{platform}` 的窗口并加载该平台的编辑页
- **AND** 该窗口不显示任何 OpenPub 覆盖层

#### Scenario: 重复同步复用同一窗口
- **WHEN** 同一平台在通道窗口仍存在时再次发起同步
- **THEN** 复用已有窗口，不创建第二个窗口
- **AND** 已有窗口被聚焦

#### Scenario: 窗口关闭后重建
- **WHEN** 用户手动关闭通道窗口后再次发起同步
- **THEN** 重新创建该平台窗口并重新加载编辑页

### Requirement: 页面就绪判定与 payload 注入
通道 SHALL 在页面加载完成且平台编辑器可用后再注入 payload；payload SHALL 只包含文章数据（`title` / `markdown` / `html` / `tags` / `cover` / `action` / `existingId`），不得包含凭据、密钥或签名参数。

#### Scenario: 页面未就绪时不注入
- **WHEN** 窗口页面加载完成但平台编辑器尚未初始化
- **THEN** 通道在就绪判定超时前持续等待，不提前注入
- **AND** 就绪判定超时后以 `network_error` 结束本次同步

#### Scenario: payload 只含文章数据
- **WHEN** 通道向页面注入 payload
- **THEN** payload 字段集合限定为文章数据字段
- **AND** 不含 Cookie、token、签名头或本地文件路径

#### Scenario: 注入失败可区分于业务失败
- **WHEN** 页面脚本注入本身失败
- **THEN** 本次同步归类为 `network_error` 而非 `auth_error`
- **AND** 日志记录注入阶段而不是平台返回的错误码

### Requirement: 页面脚本回传结构化结果
页面脚本 SHALL 通过 IPC 回传单一结构化结果 `{ platform, phase, message, isPublished?, postId?, editUrl?, stage? }`，Rust 侧 SHALL 将其映射为 `PublishResult`；`phase` 取值限定为 `success` / `failed` / `auth`，`stage` 取值限定为 `auth` / `ready` / `inject` / `submit` 且仅在 `failed` 回传时有意义。

`draft` / `publish` 的阶段语义不再作为 `phase` 取值，改由 `stage=submit` + 保留 `postId` + `isPublished=false` 表达（建稿成功、发布失败）。

#### Scenario: 成功回传映射为发布结果
- **WHEN** 页面脚本回传 `phase: success` 且带 `postId`
- **THEN** `PublishResult` 携带该 `postId` 与 `editUrl`
- **AND** `is_published` 取自请求的 `publish_status`

#### Scenario: 阶段失败带阶段标记
- **WHEN** 建稿成功但发布失败
- **THEN** 回传结果保留建稿得到的 `postId`
- **AND** `phase` 为 `failed` 且 `stage` 为 `submit`，本次同步按部分成功处理并写入草稿映射

#### Scenario: 未登录在 auth 阶段终止
- **WHEN** 页面内判定用户未登录
- **THEN** `phase` 为 `auth` 且不发起任何建稿或发布请求
- **AND** 本次同步归类为 `auth_error`

#### Scenario: 平台无编辑入口时不落盘占位 ID
- **WHEN** 页面脚本只得到占位或空 ID
- **THEN** 该结果不写入同步映射
- **AND** 本次同步标记失败

### Requirement: 超时与失败分类
通道 SHALL 为「窗口加载」「页面就绪」「脚本执行」「结果回传」四段分别设置超时；任一超时 SHALL 结束本次同步并归类 `network_error`，不得无限等待。

#### Scenario: 结果回传超时
- **WHEN** 页面脚本已执行但结果未在超时内回传
- **THEN** 本次同步以 `network_error` 结束
- **AND** 不把该次同步标记为成功

#### Scenario: 用户中途关闭窗口
- **WHEN** 用户在脚本执行期间关闭通道窗口
- **THEN** 本次同步以失败结束并归类 `network_error`
- **AND** 不写入任何映射

### Requirement: 不得移植平台签名算法
页面脚本 SHALL 只调用平台页面已有的全局函数或模块方法；脚本内 SHALL NOT 出现任何签名算法实现、签名密钥常量或签名头的本地计算。

#### Scenario: 脚本复用平台自身能力
- **WHEN** 平台接口需要运行时签名
- **THEN** 签名由平台页面在请求时自行产生
- **AND** 本应用不计算、不缓存、不伪造该签名

#### Scenario: 脚本被审查时可直接判定合规
- **WHEN** 评审新增或修改的页面脚本
- **THEN** 脚本中不存在签名函数名、密钥常量或加密串拼接逻辑
- **AND** 脚本只做「取编辑器、填内容、点按钮、读结果」四类操作

### Requirement: 单平台失败隔离
一个 `webview` 平台失败 SHALL NOT 影响其他平台的同步；每个平台的通道窗口与结果回传 SHALL 相互独立。

#### Scenario: 一个平台超时不影响其他平台
- **WHEN** 一个 `webview` 平台通道超时失败
- **THEN** 其余平台按各自通道继续执行
- **AND** 汇总结果只把该平台标记为失败