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

### Requirement: wechat html sanitization
The WeChat adapter SHALL sanitize the HTML produced by the shared converter: every `<img>` element SHALL be replaced by the literal placeholder `[图片]`, and every `<a>` element SHALL be collapsed to its inner text so that no clickable external link reaches the draft.

#### Scenario: External image becomes a placeholder
- **WHEN** the generated HTML contains `<img src="https://example.com/a.png" />`
- **THEN** `content0` contains `[图片]` in its place
- **AND** it contains neither the image URL nor any `<img` markup

#### Scenario: Link is collapsed to its text
- **WHEN** the generated HTML contains `<a href="https://example.com/doc">文档</a>`
- **THEN** `content0` contains `文档`
- **AND** it contains no `<a` markup and no `href`

### Requirement: 51cto tag normalization
The 51CTO adapter SHALL split the incoming tag string on `,`, `，` and `、`, trim each segment, drop empty segments, remove duplicates while preserving order, truncate each tag to 20 characters, replace inner spaces with `-`, and join the result with `,`. The combined string SHALL NOT exceed 100 characters; when it would, whole tags SHALL be dropped from the tail rather than truncating mid-tag.

#### Scenario: Comma separated tags stay separate
- **WHEN** the tag input is `a,b,c`
- **THEN** the submitted `tag` field contains three comma separated tags
- **AND** the separators are never replaced by spaces

#### Scenario: Long tag is truncated to 20 characters
- **WHEN** a single tag is longer than 20 characters
- **THEN** the submitted tag keeps only its first 20 characters
- **AND** no truncation marker is appended inside the tag text

#### Scenario: Inner spaces become hyphens
- **WHEN** a tag contains an inner space
- **THEN** the submitted tag carries `-` in place of that space

#### Scenario: Duplicates are removed
- **WHEN** the same tag appears twice
- **THEN** it is submitted once
- **AND** the relative order of the remaining tags is preserved

#### Scenario: Overflow is reported
- **WHEN** normalized tags exceed the combined limit and tags are dropped
- **THEN** the result message states that tags were trimmed
- **AND** the sync is not reported as failed

### Requirement: 51cto article content format
The 51CTO draft and publish requests SHALL submit the article body as the original Markdown source, because the platform editor renders Markdown itself and escapes HTML tags when handed HTML.

#### Scenario: Markdown source is passed through unchanged
- **WHEN** a 51CTO draft is created from Markdown containing a list
- **THEN** the submitted `content` field carries the literal `- ` list markers
- **AND** it carries no `<ul>` markup

### Requirement: cnblogs article content format
The cnblogs adapter SHALL submit the article body to MetaWeblog `description` as XML-escaped HTML produced by the shared converter.

#### Scenario: Body is HTML before XML escaping
- **WHEN** a cnblogs post is created from Markdown containing a heading
- **THEN** the XML payload carries escaped `<h1>` markup rather than a literal `#` prefix
- **AND** the payload stays well-formed XML

### Requirement: xiaohongshu note body is plain text
The xiaohongshu adapter SHALL strip Markdown markers from the note body before injecting it, preserving the text content and line breaks, so that markers such as `#`, `*` and backticks never reach the note description.

#### Scenario: Markdown markers are removed
- **WHEN** the note body contains `# 标题` and `**加粗**`
- **THEN** the injected content contains `标题` and `加粗`
- **AND** it contains no `#` or `*` markers

#### Scenario: Line breaks are preserved
- **WHEN** the note body spans multiple lines
- **THEN** the injected content keeps those line breaks

### Requirement: douyin long article format
The douyin adapter SHALL submit `long_article` as HTML produced by the shared converter, and SHALL log the first 100 characters of that value before injecting the draft script.

#### Scenario: Long article carries HTML
- **WHEN** a douyin draft is built from Markdown containing a code block
- **THEN** the injected `long_article` carries `<pre><code>` markup

#### Scenario: Payload prefix is logged
- **WHEN** the draft script is about to be injected
- **THEN** a log line records the first 100 characters of `long_article`
- **AND** the full body is never logged

### Requirement: douyin login predicate reads the cookie jar
The douyin adapter SHALL treat login as established only when the `passport_assist_user` cookie is present with a non-empty value in the WebView2 cookie jar, read through `cookies_for_url`. The adapter SHALL NOT use the current page URL as a login signal, because the creator console is a single page application that renders `/creator-micro/` route skeletons while unauthenticated.

#### Scenario: SPA route skeleton is not login
- **WHEN** the window shows a `/creator-micro/` route but `passport_assist_user` is absent or empty
- **THEN** login is not reported as successful
- **AND** the window stays open for the user to finish logging in
- **AND** the page URL is never consulted to decide login

