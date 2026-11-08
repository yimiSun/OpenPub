# Spec Delta

## Purpose

定义 OpenPub 的本地文章工作区：以稳定 UUID 标识每篇文章，提供文章创建、列表、编辑与删除，并持久化保存，使文章身份在切换文章、重启应用后依然可追溯。这是同步映射能够准确定位「同一篇文章」的前提。

## ADDED Requirements

### Requirement: stable article identity
Each local article SHALL be assigned a locally generated stable identifier (UUID) at creation time. That identifier SHALL remain unchanged across edits, application restarts, and article switching, and SHALL be the sole key used to look up the article's sync mappings.

#### Scenario: Identifier is generated at creation
- **WHEN** the user creates a new article
- **THEN** the article is persisted with a newly generated unique identifier
- **AND** the identifier is not derived from the article title or content

#### Scenario: Identifier survives content edits
- **WHEN** the user edits the title or body of an existing article
- **THEN** the article's identifier remains unchanged
- **AND** the article's sync mappings are still resolvable by that identifier

#### Scenario: Identifier survives restart
- **WHEN** the app is restarted after articles were created
- **THEN** every article is reloaded with its original identifier
- **AND** sync mappings keyed by those identifiers remain resolvable

### Requirement: article list and current article
The workspace SHALL present a list of all local articles and SHALL maintain exactly one current article at a time. Selecting an article in the list SHALL make it current and SHALL load its content into the editor.

#### Scenario: Article list shows all articles
- **WHEN** the workspace has one or more articles
- **THEN** the list shows each article distinguished by its title
- **AND** a title that is empty is displayed with an untitled fallback label

#### Scenario: Selecting an article loads its content
- **WHEN** the user selects an article from the list
- **THEN** the editor shows that article's title and body
- **AND** the current article identity switches to that article

#### Scenario: Switching articles preserves unsaved edits of the previous article
- **WHEN** the user edits an article and then selects a different article
- **THEN** the edits of the previously current article are persisted before the switch
- **AND** returning to that article shows the persisted edits

#### Scenario: Empty workspace guides the user to create one
- **WHEN** the workspace contains no articles
- **THEN** the list area shows guidance for creating the first article
- **AND** no current article exists until one is created

### Requirement: article create edit and delete
The workspace SHALL support creating a new article, editing an existing article's title and body, and deleting an article. Deletion SHALL require user confirmation.

#### Scenario: Creating an article adds it to the list and makes it current
- **WHEN** the user creates a new article
- **THEN** the article appears in the list
- **AND** it becomes the current article with an empty title and body

#### Scenario: Editing persists the latest content
- **WHEN** the user modifies the current article's title or body
- **THEN** the persisted article reflects the latest content on the next read

#### Scenario: Deleting an article removes it from the list
- **WHEN** the user confirms deletion of an article
- **THEN** the article no longer appears in the list
- **AND** its sync mappings are removed along with it

#### Scenario: Delete requires confirmation
- **WHEN** the user requests deletion of an article
- **THEN** a confirmation is required before the article is removed

### Requirement: article persistence across restarts
Articles SHALL be persisted to local storage so that all articles, their content, and their identifiers survive an application restart.

#### Scenario: Articles restored on restart
- **WHEN** the app is restarted after several articles were created and edited
- **THEN** all of those articles are present in the list
- **AND** each article's title and body match the values before restart