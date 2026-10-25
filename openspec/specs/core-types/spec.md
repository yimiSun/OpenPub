# core-types Specification

## Purpose
定义 OpenPub Core 层的类型契约，确立 PlatformAdapter 接口及其关联类型，为所有平台适配器和调度器提供编译时类型保障与行为约束。

## Requirements

### Requirement: PlatformAdapter interface definition
The `packages/core` package SHALL export a `PlatformAdapter` interface with the following readonly properties: `id` (string), `name` (string), `env` (array of "browser" | "api"), and the following methods: `checkAuth()`, `preprocess?()`, `uploadImages()`, `sync()`.

#### Scenario: Adapter implements all required members
- **WHEN** a class implements the `PlatformAdapter` interface
- **THEN** TypeScript enforces that `id`, `name`, `env`, `checkAuth`, `uploadImages`, and `sync` are present with correct signatures

### Requirement: PublishOptions type definition
The `packages/core` package SHALL export a `PublishOptions` interface with fields: `title` (string), `markdown` (string), `tags` (optional string array), `coverImage` (optional string), `isDraftOnly` (boolean).

#### Scenario: PublishOptions enforces required fields
- **WHEN** a `PublishOptions` object is constructed without `title` or `markdown`
- **THEN** TypeScript produces a compilation error

### Requirement: SyncResult type definition
The `packages/core` package SHALL export a `SyncResult` interface with fields: `success` (boolean), `platform` (string), `draftUrl` (optional string), `errorMsg` (optional string).

#### Scenario: SyncResult represents successful sync
- **WHEN** a sync operation succeeds
- **THEN** a `SyncResult` with `success: true` and a populated `draftUrl` is returned

#### Scenario: SyncResult represents failed sync
- **WHEN** a sync operation fails
- **THEN** a `SyncResult` with `success: false` and a populated `errorMsg` is returned

### Requirement: core package exports
The `packages/core` package SHALL export all types from a single entry point (`src/index.ts`), and the package `package.json` SHALL declare these as both ESM and CJS exports.

#### Scenario: External package imports core types
- **WHEN** another workspace package imports from `@openpub/core`
- **THEN** all public types (`PlatformAdapter`, `PublishOptions`, `SyncResult`) are accessible