#### Scenario: Empty cookie value is not login
- **WHEN** a cookie named `passport_assist_user` exists but its value is empty
- **THEN** login is not reported as successful

#### Scenario: Cookie is accepted after three probes
- **WHEN** `passport_assist_user` is present with a non-empty value on three consecutive probes spaced 500ms apart
- **THEN** login is reported as successful
- **AND** the adapter navigates to the article editor

#### Scenario: Login timeout keeps the window open
- **WHEN** no login is confirmed before the login timeout expires
- **THEN** the failure message names the cookie jar contents and the predicate result
- **AND** the channel window remains open

### Requirement: douyin login status banner
While waiting for login, the adapter SHALL keep a fixed banner at the top of the channel page that states the current login state, refreshing it on every probe so it survives full page reloads and SPA route changes. The banner SHALL NOT intercept pointer events, and it SHALL be injected idempotently.

#### Scenario: Banner reports the waiting state
- **WHEN** the predicate reports not logged in
- **THEN** the banner is visible with the waiting text
- **AND** repeated injection does not stack additional banners
- **AND** the banner does not block clicks on the page beneath it

#### Scenario: Banner turns green before the window closes
- **WHEN** login is confirmed through the `open platform page` entry
- **THEN** the banner switches to the success text and colour
- **AND** the window stays open for a perceptible dwell period before closing

#### Scenario: Banner follows the predicate after a reload
- **WHEN** a full page reload or SPA route change removes the banner
- **THEN** the banner is re-created on the next probe with the state matching the current predicate result

### Requirement: douyin login failure reporting
The adapter SHALL distinguish a definitive authentication failure from a non-definitive one when notifying the front end, so that a still-open login window is never reported as a logged-out platform.

#### Scenario: Timeout is non-definitive
- **WHEN** the login wait times out while the channel window remains open
- **THEN** the failure event is emitted without the definitive flag
- **AND** the front end does not downgrade the platform login state

#### Scenario: Closed window without login is definitive
- **WHEN** the channel window is closed and no login cookie was ever observed
- **THEN** the failure event is emitted with the definitive flag
- **AND** the front end may mark the webview platform as logged out

### Requirement: douyin auth failure retry
When the douyin draft API reports `status_code = 8`, the adapter SHALL surface the channel window, wait for the user to complete login using the cookie predicate, navigate back to the article editor, and retry the draft request exactly once.

#### Scenario: Unauthenticated draft is retried after login
- **WHEN** the draft API responds with `status_code = 8`
- **THEN** the channel window is shown and focused
- **AND** the draft request is re-sent once after login is confirmed

#### Scenario: Retry happens only once
- **WHEN** the retried draft request also reports `status_code = 8`
- **THEN** the sync fails with the authentication failure class
- **AND** no further retry is attempted

#### Scenario: Stale script state is cleared before retry
- **WHEN** the draft request is retried
- **THEN** the page-side status and started markers are cleared first
- **AND** the previous failure status is not replayed as the retry result

### Requirement: aliyun login predicate reads the cookie jar
The aliyun adapter SHALL confirm login through Rust-side cookie polling rather than a DOM probe, because the SSO credential is `HttpOnly`. Login SHALL be reported only when one of `login_aliyunid_ticket`, `login_aliyunid` or `aliyun_session` is present with a value that is non-empty, not `0` and not `null`. The generic auth-key hint list SHALL NOT be used for this platform.

#### Scenario: SSO ticket confirms login
- **WHEN** the cookie jar contains `login_aliyunid_ticket` with a non-empty value
- **THEN** login is reported as successful

#### Scenario: Anonymous visitor is not logged in
- **WHEN** the cookie jar holds only analytics, risk-control and CSRF cookies such as `cna`, `cookie2`, `_tb_token_`, `t` or `c_csrf`
- **THEN** login is not reported as successful
- **AND** the window stays open for the user to finish logging in

#### Scenario: Bare `t` is never a login signal
- **WHEN** `t` is the only login-ish field present
- **THEN** login is not reported as successful
- **AND** the window is not closed

#### Scenario: Empty ticket value is not login
- **WHEN** `login_aliyunid_ticket` or `login_aliyunid` exists but its value is empty, `0` or `null`
- **THEN** login is not reported as successful

### Requirement: aliyun cookie lookup spans login and business domains
The adapter SHALL read cookies from `login.aliyun.com`, `account.aliyun.com` and `developer.aliyun.com`, because the SSO ticket is issued on the unified login domain while the `c_csrf` gateway token is issued as a host-only cookie on the business domain.

