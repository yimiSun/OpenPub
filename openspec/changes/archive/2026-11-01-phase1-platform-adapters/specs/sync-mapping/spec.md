# Spec Delta

## Purpose

定义 OpenPub 的文章同步映射：以文章稳定标识（UUID）为唯一键，记录该文章在每个平台上的远端文章 id 与是否已上线。这份映射是「再次同步时更新而非新建」的唯一依据，也是前端判断是否展示「同步并更新线上文章」文案的数据来源。

## ADDED Requirements

### Requirement: sync map keyed by article identifier
The sync map SHALL be keyed by the article's stable identifier, and SHALL resolve, for each article, a per-platform record of `{ id, published, bodyHash }`. The article identifier SHALL be the sole lookup key; the body hash SHALL be stored only as an auxiliary integrity signal and SHALL NOT be used to resolve a mapping on its own.

#### Scenario: Mapping lookup uses article identifier
- **WHEN** the app needs the sync mappings for a given article
- **THEN** it resolves them by that article's identifier
- **AND** it does not attempt to resolve them by content hash or title

#### Scenario: Mapping survives article edits
- **WHEN** an article's body is edited after it was synced
- **THEN** the mapping is still resolvable by the same article identifier
- **AND** the recorded body hash is updated to the new content hash

#### Scenario: Body hash mismatch is surfaced rather than silently ignored
- **WHEN** a mapping is resolved whose recorded body hash differs from the current article body hash
- **THEN** the system treats the article as modified since the last sync
- **AND** it does not discard the mapping or fall back to creating a new remote post

### Requirement: sync map storage
The system SHALL persist a sync map that resolves an article identifier to a per-platform record of `{ id, published, bodyHash }`, and SHALL expose read, write-single-entry, and delete-single-entry operations for it.

#### Scenario: Mapping persists across app restarts
- **WHEN** a mapping entry is written and the app is restarted
- **THEN** the entry is still readable with the same `id` and `published` values

#### Scenario: Mapping is scoped per platform
- **WHEN** an article is mapped for cnblogs and juejin independently
- **THEN** each platform record holds its own `id` and `published` values
- **AND** updating one platform's record leaves the other untouched

#### Scenario: Deleting an entry removes only that platform record
- **WHEN** a delete operation is issued for one platform under an article identifier
- **THEN** that platform's record is removed
- **AND** records for other platforms under the same article identifier remain

#### Scenario: Article deletion removes all its mappings
- **WHEN** an article is deleted from the workspace
- **THEN** every platform record under that article identifier is removed

### Requirement: published state recorded after every sync
After a successful create or update, the system SHALL write back the returned remote id, the publication state of that request, and the current body hash into the sync map for that article and platform.

#### Scenario: Successful create records id and draft state
- **WHEN** a create request completes with `publish_status = false`
- **THEN** the mapping stores the returned id with `published: false`

#### Scenario: Successful create records id and published state
- **WHEN** a create request completes with `publish_status = true`
- **THEN** the mapping stores the returned id with `published: true`

#### Scenario: Successful update preserves or upgrades published state
- **WHEN** an update request completes
- **THEN** the mapping's id stays equal to the updated post id
- **AND** the `published` value reflects the resulting state, so a draft-to-live update records `published: true`

#### Scenario: Failed sync does not overwrite an existing mapping
- **WHEN** a sync request fails
- **THEN** any pre-existing mapping record for that platform is left unchanged

### Requirement: update detection drives user-facing action
The frontend SHALL query the sync map for the current article before a sync is dispatched and SHALL present an update-oriented action when a mapping exists for that article and platform.

#### Scenario: Existing mapping switches action label
- **WHEN** the current article has a mapping record for a selected platform
- **THEN** the sync control for that platform presents an update-oriented label instead of a create-oriented label
- **AND** the dispatch carries `is_update: true` with the mapped id as `existing_id`

#### Scenario: No mapping triggers create path
- **WHEN** the current article has no mapping record for a selected platform
- **THEN** the sync is dispatched with `is_update: false`
- **AND** no `existing_id` is supplied

#### Scenario: Mapping is re-resolved after switching articles
- **WHEN** the user switches to a different current article
- **THEN** the update detection for each platform reflects that article's own mappings
- **AND** the previous article's mappings do not affect the new current article

### Requirement: irreversible overwrite requires explicit confirmation
Before dispatching any request with `publish_status = true`, the frontend SHALL display a confirmation dialog stating that the remote article will be overwritten irreversibly, and SHALL dispatch only after the user confirms.

#### Scenario: Overwrite confirmation shown before live request
- **WHEN** the user selects live publication (`publish_status = true`)
- **THEN** a confirmation dialog warns that the live article will be overwritten irreversibly
- **AND** no request is dispatched until the user confirms

#### Scenario: Cancelling confirmation aborts the request
- **WHEN** the user dismisses the overwrite confirmation without confirming
- **THEN** no request is dispatched
- **AND** the sync map and remote article remain unchanged

#### Scenario: Draft-only requests require no overwrite confirmation
- **WHEN** the user syncs with `publish_status = false`
- **THEN** no irreversible-overwrite confirmation is shown