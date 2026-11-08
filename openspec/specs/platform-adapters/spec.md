# platform-adapters Specification

## Purpose
定义 OpenPub 平台适配器的统一接入契约：平台注册表、发布路由与结果模型。所有平台适配器（含已跑通的博客园与掘金）都通过同一套路由与结果语义对外服务，使前端无需感知具体平台实现，并让新增平台只需注册一次。

## Requirements

### Requirement: unified publish result model
The publish command SHALL return a result carrying `success`, `platform`, `operation` (`create` | `update`), `isPublished`, optional `id`, optional `editUrl`, and a human-readable `message`, and SHALL classify failures into one of: `not_configured`, `auth_invalid`, `platform_error`, `network_error`, `not_implemented`, `unknown`.

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

#### Scenario: Unpublished target never uses the live update endpoint
- **WHEN** `is_update = true` and the target article is not yet published
- **THEN** the adapter routes to the draft-update endpoint even if `publish_status = true`
- **AND** the request body carries `draft_id` rather than `article_id`

#### Scenario: Published article update targets the article update endpoint
- **WHEN** `is_update = true`, `existing_id` is supplied, and the target article is published
- **THEN** the adapter calls the juejin article-update endpoint with that article id
- **AND** the draft-update endpoint is not used

#### Scenario: Juejin create-live is not supported yet
- **WHEN** a juejin publish request is issued with `is_update = false` and `publish_status = true`
- **THEN** the result has `success: false` with failure class `not_implemented`
- **AND** the `message` states that juejin currently supports draft saving only and the user should publish manually from the juejin console
- **AND** no draft-create request is made with a live-publish expectation

### Requirement: juejin draft update request body
The juejin draft-update request SHALL carry `title`, `mark_content` (Markdown source), `content` (HTML rendering), `category_id` set to a valid juejin category identifier, `tag_ids` as an array, `brief_content`, `edit_type`, and both `draft_id` and `id` set to the target draft id.

#### Scenario: Markdown and HTML content are both supplied
- **WHEN** a juejin draft-update request is built
- **THEN** `mark_content` carries the Markdown source and `content` carries an HTML rendering of it
- **AND** neither field is omitted

#### Scenario: Category identifier is never a placeholder
- **WHEN** a juejin draft-update request is built
- **THEN** `category_id` carries a real juejin category identifier
- **AND** it is never `"0"` or an empty string

#### Scenario: Empty tag list is sent as an empty array
- **WHEN** the article has no tags
- **THEN** `tag_ids` is an empty array rather than an omitted field

#### Scenario: Draft id is supplied under both key names
- **WHEN** a juejin draft-update request targets draft `D`
- **THEN** both `draft_id` and `id` equal `D`

### Requirement: juejin response id extraction
The juejin adapter SHALL extract the created or updated id from the response payload by probing the identifier keys `id`, `draft_id`, `article_id`, `articleId`, and `post_id`, accepting both string and numeric values, and SHALL treat `""`, `"0"`, `"未知"`, and `"(未知)"` as absent identifiers.

#### Scenario: Identifier read from data id
- **WHEN** the response payload is `{"data": {"id": "7702475357690544137"}}`
- **THEN** the extracted id is `7702475357690544137`

#### Scenario: Placeholder article id is skipped
- **WHEN** the response payload carries `article_id: "0"` alongside a real `draft_id`
- **THEN** the placeholder is ignored and the real draft id is extracted

#### Scenario: Extraction failure is reported instead of substituted
- **WHEN** a success response contains no usable identifier
- **THEN** the result has `success: false` with failure class `platform_error`
- **AND** the message includes a snippet of the raw response
- **AND** no placeholder identifier is ever persisted into the sync map

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

### Requirement: platform registry single source of truth
The set of supported platform ids SHALL be defined once in a shared registry and SHALL be mirrored by the backend dispatcher, such that the registry ids and the dispatched ids are provably equal. Each registry entry SHALL additionally declare its publish capability, and any platform declared available SHALL have a corresponding adapter implementation.

#### Scenario: Registry and dispatcher agree
- **WHEN** a consistency check compares the shared registry ids against the backend dispatcher's covered ids
- **THEN** the two sets are equal
- **AND** removing a dispatcher branch makes the check fail

