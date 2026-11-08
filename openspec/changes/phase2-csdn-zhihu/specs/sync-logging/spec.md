# Spec Delta

## MODIFIED Requirements

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