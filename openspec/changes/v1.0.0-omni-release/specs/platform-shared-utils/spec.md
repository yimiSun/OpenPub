# Spec Delta

## ADDED Requirements

### Requirement: 页面 token 抽取
共享层 SHALL 提供从平台页面 HTML 中抽取 token 的能力，作为 `http` 通道的 preflight 供适配器调用；抽取失败 SHALL 返回明确错误，不得返回空串继续发起业务请求。

#### Scenario: 从页面 HTML 抽取 token
- **WHEN** 某平台需要从编辑器页面 HTML 中读取 token（如微信公众号的 `token`）
- **THEN** 共享层按该平台给定的模式抽取 token 并返回非空值
- **AND** 调用方不需要自己写正则

#### Scenario: 抽取失败返回错误
- **WHEN** 页面 HTML 中不存在目标 token（页面改版或未登录）
- **THEN** 共享层返回错误并指明缺失的 token 名称
- **AND** 适配器在发业务请求前终止，归类为 `not_configured`

#### Scenario: 抽取结果不落盘
- **WHEN** token 抽取成功
- **THEN** token 只在本次同步内存中存在
- **AND** 不写入凭据存储、不写入日志

### Requirement: 响应头 CSRF 探针
共享层 SHALL 提供通过一次轻量请求（HEAD 或 GET）读取响应头 `x-ware-csrf-token` 的能力，供掘金与头条使用；探针须带请求头 `x-secsdk-csrf-request: 1`，否则 404。

#### Scenario: 探针取得 csrf token
- **WHEN** 适配器调用探针并传入探测地址与 Cookie
- **THEN** 共享层返回响应头中的 `x-ware-csrf-token` 值
- **AND** 该值供后续业务请求作为请求头使用

#### Scenario: 探针的版本头由调用方传入
- **WHEN** 适配器调用探针
- **THEN** `x-secsdk-csrf-version` 由调用方传入，头条为 `1.2.22`、掘金为 `1.2.10`
- **AND** 共享层不写死该版本号常量

#### Scenario: 探针返回值支持两种取值方式
- **WHEN** 适配器调用探针
- **THEN** 共享层支持「整串」与「逗号第 N 段」两种取值方式
- **AND** 头条使用整串、掘金使用 `split(",")[1]`

#### Scenario: 探针失败不重试
- **WHEN** 探针请求失败或响应头缺失
- **THEN** 共享层返回错误，不自动重试
- **AND** 适配器归类为 `not_configured` 并提示重新登录

#### Scenario: 探针不产生副作用
- **WHEN** 执行探针
- **THEN** 只发起读请求，不提交任何内容
- **AND** 不改变平台侧的文章状态

### Requirement: 编辑器配置 JSON 抽取
共享层 SHALL 提供从页面 HTML 中抽取内嵌配置 JSON 并读取指定字段的能力，供微博等以配置对象传递凭据的平台使用。

#### Scenario: 从配置 JSON 取字段
- **WHEN** 平台页面内嵌一段配置 JSON 且鉴权值位于其中
- **THEN** 共享层抽取该 JSON 并按字段名返回值
- **AND** 字段缺失时返回错误而不是 `null`

#### Scenario: JSON 结构异常时安全失败
- **WHEN** 抽取到的片段不是合法 JSON
- **THEN** 共享层返回错误且不 panic
- **AND** 适配器归类为 `not_configured`