#### Scenario: Both credential classes are collected
- **WHEN** the poller collects cookies for aliyun
- **THEN** all three domains are queried
- **AND** duplicate URLs are not queried twice

#### Scenario: Cookie key names are logged per poll
- **WHEN** the aliyun poller runs a probe
- **THEN** a log line records the cookie key names for that probe
- **AND** cookie values are never logged

### Requirement: oschina login predicate reads the cookie jar
The oschina adapter SHALL confirm login through Rust-side cookie polling rather than a DOM probe, because `oscid` is an `HttpOnly` session ticket. Login SHALL be reported only when one of `oscid`, `oschina_new_user` or `oschina` is present with a value that is non-empty, not `0` and not `null`.

#### Scenario: oscid confirms login
- **WHEN** the cookie jar contains `oscid` with a non-empty value
- **THEN** login is reported as successful

#### Scenario: Anonymous visitor is not logged in
- **WHEN** the cookie jar holds only analytics, locale and source-marker cookies, and `oscid` is absent or empty
- **THEN** login is not reported as successful
- **AND** the window stays open for the user to finish logging in

#### Scenario: Empty ticket value is not login
- **WHEN** `oscid` exists but its value is empty, `0` or `null`
- **THEN** login is not reported as successful

### Requirement: oschina cookie lookup spans the profile subdomain
The adapter SHALL read cookies from both `www.oschina.net` and `my.oschina.net`, because the login entry lives on the former while the ticket is typically issued on the profile subdomain that `apiv1.oschina.net` exchanges for a session.

#### Scenario: Profile subdomain is queried
- **WHEN** the poller collects cookies for oschina
- **THEN** `my.oschina.net` is queried in addition to the login entry URL

#### Scenario: Repeated same-domain lookups do not duplicate cookie names
- **WHEN** two query URLs share the same host but differ in path
- **THEN** both URLs are queried
- **AND** the assembled cookie string still holds each cookie name once

### Requirement: baijiahao login predicate reads the cookie jar
The baijiahao adapter SHALL confirm login through Rust-side cookie polling rather than a DOM probe, because `BDUSS` is an `HttpOnly` Baidu passport cookie. Login SHALL be reported only when one of `BDUSS`, `BDUSS_BFESS`, `STOKEN` or `PTOKEN` is present with a value that is non-empty, not `0` and not `null`.

#### Scenario: BDUSS confirms login
- **WHEN** the cookie jar contains `BDUSS` with a non-empty value
- **THEN** login is reported as successful

#### Scenario: Anonymous visitor is not logged in
- **WHEN** the cookie jar holds only Baidu anonymous identifiers and statistics such as `BAIDUID`, `BAIDUID_BFESS`, `BIDUPSID`, `PSTM` or `H_PS_PSSID`
- **THEN** login is not reported as successful
- **AND** the window stays open for the user to finish logging in

#### Scenario: Empty passport ticket is not login
- **WHEN** `BDUSS` or `STOKEN` exists but its value is empty, `0` or `null`
- **THEN** login is not reported as successful

### Requirement: baijiahao cookie lookup spans the passport domain
The adapter SHALL read cookies from both `baijiahao.baidu.com` and `passport.baidu.com`, because the passport ticket may be issued as a host-only cookie on the passport subdomain.

#### Scenario: Passport domain is queried
- **WHEN** the poller collects cookies for baijiahao
- **THEN** `passport.baidu.com` is queried in addition to the business domain

### Requirement: netease login predicate reads the cookie jar
The netease adapter SHALL confirm login through Rust-side cookie polling rather than a DOM probe, because the session ticket is `HttpOnly`. Login SHALL require `NTES_YD_SESS`, the mp.163.com business-domain ticket, carrying a non-empty value that is not `0` or `null`; `NTES_SESS` and `NTES_SESSID` SHALL be fallbacks only and SHALL NOT be ordered ahead of it.

#### Scenario: Business-domain ticket confirms login
- **WHEN** the cookie jar contains `NTES_YD_SESS` with a non-empty value
- **THEN** login is reported as successful

#### Scenario: Passport session confirms login as a fallback
- **WHEN** the cookie jar contains `NTES_SESS` or `NTES_SESSID` with a non-empty value, but no `NTES_YD_SESS`
- **THEN** login is reported as successful

#### Scenario: Anonymous visitor is not logged in
- **WHEN** the cookie jar holds only anonymous or attribution cookies such as `P_INFO`, `S_INFO`, `NTESwebSI`, `_ntes_nuid`, `_ntes_origin_from` or `_antanalysis_s_id`
- **THEN** login is not reported as successful
- **AND** the window stays open for the user to finish logging in

