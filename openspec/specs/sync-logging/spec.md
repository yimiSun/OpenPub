# sync-logging Specification

## Purpose
定义 OpenPub 同步日志的采集与呈现契约：把每一次同步的可观察结果结构化为日志条目，在桌面端以可扫描的时间线呈现，并支持导出排查。让用户能区分"哪个平台、哪个阶段、为什么失败"，而不是只看到一句笼统的报错。

## Requirements

### Requirement: structured sync log entries
Each platform sync attempt SHALL produce a structured log entry containing: timestamp, platform id, platform display name, lifecycle stage (`auth_check` | `preprocess` | `upload` | `sync` | `publish`), outcome (`running` | `success` | `failure` | `cancelled`), failure class when applicable, human-readable message, draft URL when successful, operation (`create` | `update`), and publication state.

#### Scenario: Successful sync logged with draft link
- **WHEN** a platform sync succeeds
- **THEN** a log entry records stage `sync`, outcome `success`, and the draft URL
- **AND** the entry is surfaced in the log panel with an actionable link

#### Scenario: Failed sync logged with failure class
- **WHEN** a platform sync fails
- **THEN** a log entry records outcome `failure` and the failure class
- **AND** the message distinguishes configuration problems from credential problems from network problems

#### Scenario: Update is distinguishable from create in the log
- **WHEN** a platform sync completes with `is_update = true`
- **THEN** the log entry records `operation: update`
- **AND** the log panel labels the entry as an update rather than a create

#### Scenario: Multi-platform batch logged independently
- **WHEN** the user publishes to several platforms in one action
- **THEN** one log entry is produced per platform
- **AND** a partially failed batch still records the successful platforms as successes

#### Scenario: Two-phase failure names the failing stage
- **WHEN** a platform performs draft creation and then live publication, and the publication action fails
- **THEN** the log entry records stage `publish`
- **AND** the message states that the draft was saved but publishing failed

#### Scenario: Draft-phase failure is distinguished from publish-phase failure
- **WHEN** a two-phase sync fails during draft creation
- **THEN** the log entry records stage `sync` rather than `publish`
- **AND** the user can tell from the stage whether a draft exists on the platform

### Requirement: sync log panel renders three boundary states
The sync log panel SHALL render distinct UI for three states: Empty (no sync attempted yet, with guidance on how to start), Running (a sync in progress, showing which platforms are pending), and Error (all attempts failed, with a retry action).

#### Scenario: Empty state before first sync
- **WHEN** the app starts and no sync has been attempted
- **THEN** the log panel shows an empty state with guidance text instead of a blank area

#### Scenario: Running state during batch sync
- **WHEN** a batch sync is in progress
- **THEN** the log panel indicates which platforms are still pending
- **AND** already-completed entries remain visible

#### Scenario: Error state when all attempts fail
- **WHEN** every platform in a batch fails
- **THEN** the log panel shows an error state offering a retry action

### Requirement: sync log export
The app SHALL allow the user to export the current sync log session to a local file as machine-readable text, preserving every recorded entry.

#### Scenario: User exports sync log
- **WHEN** the user triggers log export
- **THEN** a file is written containing every log entry of the current session
- **AND** the export path is reported to the user

#### Scenario: Export with no entries is refused
- **WHEN** the user triggers log export with an empty log
- **THEN** no file is written and the user is informed that there is nothing to export