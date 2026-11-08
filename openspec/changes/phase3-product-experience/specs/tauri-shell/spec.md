# Spec Delta

## ADDED Requirements

### Requirement: Clipboard write permission is declared for the main window
主窗口的 capability SHALL 声明剪贴板写权限，使前端能够通过 Tauri 剪贴板插件把文本写入系统剪贴板；未声明该权限时前端调用 SHALL 以权限错误失败而非静默无效。

#### Scenario: Clipboard write succeeds from frontend
- **WHEN** 前端调用剪贴板写入 API 写入一段文本
- **THEN** 系统剪贴板内容被替换为该文本
- **AND** 调用正常返回

#### Scenario: Clipboard permission is not granted to login windows
- **WHEN** 登录窗口的 capability 被审查
- **THEN** 该窗口不包含剪贴板写权限
- **AND** 登录窗口无法通过前端写入系统剪贴板

### Requirement: Host environment info is readable from the frontend
应用 SHALL 提供只读命令返回应用版本与操作系统版本，供设置页与关于弹窗展示；该命令 SHALL NOT 返回任何凭据、路径或其他敏感信息。

#### Scenario: Version info returned
- **WHEN** 前端调用版本信息命令
- **THEN** 返回应用版本号与操作系统版本描述

#### Scenario: Command carries no sensitive fields
- **WHEN** 审查该命令的返回结构
- **THEN** 返回字段仅包含版本与系统描述
- **AND** 不包含 Cookie、令牌、文件路径或用户名

### Requirement: Login windows get their own capability
登录用的 `WebviewWindow` SHALL 使用独立的 capability 声明，仅授予打开平台登录页与读取该窗口 Cookie 所需的最小权限，不继承主窗口的全部权限。

#### Scenario: Login window opens with minimal permissions
- **WHEN** 应用创建登录窗口
- **THEN** 该窗口匹配到独立的 capability 条目
- **AND** 其权限集合小于或等于主窗口权限集合

#### Scenario: Login window cannot invoke publish commands
- **WHEN** 登录窗口内页面尝试调用发布类命令
- **THEN** 调用被权限系统拒绝

### Requirement: Configuration backup and restore commands exist in the host layer
宿主层 SHALL 提供配置导出、配置导入与全量数据重置三条命令，导入与重置 SHALL 在失败时保持原有数据不变，所有命令 SHALL 返回 `Result` 并把错误信息透传到前端。

#### Scenario: Export writes a file to a chosen path
- **WHEN** 前端调用导出命令并传入目标路径
- **THEN** 宿主层在该路径写入完整 JSON 文件并返回路径

#### Scenario: Import validates before overwriting
- **WHEN** 前端调用导入命令且文件结构不合法
- **THEN** 命令返回错误且不改动现有存储内容

#### Scenario: Reset removes credentials and article data
- **WHEN** 前端调用重置命令
- **THEN** 凭据存储与文章相关键被清空
- **AND** 命令返回成功