#### Scenario: Unregistered platform id is rejected
- **WHEN** a publish request targets an id absent from the registry
- **THEN** the result has `success: false`
- **AND** the error message contains that unknown id

#### Scenario: Available platform must have an adapter
- **WHEN** a platform is marked available in the registry
- **THEN** that platform id is present in the implemented-adapter list
- **AND** the consistency check fails if it is missing

#### Scenario: Implemented adapter must be marked available
- **WHEN** a platform id appears in the implemented-adapter list but is marked unavailable in the registry
- **THEN** the consistency check reports that platform as mislabeled

#### Scenario: Publish capability is declared per platform
- **WHEN** the registry declares a platform
- **THEN** it states whether that platform supports live publication or draft-only saving
- **AND** user-facing notices about draft-only behavior are derived from that declaration rather than from a platform id literal

#### Scenario: Draft-only platform suppresses live-publish expectations
- **WHEN** a draft-only platform is selected together with live publication
- **THEN** the user-facing notice states that the content will be saved as a draft for manual publication
- **AND** the same notice mechanism applies to every draft-only platform, not only to one hardcoded platform

### Requirement: csdn adapter
The csdn adapter SHALL save articles via a cookie-authenticated REST endpoint, sending the CSRF token alongside the cookie, and SHALL sign every request with the platform gateway's HMAC-SHA256 headers so the request reaches the business layer. Draft saving and live publication SHALL be two distinct actions sharing one save endpoint, differing only in the request body `status` field (`2` draft, `0` live), and a live-publication request SHALL NEVER be silently downgraded into a draft save.

#### Scenario: Valid cookie saves a csdn draft
- **WHEN** a non-empty csdn cookie is stored and the draft endpoint returns success
- **THEN** the result has `success: true`
- **AND** the returned article id is mapped into a csdn draft edit URL
- **AND** the request body carries `status: 2`

#### Scenario: Missing cookie is rejected before requesting
- **WHEN** no csdn cookie is stored
- **THEN** the result has `success: false` with failure class `not_configured`
- **AND** no request is sent to the csdn API

#### Scenario: Cookie without csrf token is rejected before requesting
- **WHEN** the stored csdn cookie does not contain the expected CSRF key
- **THEN** the result has `success: false` with failure class `not_configured`
- **AND** the message names the missing token so the user knows to re-copy the cookie

#### Scenario: Rejected cookie classified as auth_invalid
- **WHEN** the csdn endpoint responds with an authentication or login-required rejection
- **THEN** the result has `success: false` with failure class `auth_invalid`

#### Scenario: Live publication submits with status zero
- **WHEN** a csdn publish request is issued with `publish_status = true`
- **THEN** the adapter calls the save endpoint with `status: 0`
- **AND** the result has `success: true` when the platform accepts the submission
- **AND** the result reports `is_published: true`
- **AND** the message states that the article was published

#### Scenario: Live publication carries a tag
- **WHEN** a csdn publish request is issued with `publish_status = true` and no tag supplied
- **THEN** the request body carries a non-empty default tag
- **AND** the platform does not reject the request for a missing tag

#### Scenario: Draft publication keeps an empty tag
- **WHEN** a csdn draft request is issued with `publish_status = false` and no tag supplied
- **THEN** the request body carries an empty tag string

#### Scenario: Live article is updated when overwrite is requested
- **WHEN** `is_update = true`, `existing_id` is supplied, the target article is already live, and `publish_status = true`
- **THEN** the adapter submits the same article id with `status: 0`
- **AND** the request is not rejected

#### Scenario: Live target without overwrite is rejected before requesting
- **WHEN** `is_update = true`, `existing_id` is supplied, the target article is already live, and `publish_status = false`
- **THEN** the result has `success: false` with failure class `not_implemented`
- **AND** the message states that no request was sent and that the live article was left untouched
- **AND** the save endpoint is not called with the live article id

#### Scenario: Draft intent is served with the draft status
- **WHEN** a csdn publish request is accepted with `publish_status = false` and the target is not live
- **THEN** the adapter calls the single save endpoint with `status: 2`
- **AND** the body identifies the target article id on update and omits the id on create

#### Scenario: csdn request carries browser-equivalent headers
- **WHEN** the csdn adapter issues any request
- **THEN** it sends a browser-equivalent user agent, referer, origin, and content type
- **AND** the csrf token is sent in the header the platform expects