#### Scenario: Empty session ticket is not login
- **WHEN** `NTES_YD_SESS` exists but its value is empty, `0` or `null`
- **THEN** login is not reported as successful

### Requirement: netease cookie lookup spans the passport domains
The adapter SHALL read cookies from `mp.163.com`, `reg.163.com` and `passport.163.com`, because the session ticket may be issued as a host-only cookie on a 163 passport subdomain.

#### Scenario: Passport domains are queried
- **WHEN** the poller collects cookies for netease
- **THEN** `reg.163.com` and `passport.163.com` are queried in addition to the business domain

### Requirement: unstable credential platforms log their cookie key names
Platforms whose credential key names cannot be confirmed in advance SHALL log the sorted cookie key names on every poll, so a real-device run can reveal the actual names. Cookie values SHALL never appear in these logs.

#### Scenario: Key names are logged for aliyun, oschina, baijiahao and netease
- **WHEN** the poller probes aliyun, oschina, baijiahao or netease
- **THEN** a log line lists the cookie key names, sorted and de-duplicated
- **AND** no cookie value is included

#### Scenario: Settled platforms do not spam the log
- **WHEN** the poller probes a platform whose predicate is already confirmed, such as 51CTO
- **THEN** no per-poll key-name line is emitted

### Requirement: zhihu body injection prefers a simulated paste
The Zhihu editor script SHALL fill the body by dispatching a synthetic `paste` event carrying a `DataTransfer` with both `text/html` and `text/plain`, so the Draft.js editor parses the HTML through its own paste handler. It SHALL fall back to `insertHTML`, then direct `innerHTML` assignment, then plain-text `insertText` only when the paste probe fails, and SHALL verify the body through a text probe before publishing.

#### Scenario: Paste is the first attempt
- **WHEN** the body editor is located and is still empty
- **THEN** the script dispatches a `ClipboardEvent('paste')` with `text/html` and `text/plain` data
- **AND** `insertHTML`, `innerHTML` and `insertText` are not attempted first

#### Scenario: Paste failure falls back in order
- **WHEN** the paste probe does not find the expected text in the editor
- **THEN** `insertHTML` is attempted next, then `innerHTML`, then plain text
- **AND** each attempt is followed by the same body probe

#### Scenario: Unfilled body is never published
- **WHEN** every injection attempt leaves the probe text absent from the editor
- **THEN** the script reports `FAIL` and does not click the publish button

#### Scenario: Zhihu body uses the downgraded HTML
- **WHEN** the Zhihu adapter prepares the body
- **THEN** it uses the shared Zhihu downgrade rather than the generic converter
- **AND** no `<table>` or `<pre>` markup reaches the editor

### Requirement: HttpOnly credential platforms follow one onboarding sequence
A platform whose login credential is `HttpOnly` SHALL be onboarded through a single atomic sequence: enable the poller allowlist, extend the supplemental cookie URLs, add a platform-specific predicate, register the key-name log label, and add unit tests. These steps SHALL NOT be split across separate changes, because a partially onboarded platform falls back to the generic substring allowlist and may close the window on an anonymous cookie.

#### Scenario: Predicate exists before polling is enabled
- **WHEN** a platform is added to the poller allowlist
- **THEN** a platform-specific predicate already exists for it
- **AND** the generic auth-key hint list is not consulted for that platform

### Requirement: aliyun draft payload matches the editor schema
The aliyun adapter SHALL submit the `putDraft` draft request body with exactly the field names the platform editor itself uses: `title`, `content`, `contentRender`, `format`, `aid`, `productTags` and `freeTierVOS`. The `content` field SHALL carry the Markdown source verbatim — the platform parses Markdown itself, so the adapter SHALL NOT convert it to HTML. The fields `markdown`, `categoryId` and `tagIds` SHALL NOT appear in the payload: they have never been part of the platform contract, and their presence indicates a regression.

#### Scenario: Draft body carries the editor field names
- **WHEN** a draft is created or updated through `putDraft`
- **THEN** the request body contains exactly the keys `title`, `content`, `contentRender`, `format`, `aid`, `productTags` and `freeTierVOS`
- **AND** `contentRender` is an empty string and `format` is null, matching the editor behaviour

#### Scenario: Content is submitted as Markdown source
- **WHEN** the article content contains Markdown syntax such as headings or lists
- **THEN** the `content` field receives that source text unchanged
- **AND** no HTML conversion is applied before submission

#### Scenario: Regressed field names fail the contract test
- **WHEN** the payload contains `markdown`, `categoryId` or `tagIds`
- **THEN** the contract test fails
- **AND** the fix is made in `build_draft_payload` against a fresh capture, not by editing the test alone