# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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