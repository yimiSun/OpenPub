# auth-demo Specification

## Purpose
提供 Tauri Webview 内嵌登录的最小闭环示例，验证从"用户点击登录"到"Cookie 加密落盘"再到"前端获知登录状态"的完整链路，为后续所有平台适配器的鉴权流程奠定可复用的模式。

## Requirements

### Requirement: WebviewWindow loads platform login page
The app SHALL provide a function that creates a new `WebviewWindow` loading a specified platform's login URL. The window SHALL display the platform's native login page (including QR code scan) without any OpenPub UI overlay.

#### Scenario: User opens login window for a platform
- **WHEN** the user clicks "Login" for a platform (e.g., Juejin)
- **THEN** a new Tauri `WebviewWindow` opens loading `https://juejin.cn/login`
- **AND** the window displays the platform's native login interface

### Requirement: Cookie extraction after login detection
After the user completes login in the `WebviewWindow`, the Rust backend SHALL detect the successful login (by monitoring navigation to a post-login URL or cookie presence) and extract the platform's authentication cookie(s) via the Tauri Webview cookie API.

#### Scenario: Cookie extracted after successful login
- **WHEN** the user completes login and the Webview navigates to the platform's homepage
- **THEN** the Rust backend extracts the authentication cookie (e.g., `sessionid` for Juejin)
- **AND** the login Webview window is closed automatically

#### Scenario: Login cancelled or failed
- **WHEN** the user closes the login window without completing login
- **THEN** no cookie is extracted and the frontend receives a "login cancelled" status

### Requirement: DPAPI encrypted storage with no plaintext on disk
Extracted cookies SHALL be immediately encrypted via Windows DPAPI and stored in `tauri-plugin-store`. The on-disk store file SHALL NEVER contain plaintext cookie values. All cookie values SHALL be DPAPI-encrypted before being written and decrypted only when read at runtime.

#### Scenario: Cookie stored as ciphertext
- **WHEN** a cookie is saved to the store
- **THEN** the store file on disk contains only DPAPI-encrypted ciphertext for that cookie value

#### Scenario: No plaintext cookie in any store file
- **WHEN** the store file is inspected with a text editor or hex viewer
- **THEN** no plaintext cookie value is findable in the file content

#### Scenario: Legacy plaintext entry is not treated as valid
- **WHEN** a stored credential entry cannot be decrypted because it was written in an earlier plaintext format
- **THEN** the entry is reported as requiring re-authentication
- **AND** the app never falls back to using the raw stored value as a credential

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

### Requirement: one-click clear all login data
The settings page SHALL provide a "Clear All Login Data" action that deletes all stored credentials from `tauri-plugin-store`. This action SHALL require user confirmation before execution and SHALL irreversibly remove all encrypted credential entries.

#### Scenario: User clears all login data
- **WHEN** the user clicks "Clear All Login Data" and confirms the dialog
- **THEN** all credential entries are deleted from the store
- **AND** all platform status indicators change to "Not Logged In"

#### Scenario: Clear action requires confirmation
- **WHEN** the user clicks "Clear All Login Data"
- **THEN** a confirmation dialog is shown before any data is deleted

#### Scenario: Clearing credentials makes publish report not_configured
- **WHEN** a platform's credentials have been cleared and a publish is attempted
- **THEN** the publish result reports the credential as not configured
- **AND** no request is issued using a stale credential