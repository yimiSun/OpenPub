# Spec Delta

## ADDED Requirements

### Requirement: two-phase partial success records the draft mapping
When a platform completes a two-phase sync in which the draft was created or updated successfully but the live publication action failed, the system SHALL still write the draft mapping for that article and platform with `published: false`, and SHALL NOT treat the overall failure as a reason to skip the mapping write.

#### Scenario: Draft created then publish fails writes draft mapping
- **WHEN** a two-phase sync reports overall failure but carries a usable created draft id
- **THEN** the mapping stores that draft id with `published: false`
- **AND** the body hash is recorded so a later sync can detect content changes

#### Scenario: Next sync continues from the recorded draft
- **WHEN** the user syncs the same article to the same platform again after a partial success
- **THEN** the sync is dispatched as an update against the recorded draft id
- **AND** no second draft is created for that article

#### Scenario: Failure without a usable id writes nothing
- **WHEN** a two-phase sync fails before the draft exists, or the created id is a placeholder
- **THEN** no mapping record is written for that platform
- **AND** the stored map remains free of unusable identifiers

#### Scenario: Mapping is written once per platform in a batch
- **WHEN** a batch sync produces a partial success on one platform and full success on another
- **THEN** each platform's mapping reflects its own outcome
- **AND** the partial platform records `published: false` while the full platform records the actual publication state