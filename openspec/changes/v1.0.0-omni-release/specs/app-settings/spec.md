# Spec Delta

## Purpose

为用户提供统一的全局设置中心，使其能够备份与迁移本地配置、在换机或重装后恢复平台凭据与文章映射，并能彻底清除本机数据以重置应用状态。

## ADDED Requirements

### Requirement: Settings entry opens the settings center
应用 SHALL 在顶部导航栏提供「设置」入口，点击后打开设置中心视图，视图内按「配置备份与恢复」「数据清理」「关于与支持」分区呈现功能，任一时刻只展示一个分区内容。

#### Scenario: Open settings from header
- **WHEN** 用户点击顶部导航栏的「设置」按钮
- **THEN** 设置中心打开并默认展示「配置备份与恢复」分区
- **AND** 顶部导航栏的「设置」按钮显示为选中状态

#### Scenario: Switch between settings sections
- **WHEN** 用户在设置中心内切换到另一个分区
- **THEN** 只有目标分区的内容可见，前一分区内容不再渲染

### Requirement: Export configuration to a single JSON file
设置中心 SHALL 提供「导出配置」按钮，把平台配置（含博客园凭证与各平台 Cookie）、文章正文、文章索引与同步映射打包为单个 JSON 文件并写入用户指定的本地路径，导出结果包含 `version`、`exportedAt`、`platforms`、`articles`、`syncMap` 五个顶层字段。

#### Scenario: Export produces a complete file
- **WHEN** 用户点击「导出配置」并选择一个可写路径
- **THEN** 该路径下生成一个 JSON 文件，其 `platforms` 含全部已配置平台的配置项，`articles` 含全部文章，`syncMap` 含全部文章的同步映射
- **AND** 界面提示导出成功并显示文件路径

#### Scenario: Export with empty data still succeeds
- **WHEN** 应用内没有任何平台配置与文章时用户点击「导出配置」
- **THEN** 仍生成一个 JSON 文件，各集合为空数组或空对象
- **AND** 不因数据为空而报错

#### Scenario: Export failure surfaces the reason
- **WHEN** 目标路径不可写或写入过程中发生 I/O 错误
- **THEN** 导出以失败结束，界面显示失败原因
- **AND** 不留下只写了一半的 JSON 文件

### Requirement: Export requires acknowledgement of plaintext secrets
由于导出文件为可移植的明文 JSON，导出操作 SHALL 在写入前要求用户确认「该文件包含明文 Cookie 与令牌，请妥善保管」，用户取消时不得写入任何文件。

#### Scenario: User cancels the acknowledgement
- **WHEN** 用户在导出前的确认弹窗中选择取消
- **THEN** 不产生任何文件写入
- **AND** 界面不显示导出成功提示

#### Scenario: User confirms the acknowledgement
- **WHEN** 用户确认该文件包含明文凭据并继续
- **THEN** 导出流程继续执行

### Requirement: Import configuration with validation and rollback
设置中心 SHALL 提供「导入配置」按钮，读取用户选择的 JSON 文件，校验 `version` 与集合结构后覆盖当前平台配置、文章与同步映射；校验失败或写入中途失败时 SHALL 保持导入前的数据不变，并报告失败原因。

#### Scenario: Import replaces existing data
- **WHEN** 用户选择一个结构合法的导出文件并确认导入
- **THEN** 平台配置、文章与同步映射被文件内容替换
- **AND** 界面刷新后展示导入后的平台配置状态与文章列表

#### Scenario: Import rejects malformed file
- **WHEN** 用户选择的文件不是合法 JSON，或缺少 `version`、`platforms`、`articles`、`syncMap` 中任一顶层字段
- **THEN** 导入被拒绝，界面显示具体缺失项或解析错误
- **AND** 当前数据保持不变

#### Scenario: Import failure rolls back
- **WHEN** 校验通过但写入存储过程中发生错误
- **THEN** 已写入的部分被还原为导入前的数据
- **AND** 界面显示失败原因

### Requirement: Reset all cookies and article data
设置中心 SHALL 提供「一键清空所有 Cookie 与文章数据」的红线操作，删除全部加密 Cookie 条目、全部文章、文章索引与同步映射，执行前 SHALL 要求用户二次确认。

#### Scenario: Reset requires explicit confirmation
- **WHEN** 用户点击「一键清空所有 Cookie 与文章数据」
- **THEN** 弹出确认弹窗，明确说明该操作不可恢复
- **AND** 在用户确认前不删除任何数据

#### Scenario: Reset clears credentials and articles
- **WHEN** 用户确认执行清空
- **THEN** 全部 Cookie 条目、全部文章、文章索引与同步映射被删除
- **AND** 界面回到无文章、无平台配置的初始状态

#### Scenario: Reset keeps non-credential preferences
- **WHEN** 用户确认执行清空
- **THEN** 界面偏好类设置（如已选平台、推广区关闭状态）不被清除

#### Scenario: Publish after reset reports not configured
- **WHEN** 清空完成后用户尝试同步某个平台
- **THEN** 该平台被判定为未配置并给出重新配置的提示
- **AND** 不发起任何使用旧凭据的请求