# Spec Delta

## Purpose

定义所有 Cookie 类平台适配器共用的凭据与内容处理契约：从浏览器 Cookie 串中提取指定键值、把 Markdown 转换为平台可接受的 HTML、提取 CSRF/XSRF 令牌、按字符安全截断错误响应。让 CSDN、知乎、掘金共用同一份实现，避免三份逻辑各自漂移。

## ADDED Requirements

### Requirement: cookie value extraction by key
The shared cookie utility SHALL extract the value of a named key from a browser cookie string, tolerating surrounding whitespace and an optional leading space after each `;` separator, and SHALL report a missing key as a configuration failure rather than returning an empty string.

#### Scenario: Named key is extracted from a cookie string
- **WHEN** the cookie string is `a=1; _xsrf=TOKEN; b=2` and the requested key is `_xsrf`
- **THEN** the extracted value is `TOKEN`
- **AND** values of other keys are unaffected

#### Scenario: Last occurrence wins for duplicated keys
- **WHEN** the cookie string contains the requested key twice
- **THEN** the extracted value is the one appearing last

#### Scenario: Missing key is reported rather than blank
- **WHEN** the requested key does not appear in the cookie string
- **THEN** the utility reports a configuration failure naming the missing key
- **AND** it never returns an empty string as a usable value

#### Scenario: Malformed segment is skipped
- **WHEN** the cookie string contains a segment without an `=` sign
- **THEN** that segment is ignored
- **AND** extraction of valid keys still succeeds

### Requirement: markdown to html conversion
The shared markdown utility SHALL convert Markdown source into an HTML fragment by escaping HTML special characters and wrapping each non-empty line in a paragraph element, and SHALL produce a single empty paragraph when the source has no non-empty line.

#### Scenario: Special characters are escaped
- **WHEN** the Markdown source contains `&`, `<`, or `>`
- **THEN** the resulting HTML contains their entity forms instead of raw characters

#### Scenario: Each non-empty line becomes a paragraph
- **WHEN** the Markdown source has three non-empty lines separated by blank lines
- **THEN** the resulting HTML contains three paragraph elements in source order

#### Scenario: Empty source yields one empty paragraph
- **WHEN** the Markdown source is empty or contains only whitespace
- **THEN** the resulting HTML is a single empty paragraph
- **AND** it is never an empty string

#### Scenario: Conversion is identical across platforms
- **WHEN** two adapters convert the same Markdown source
- **THEN** they produce byte-identical HTML
- **AND** no adapter carries its own private conversion implementation

### Requirement: csrf token extraction
The shared csrf utility SHALL extract a CSRF/XSRF token from the cookie string by key and SHALL URL-decode the value before returning it, since the token is stored percent-encoded.

#### Scenario: Percent-encoded token is decoded
- **WHEN** the cookie carries `_xsrf=abc%3D%3D` and the requested key is `_xsrf`
- **THEN** the returned token is `abc==`

#### Scenario: Token with no encoding is returned as-is
- **WHEN** the cookie carries a token containing no percent-encoding
- **THEN** the returned token equals the raw cookie value

#### Scenario: Missing token is reported rather than blank
- **WHEN** the cookie string lacks the requested CSRF key
- **THEN** the utility reports a configuration failure naming that key
- **AND** the caller does not send a request with an empty token

### Requirement: character-safe error truncation
The shared truncation utility SHALL truncate text by characters rather than bytes and SHALL append a truncation marker only when truncation actually occurred.

#### Scenario: Long Chinese body is truncated safely
- **WHEN** a response body containing multi-byte characters exceeds the limit
- **THEN** the result is a valid UTF-8 prefix of the original plus a truncation marker
- **AND** no panic occurs

#### Scenario: Short body is returned unchanged
- **WHEN** the body is shorter than the limit
- **THEN** the result equals the original body with no marker appended