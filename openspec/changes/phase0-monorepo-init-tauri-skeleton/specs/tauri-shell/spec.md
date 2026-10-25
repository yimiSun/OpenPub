# Spec Delta

## Purpose

提供 OpenPub 的 Tauri v2 桌面 App 壳，作为 Windows 桌面端的宿主环境，承载窗口生命周期、Rust 命令桥接、系统凭据加密存储、本地文件读写和 Windows 安装包构建等宿主能力。

## ADDED Requirements

### Requirement: Tauri v2 application initialization
The `apps/desktop-app` SHALL be initialized via `create-tauri-app` with React + TypeScript template targeting Tauri v2, and `pnpm tauri dev` SHALL launch the main window with hot-reload enabled.

#### Scenario: Dev server starts with hot reload
- **WHEN** `pnpm tauri dev` is executed
- **THEN** the Tauri desktop window opens and React frontend changes are reflected without manual refresh

### Requirement: Rust commands return Result and never silently swallow errors
All Rust Tauri commands SHALL return `Result<T, E>` where `E` is a serializable error type. The use of `unwrap()`, `unwrap_or_default()`, or any pattern that discards error information SHALL be prohibited. Errors SHALL be propagated to the frontend via the `invoke` rejection handler.

#### Scenario: Rust command failure surfaces in UI
- **WHEN** a Rust command encounters an error (e.g., file not found, network failure)
- **THEN** the error is returned as `Err(E)`, the frontend `invoke` call rejects, and the UI displays the error message to the user

#### Scenario: unwrap_or_default is rejected at review
- **WHEN** a code review finds `unwrap_or_default()` in a Tauri command
- **THEN** the code is rejected and must be rewritten to use proper `Result` propagation

### Requirement: async commands for long-running operations
Any Rust command that performs I/O (network requests, file reads/writes, image processing) or may take longer than 50ms SHALL be declared as `async` Tauri commands to avoid blocking the main thread.

#### Scenario: Network request does not freeze UI
- **WHEN** a Rust command performs an HTTP request
- **THEN** the command is declared `async` and the UI remains responsive during the request

### Requirement: encrypted credential storage via tauri-plugin-store with DPAPI
The app SHALL use `tauri-plugin-store` for persisting platform credentials (cookies/tokens). On Windows, the store SHALL be configured to use DPAPI encryption so that credentials are never stored in plaintext on disk.

#### Scenario: Cookie stored with DPAPI encryption
- **WHEN** a platform cookie is saved via the store
- **THEN** the on-disk store file contains DPAPI-encrypted ciphertext, not the plaintext cookie value

#### Scenario: Cookie retrieved after DPAPI decryption
- **WHEN** a stored cookie is read from the store
- **THEN** DPAPI decrypts the ciphertext and the original cookie value is returned to the caller

#### Scenario: Different Windows user cannot decrypt
- **WHEN** the store file is accessed under a different Windows user account
- **THEN** DPAPI decryption fails and the credential is inaccessible

### Requirement: Windows installer packaging
The `tauri.conf.json` SHALL configure `bundle.targets` to include both `nsis` (for `.exe` installer) and `msi` (for `.msi` installer). `pnpm tauri build` SHALL produce both installer formats.

#### Scenario: Build produces exe and msi
- **WHEN** `pnpm tauri build` is executed on Windows
- **THEN** both a `.exe` (NSIS) and a `.msi` installer are produced in the target directory

### Requirement: pnpm tauri dev hot-debug environment
The project SHALL support `pnpm tauri dev` for development with hot module replacement on the React frontend and automatic Rust recompilation on backend changes.

#### Scenario: Frontend change reflects instantly
- **WHEN** a React component file is modified while `pnpm tauri dev` is running
- **THEN** the change is hot-reloaded in the Tauri window without a full restart

#### Scenario: Rust change triggers recompilation
- **WHEN** a Rust source file is modified while `pnpm tauri dev` is running
- **THEN** the Rust backend is recompiled and the app restarts with the new backend