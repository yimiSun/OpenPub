# monorepo-infra Specification

## Purpose
提供 OpenPub 项目的 Monorepo 工作空间基础设施，统一管理多包依赖、构建与代码规范，确保跨包类型引用和开发体验的一致性。

## Requirements

### Requirement: pnpm workspace configuration
The project root SHALL contain a `pnpm-workspace.yaml` file that declares `packages/core`, `packages/ui`, `packages/adapters/*`, and `apps/desktop-app` as workspace members.

#### Scenario: Workspace resolves all packages
- **WHEN** `pnpm install` is executed at the project root
- **THEN** all workspace packages are resolved and symlinked correctly

### Requirement: shared TypeScript configuration
The project root SHALL provide a base `tsconfig.json` with strict mode enabled, and each workspace package SHALL extend this base configuration via `"extends"`.

#### Scenario: Package inherits strict TypeScript settings
- **WHEN** a workspace package's `tsconfig.json` extends the root config
- **THEN** `strict`, `esModuleInterop`, `skipLibCheck`, and `resolveJsonModule` are all inherited as `true`

### Requirement: unified code formatting
The project SHALL include ESLint and Prettier configurations at the root level, and all workspace packages SHALL be subject to the same linting and formatting rules.

#### Scenario: Lint runs across all packages
- **WHEN** `pnpm run lint` is executed at the project root
- **THEN** ESLint checks all workspace packages using the shared configuration

### Requirement: root scripts orchestration
The root `package.json` SHALL define `build`, `dev`, `lint`, and `typecheck` scripts that run the corresponding script in each workspace package via `pnpm -r run`.

#### Scenario: Build all packages from root
- **WHEN** `pnpm run build` is executed at the project root
- **THEN** all workspace packages are built in dependency order
