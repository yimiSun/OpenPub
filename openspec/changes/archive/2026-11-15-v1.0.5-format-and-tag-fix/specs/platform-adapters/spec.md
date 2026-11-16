# Spec Delta

## ADDED Requirements

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
The 51CTO draft and publish requests SHALL submit the article body as HTML produced by the shared converter.

#### Scenario: Markdown source is converted
- **WHEN** a 51CTO draft is created from Markdown containing a list
- **THEN** the submitted `content` field carries `<ul>` markup
- **AND** it carries no literal `- ` list markers

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

#### Scenario: Anonymous cookies never pass a dedicated predicate
- **WHEN** a platform has a dedicated predicate
- **THEN** a cookie string containing only anonymous identifiers is rejected
- **AND** a unit test covers that rejection

#### Scenario: Weak key names are documented with a fallback
- **WHEN** a predicate includes a key name that may also be issued to anonymous visitors
- **THEN** the predicate's documentation states that the key must be removed if a window-close-on-open is observed
- **AND** the documentation forbids loosening the predicate instead

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