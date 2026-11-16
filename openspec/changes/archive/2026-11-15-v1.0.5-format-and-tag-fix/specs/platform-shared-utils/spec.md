# Spec Delta

## MODIFIED Requirements

### Requirement: markdown to html conversion
The shared markdown utility SHALL convert Markdown source into an HTML fragment by parsing it as GitHub Flavored Markdown, enabling table, strikethrough, autolink and tasklist extensions, passing raw HTML tags through unchanged, and rendering single line breaks as hard breaks. It SHALL produce a single empty paragraph when the source has no non-empty content.

#### Scenario: Fenced code block is rendered as code markup
- **WHEN** the Markdown source contains a fenced block tagged `rust` holding `fn main() {}`
- **THEN** the resulting HTML contains a `<pre><code class="language-rust">` element wrapping the source text
- **AND** the fence markers themselves never appear in the output

#### Scenario: Lists are rendered as list markup
- **WHEN** the Markdown source contains `- a` and `- b` on consecutive lines
- **THEN** the resulting HTML contains a `<ul>` element with two `<li>` children

#### Scenario: Tables are rendered as table markup
- **WHEN** the Markdown source contains a GFM table
- **THEN** the resulting HTML contains `<table>`, `<thead>` and `<td>` elements

#### Scenario: Special characters are escaped
- **WHEN** the Markdown source contains `&`, `<`, or `>` outside of a raw HTML tag
- **THEN** the resulting HTML contains their entity forms instead of raw characters

#### Scenario: Raw HTML tags pass through unchanged
- **WHEN** the Markdown source contains a literal `<div>` element
- **THEN** the resulting HTML keeps that element as markup rather than escaped text

#### Scenario: Each non-empty line becomes a paragraph
- **WHEN** the Markdown source has three non-empty lines separated by blank lines
- **THEN** the resulting HTML contains three paragraph elements in source order

#### Scenario: Single line break becomes a hard break
- **WHEN** the Markdown source has two non-empty lines separated by a single line break
- **THEN** the resulting HTML contains a `<br />` between them
- **AND** the two lines are never merged into a single run of text

#### Scenario: Empty source yields one empty paragraph
- **WHEN** the Markdown source is empty or contains only whitespace
- **THEN** the resulting HTML is a single empty paragraph
- **AND** it is never an empty string

#### Scenario: Conversion is identical across platforms
- **WHEN** two adapters convert the same Markdown source with the generic converter
- **THEN** they produce byte-identical HTML
- **AND** no adapter carries its own private conversion implementation
- **AND** the only permitted variation is the Zhihu-specific downgrade, which lives in the shared utility rather than in an adapter

#### Scenario: Platform sanitizers operate on generated HTML
- **WHEN** a platform needs to remove or rewrite elements (e.g. WeChat dropping images)
- **THEN** the sanitizer operates on the HTML produced by the shared converter
- **AND** it does not re-implement Markdown parsing

## ADDED Requirements

### Requirement: zhihu html downgrade
The shared markdown utility SHALL provide `to_zhihu_html_content`, a Zhihu-specific view derived from the generic converter output without modifying it, because the Draft.js editor discards structural markup it cannot parse and leaves only scattered paragraphs. It SHALL downgrade code blocks to blockquotes, flatten tables to plain text rows, and keep only tags the editor parses reliably, stripping unknown tags while keeping their text.

#### Scenario: Code block becomes a blockquote
- **WHEN** the Markdown source contains a fenced code block
- **THEN** the Zhihu output wraps the code text in `<blockquote>`
- **AND** no `<pre>` or `<code>` tag remains
- **AND** the original line breaks are preserved with `<br />`

#### Scenario: Table becomes plain text rows
- **WHEN** the Markdown source contains a GFM table
- **THEN** the Zhihu output contains one line per non-empty row with cells joined by ` | ` and rows joined by `<br />`
- **AND** no `<table>`, `<tr>`, `<td>` or `<thead>` tag remains

#### Scenario: Only compatible tags survive
- **WHEN** the Markdown source contains headings, bold, italic, lists, quotes and links
- **THEN** the Zhihu output keeps only `p`, `br`, `strong`, `b`, `em`, `i`, `a`, `h1`-`h6`, `ul`, `ol`, `li`, `blockquote`, `code` and `hr`
- **AND** the anchor keeps its `href` and loses every other attribute

#### Scenario: Unsupported tags are stripped but text is kept
- **WHEN** the Markdown source contains raw `<div>` or `<span>` elements
- **THEN** the tags are removed from the Zhihu output
- **AND** their inner text remains

#### Scenario: Images are dropped
- **WHEN** the Markdown source contains an image
- **THEN** no `<img>` tag appears in the Zhihu output

#### Scenario: Generic conversion is unaffected
- **WHEN** any other platform converts the same Markdown source
- **THEN** its output is identical to before the Zhihu downgrade was introduced