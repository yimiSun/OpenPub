# adapter-scaffold Specification

## Purpose
提供适配器脚手架工具，让开发者通过一条命令生成符合 PlatformAdapter 接口的平台适配器模板代码，降低社区贡献者的上手门槛。

## Requirements

### Requirement: create-adapter script generates adapter scaffold
The `scripts/create-adapter.mjs` script SHALL accept a platform name argument and generate a complete adapter package under `packages/adapters/<name>/` with: `package.json` (name `@openpub/adapter-<name>`, dependency on `@openpub/core`), `tsconfig.json` (extending root config), `src/index.ts` (re-exporting the adapter), and `src/adapter.ts` (implementing `PlatformAdapter` interface stubs).

#### Scenario: Generate adapter for a new platform
- **WHEN** `node scripts/create-adapter.mjs juejin` is executed
- **THEN** `packages/adapters/juejin/` is created with all required files
- **AND** the adapter stub implements all `PlatformAdapter` methods

#### Scenario: Generated adapter compiles
- **WHEN** a new adapter is generated
- **THEN** `tsc --noEmit` passes for the adapter package

### Requirement: workspace auto-update
The `create-adapter.mjs` script SHALL automatically update `pnpm-workspace.yaml` to include the new adapter path under `packages/adapters/*`, using `js-yaml` for safe YAML parsing and serialization.

#### Scenario: Workspace updated after adapter creation
- **WHEN** `create-adapter` generates a new adapter
- **THEN** `pnpm-workspace.yaml` includes the new adapter path
- **AND** `pnpm install` resolves the new package without errors

#### Scenario: Existing workspace entries preserved
- **WHEN** `create-adapter` updates `pnpm-workspace.yaml` for a new adapter
- **THEN** previously declared workspace members remain unchanged

### Requirement: root script registration
The root `package.json` SHALL include a `"create-adapter"` script that runs `node scripts/create-adapter.mjs`.

#### Scenario: Run create-adapter via pnpm
- **WHEN** `pnpm create-adapter test-platform` is executed
- **THEN** the adapter scaffold is generated successfully
