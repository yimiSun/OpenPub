# Spec Delta

## Purpose

提供 OpenPub Core 层的类型契约，定义 PlatformAdapter 接口及所有关联类型，作为平台适配器与调度器之间的编译时保障，确保任何适配器实现都满足统一的同步协议。

## ADDED Requirements

### Requirement: PublishOptions type definition
The `packages/core/src/types.ts` SHALL export a `PublishOptions` interface with the following fields: `title: string`, `markdown: string`, `tags?: string[]`, `coverImage?: string`, `isDraftOnly: boolean`.

#### Scenario: PublishOptions compiles with all required fields
- **WHEN** a `PublishOptions` object is created with `title`, `markdown`, and `isDraftOnly`
- **THEN** TypeScript compilation succeeds

#### Scenario: PublishOptions rejects missing required field
- **WHEN** a `PublishOptions` object is created without `title`
- **THEN** TypeScript compilation fails with a type error

### Requirement: SyncResult type definition
The `packages/core/src/types.ts` SHALL export a `SyncResult` interface with the following fields: `success: boolean`, `platform: string`, `draftUrl?: string`, `errorMsg?: string`.

#### Scenario: SyncResult represents success
- **WHEN** a sync operation succeeds
- **THEN** a `SyncResult` with `success: true`, `platform`, and `draftUrl` is returned

#### Scenario: SyncResult represents failure
- **WHEN** a sync operation fails
- **THEN** a `SyncResult` with `success: false`, `platform`, and `errorMsg` is returned

### Requirement: PlatformAdapter interface definition
The `packages/core/src/types.ts` SHALL export a `PlatformAdapter` interface with: `readonly id: string`, `readonly name: string`, `readonly env: ("browser" | "api")[]`, `checkAuth(): Promise<boolean>`, `preprocess?(options: PublishOptions): Promise<PublishOptions>`, `uploadImages(images: string[], context: any): Promise<string[]>`, `sync(options: PublishOptions, context: any): Promise<SyncResult>`.

#### Scenario: Adapter implementing full interface compiles
- **WHEN** a class implements all required and optional methods of `PlatformAdapter`
- **THEN** TypeScript compilation succeeds

#### Scenario: Adapter missing required method fails compilation
- **WHEN** a class implements `PlatformAdapter` but omits `sync`
- **THEN** TypeScript compilation fails with a type error

### Requirement: core package exports
The `packages/core/src/index.ts` SHALL re-export all public types: `PublishOptions`, `SyncResult`, and `PlatformAdapter`.

#### Scenario: Consumer imports types from core
- **WHEN** another package imports from `@openpub/core`
- **THEN** `PublishOptions`, `SyncResult`, and `PlatformAdapter` are accessible