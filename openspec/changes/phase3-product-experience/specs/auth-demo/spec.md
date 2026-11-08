# Spec Delta

## MODIFIED Requirements

### Requirement: WebviewWindow loads platform login page
The app SHALL provide a function that creates a new `WebviewWindow` loading a specified platform's login URL. The window SHALL display the platform's native login page (including QR code scan) without any OpenPub UI overlay.

#### Scenario: User opens login window for a platform
- **WHEN** the user clicks "Login" for a platform (e.g., Juejin)
- **THEN** a new Tauri `WebviewWindow` opens loading `https://juejin.cn/login`
- **AND** the window displays the platform's native login interface

#### Scenario: Login URL comes from the platform registry
- **WHEN** the user clicks "One-click login" for any cookie-based platform
- **THEN** the login window loads that platform's `loginUrl` from the registry
- **AND** no login URL is hardcoded per platform in the frontend

#### Scenario: Duplicate login window is prevented
- **WHEN** the user clicks "One-click login" while a login window for the same platform is already open
- **THEN** no second window is created
- **AND** the existing window is focused

### Requirement: Cookie extraction after login detection
After the user completes login in the `WebviewWindow`, the Rust backend SHALL detect the successful login (by monitoring navigation to a post-login URL or cookie presence) and extract the platform's authentication cookie(s) via the Tauri Webview cookie API.

#### Scenario: Cookie extracted after successful login
- **WHEN** the user completes login and the Webview navigates to the platform's homepage
- **THEN** the Rust backend extracts the authentication cookie (e.g., `sessionid` for Juejin)
- **AND** the login Webview window is closed automatically

#### Scenario: Cookie is assembled from all cookies of the platform domain
- **WHEN** the backend extracts credentials after a successful login
- **THEN** every cookie visible to the login window for that platform domain is serialized into a single `name=value; name=value` string
- **AND** the assembled string is stored as the platform credential without further parsing on the frontend

#### Scenario: Login cancelled or failed
- **WHEN** the user closes the login window without completing login
- **THEN** no cookie is extracted and the frontend receives a "login cancelled" status
- **AND** no partial or empty credential is written to storage

#### Scenario: Extraction happens only once per login session
- **WHEN** the login window navigates between pages of the same platform after credentials have already been extracted
- **THEN** no additional credential write occurs for that window

### Requirement: frontend login status callback
After cookie extraction and encrypted storage complete, the Rust backend SHALL emit an event to the frontend indicating the login result (success with platform name, or failure with error message). The frontend SHALL update the platform's auth status indicator accordingly.

#### Scenario: Frontend shows logged-in status
- **WHEN** the Rust backend successfully stores a cookie for a platform
- **THEN** the frontend receives a `login-success` event with the platform name
- **AND** the platform's status indicator changes to "Logged In"

#### Scenario: Frontend shows login failure
- **WHEN** the Rust backend fails to extract or store a cookie
- **THEN** the frontend receives a `login-failed` event with an error message
- **AND** the platform's status indicator shows the error

#### Scenario: Event is emitted after storage completes
- **WHEN** the credential has been encrypted and persisted
- **THEN** the `login-success` event is emitted only after the write succeeds
- **AND** a write failure results in `login-failed` rather than a success event

## ADDED Requirements

### Requirement: Captured cookie must satisfy platform key requirements
提取到的 Cookie SHALL 在落盘前按平台要求校验关键键是否存在；缺失关键键时 SHALL 判定登录未完成、不发成功事件，并提示用户重新登录。

#### Scenario: Required key present
- **WHEN** 抓取到的 Cookie 包含该平台鉴权所需的关键键
- **THEN** 凭据被加密保存并发出 `login-success` 事件

#### Scenario: Required key missing
- **WHEN** 抓取到的 Cookie 缺少该平台鉴权所需的关键键（例如仅剩匿名访问类 Cookie）
- **THEN** 不保存凭据，发出 `login-failed` 事件并说明缺少的键名
- **AND** 平台状态保持为未登录

#### Scenario: Empty cookie set
- **WHEN** 登录窗口关闭时未取得任何 Cookie
- **THEN** 视为登录取消，不写入任何凭据

### Requirement: One-click login is the primary credential path
对于 Cookie 类平台，配置弹窗 SHALL 以「一键登录」为主入口，手动粘贴 Cookie 保留为折叠的备用入口；通过一键登录成功后，平台配置状态 SHALL 自动更新为「已登录」，无需用户再次点击保存。

#### Scenario: Successful login updates status without manual save
- **WHEN** 用户通过一键登录完成扫码并收到成功事件
- **THEN** 配置弹窗关闭且该平台状态显示为「已登录」
- **AND** 用户不需要再点击任何保存按钮

#### Scenario: Manual paste remains available as fallback
- **WHEN** 用户展开备用入口并粘贴 Cookie 后保存
- **THEN** 配置按手动粘贴的内容保存，行为与一键登录保存的凭据一致

#### Scenario: Configured state survives restart
- **WHEN** 通过一键登录保存凭据后重启应用
- **THEN** 该平台仍显示为已配置状态
- **AND** 不需要重新登录即可发起同步