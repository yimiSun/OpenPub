# Spec Delta

## Purpose

提供适配器脚手架工具，让开发者通过一条命令生成符合 PlatformAdapter 接口的平台适配器模板代码，降低社区贡献者的上手门槛。

## ADDED Requirements

### Requirement: create-adapter script
The project SHALL provide a `create-adapter` script (invoked via `pnpm create-adapter <platform-name>`) that generates a new adapter package under `packages/adapters/<platform-name>/` with all required files.

#### Scenario: Generate adapter for new platform
- **WHEN** `pnpm create-adapter bilibili` is executed
- **THEN** a directory `packages/adapters/bilibili/` is created containing `package.json`, `tsconfig.json`, `src/index.ts`, and `src/adapter.ts` with a skeleton implementing `PlatformAdapter`

### Requirement: generated adapter implements PlatformAdapter
The generated adapter skeleton SHALL implement the `PlatformAdapter` interface with stub methods: `checkAuth` returning `false`, `uploadImages` returning input unchanged, and `sync` returning a failed `SyncResult`.

#### Scenario: Generated adapter compiles without errors
- **WHEN** the generated adapter is built via `pnpm build`
- **THEN** TypeScript compilation succeeds with no errors

### Requirement: generated adapter registered in workspace
The `create-adapter` script SHALL update `pnpm-workspace.yaml` to include the new adapter path if not already present, and add the adapter as a dependency to `packages/core` if applicable.

#### Scenario: New adapter is part of workspace
- **WHEN** `create-adapter` completes for a new platform
- **THEN** `pnpm install` at the root resolves the new adapter package correctly