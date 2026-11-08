# Spec Delta

## Purpose

定义 OpenPub 平台适配器的统一接入契约：平台注册表、发布路由与结果模型。所有平台适配器（含已跑通的博客园与掘金）都通过同一套路由与结果语义对外服务，使前端无需感知具体平台实现，并让新增平台只需注册一次。

## ADDED Requirements

### Requirement: unified publish result model
The publish command SHALL return a result carrying `success`, `platform`, `operation` (`create` | `update`), `isPublished`, optional `id`, optional `editUrl`, and a human-readable `message`, and SHALL classify failures into one of: `not_configured`, `auth_invalid`, `rate_limited`, `network_error`, `not_implemented`, `platform_error`.

#### Scenario: Successful draft creation reports draft URL
- **WHEN** a configured platform accepts the publish request
- **THEN** the result has `success: true`, a populated `editUrl` pointing at the platform's draft edit page
- **AND** the `message` confirms the draft was created rather than published

#### Scenario: Missing credentials classified as not_configured
- **WHEN** the publish request targets a platform whose required credentials are absent
- **THEN** the result has `success: false` with failure class `not_configured`
- **AND** the `message` names which credential field is missing

#### Scenario: Rejected credentials classified as auth_invalid
- **WHEN** the platform responds with an authentication or authorization rejection
- **THEN** the result has `success: false` with failure class `auth_invalid`
- **AND** the `message` instructs the user to re-authenticate the platform

#### Scenario: Unimplemented platform classified as not_implemented
- **WHEN** the publish request targets a registered platform that has no adapter implementation yet
- **THEN** the result has `success: false` with failure class `not_implemented`
- **AND** no network request is issued

#### Scenario: Result distinguishes create from update
- **WHEN** a publish request is dispatched with `is_update = false`
- **THEN** the result reports `operation: create`
- **AND** when `is_update = true` the result reports `operation: update`

#### Scenario: Result reports live publication state
- **WHEN** a publish request completes with `publish_status = true`
- **THEN** the result reports `isPublished: true`
- **AND** when `publish_status = false` the result reports `isPublished: false`

### Requirement: draft-first publishing for domestic platforms
Domestic platform adapters SHALL default to creating drafts. Live publication on a domestic platform SHALL occur only when `publish_status = true` is explicitly supplied for that request; absent that explicit flag the adapter SHALL NOT publish live.

#### Scenario: Publishing creates a draft by default
- **WHEN** a domestic platform adapter completes a publish request without `publish_status = true`
- **THEN** the platform-side artifact is a draft
- **AND** the returned `editUrl` leads to the platform's draft editing view

#### Scenario: Explicit override publishes live
- **WHEN** the request supplies `publish_status = true` and the platform supports live publication
- **THEN** the platform-side artifact is a live post
- **AND** the result reports `isPublished: true`

#### Scenario: Missing flag never publishes live
- **WHEN** a publish request omits `publish_status` or sets it to `false`
- **THEN** no adapter publishes a live post under any circumstance

### Requirement: cnblogs MetaWeblog adapter
The cnblogs adapter SHALL create posts via `metaWeblog.newPost` and SHALL update existing posts via `metaWeblog.editPost` when `is_update = true`, authenticating with blog name, user name, and access token. The publish boolean sent to MetaWeblog SHALL be derived from `publish_status` and SHALL NOT be hardcoded.

#### Scenario: Valid MetaWeblog configuration creates draft
- **WHEN** blog name, user name, and access token are all present and the call succeeds
- **THEN** the result has `success: true` and the returned post id is mapped into a cnblogs draft edit URL

#### Scenario: Incomplete MetaWeblog configuration is rejected before requesting
- **WHEN** any of blog name, user name, or access token is empty
- **THEN** the result has `success: false` with failure class `not_configured`
- **AND** no request is sent to the MetaWeblog endpoint

#### Scenario: XML-RPC fault surfaces as platform error
- **WHEN** the MetaWeblog endpoint returns a fault payload
- **THEN** the result has `success: false` with failure class `platform_error`
- **AND** the fault text is included in the `message`