### Requirement: csdn draft request body
The csdn draft request SHALL carry the article title, the Markdown source, the HTML rendering produced by the shared converter, the platform's camel-case reading-permission field, and a tag value appropriate to the action: an empty string for a draft and a non-empty tag for a publication.

#### Scenario: Markdown and HTML content are both supplied
- **WHEN** a csdn draft request is built
- **THEN** the body carries the Markdown source and an HTML rendering of it
- **AND** neither field is omitted

#### Scenario: Reading permission uses the platform's field name
- **WHEN** a csdn request body is built
- **THEN** the reading permission is carried under the platform's camel-case field name
- **AND** the snake-case spelling is absent

#### Scenario: Draft and publication tag values differ
- **WHEN** a csdn draft request is built without tags
- **THEN** the tag field is an empty string
- **AND** when the request is a publication the tag field is non-empty

#### Scenario: Draft id is supplied on update
- **WHEN** a csdn draft update request targets draft `D`
- **THEN** the body identifies `D` as the article being updated

### Requirement: zhihu adapter
The zhihu adapter SHALL create an article through the zhihu content API using a cookie and an XSRF token header, and SHALL treat article creation and live publication as two distinct platform actions. Creation SHALL always be a draft; live publication SHALL require a separate follow-up action.

#### Scenario: Valid cookie and xsrf create a zhihu draft
- **WHEN** a stored zhihu cookie contains the expected XSRF key and the create endpoint returns success
- **THEN** the result has `success: true` with `isPublished: false`
- **AND** the returned article id is mapped into a zhihu draft edit URL

#### Scenario: Missing cookie is rejected before requesting
- **WHEN** no zhihu cookie is stored
- **THEN** the result has `success: false` with failure class `not_configured`
- **AND** no request is sent to the zhihu API

#### Scenario: Missing xsrf token is rejected before requesting
- **WHEN** the stored zhihu cookie lacks the XSRF key
- **THEN** the result has `success: false` with failure class `not_configured`
- **AND** the message names the missing token

#### Scenario: Draft-only request stops after creation
- **WHEN** a zhihu publish request is issued with `publish_status = false`
- **THEN** only the create action is performed
- **AND** no publication action is performed

#### Scenario: Live request performs a second publication action
- **WHEN** a zhihu publish request is issued with `publish_status = true`
- **THEN** the adapter creates the article first and then performs the publication action on the created id
- **AND** the result reports `isPublished: true` only after the publication action succeeds

#### Scenario: Unpublished target update never publishes implicitly
- **WHEN** `is_update = true`, `existing_id` is supplied, and the target article is not yet published
- **THEN** the adapter updates the draft content without performing the publication action unless `publish_status = true`
- **AND** the result reports `isPublished: false`

#### Scenario: Rejected cookie classified as auth_invalid
- **WHEN** the zhihu endpoint responds with an authentication or login-required rejection
- **THEN** the result has `success: false` with failure class `auth_invalid`

### Requirement: zhihu two-phase partial failure
When a zhihu request creates the article successfully but the follow-up publication action fails, the result SHALL report failure while still surfacing the created draft id, so the draft is not lost and the next sync can continue from it.

#### Scenario: Publication failure after successful creation reports the draft id
- **WHEN** the create action succeeds and the publication action fails
- **THEN** the result has `success: false` with failure class `platform_error`
- **AND** the result carries the created draft id and its edit URL
- **AND** the message states that the draft was saved but publishing failed

#### Scenario: Publication failure does not discard the draft
- **WHEN** the publication action fails after creation
- **THEN** no deletion or cleanup request is issued against the created article

### Requirement: markdown conversion delegated to shared utility
Every adapter that supplies HTML content to a platform SHALL obtain that HTML from the shared markdown utility, and SHALL NOT carry a private Markdown-to-HTML implementation.

#### Scenario: Adapter html matches shared conversion
- **WHEN** any adapter builds a request body containing HTML
- **THEN** the HTML equals the shared utility's conversion of the same Markdown source

#### Scenario: No private conversion implementation remains
- **WHEN** the adapter modules are inspected
- **THEN** no adapter defines its own HTML escaping or paragraph-wrapping routine