#### Scenario: publish_status false sends a false publish boolean
- **WHEN** a cnblogs publish request is issued with `publish_status = false`
- **THEN** the XML-RPC body carries a false publish boolean
- **AND** the resulting article is a draft

#### Scenario: publish_status true sends a true publish boolean
- **WHEN** a cnblogs publish request is issued with `publish_status = true`
- **THEN** the XML-RPC body carries a true publish boolean
- **AND** the resulting article is published live

#### Scenario: Update calls editPost with the existing post id
- **WHEN** `is_update = true` and `existing_id` is supplied
- **THEN** the adapter calls `metaWeblog.editPost` with that post id
- **AND** no `metaWeblog.newPost` call is made

#### Scenario: Update without post id is rejected
- **WHEN** `is_update = true` but `existing_id` is empty
- **THEN** the result has `success: false` with failure class `not_configured`
- **AND** no request is sent to the MetaWeblog endpoint

### Requirement: juejin draft adapter
The juejin adapter SHALL create a draft via the content API draft-create endpoint and SHALL update an existing draft via the draft-update endpoint when `is_update = true` and the target is still a draft, using the stored cookie. It SHALL send the request with browser-equivalent user agent, referer, and origin headers.

#### Scenario: Valid cookie creates juejin draft
- **WHEN** a non-empty juejin cookie is stored and the draft-create endpoint returns success
- **THEN** the result has `success: true` and the returned draft id is mapped into a juejin draft edit URL

#### Scenario: Missing cookie is rejected before requesting
- **WHEN** no juejin cookie is stored
- **THEN** the result has `success: false` with failure class `not_configured`
- **AND** no request is sent to the juejin API

#### Scenario: Rejected cookie classified as auth_invalid
- **WHEN** the juejin draft-create endpoint responds with a login-required or permission error code
- **THEN** the result has `success: false` with failure class `auth_invalid`

#### Scenario: Update targets the draft update endpoint
- **WHEN** `is_update = true`, `existing_id` is supplied, and the target article is not yet published
- **THEN** the adapter calls the juejin draft-update endpoint with that draft id
- **AND** no draft-create request is made

#### Scenario: Published article update targets the article update endpoint
- **WHEN** `is_update = true`, `existing_id` is supplied, and the target article is published
- **THEN** the adapter calls the juejin article-update endpoint with that article id
- **AND** the draft-update endpoint is not used

#### Scenario: Juejin create-live is not supported yet
- **WHEN** a juejin publish request is issued with `is_update = false` and `publish_status = true`
- **THEN** the result has `success: false` with failure class `not_implemented`
- **AND** the `message` states that juejin currently supports draft saving only and the user should publish manually from the juejin console
- **AND** no draft-create request is made with a live-publish expectation

### Requirement: non-ASCII safe response truncation
Any platform adapter that truncates a response body for inclusion in an error message SHALL truncate by characters, not bytes, so that multi-byte characters are never split.

#### Scenario: Chinese response body does not panic
- **WHEN** a platform returns an error body containing Chinese characters longer than the truncation limit
- **THEN** the adapter produces a truncated string without panicking
- **AND** the truncated text remains valid UTF-8

### Requirement: update existing post semantics
When a publish request carries `is_update = true` and a non-empty `existing_id`, the adapter SHALL update the identified post instead of creating a new one. An update failure SHALL be reported as a failure and SHALL NOT silently fall back to creating a new post.

#### Scenario: Update never creates a duplicate
- **WHEN** a publish request with `is_update = true` and `existing_id` succeeds
- **THEN** the platform-side post count for that article remains one
- **AND** the returned id equals the supplied `existing_id`

#### Scenario: Failed update does not fall back to create
- **WHEN** an update request fails at the platform
- **THEN** the result has `success: false` with an appropriate failure class
- **AND** no create request is issued as a fallback

#### Scenario: Non-existing post id reported as platform error
- **WHEN** the supplied `existing_id` does not exist on the platform (e.g., the post was deleted)
- **THEN** the result has `success: false` with failure class `platform_error`
- **AND** the `message` states that the remote post was not found