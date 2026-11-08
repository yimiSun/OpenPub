# Spec Delta

## MODIFIED Requirements

### Requirement: DPAPI encrypted storage with no plaintext on disk
Extracted cookies SHALL be immediately encrypted via Windows DPAPI and stored in `tauri-plugin-store`. The on-disk store file SHALL NEVER contain plaintext cookie values. All cookie values SHALL be DPAPI-encrypted before being written and decrypted only when read at runtime.

#### Scenario: Cookie stored as ciphertext
- **WHEN** a cookie is saved to the store
- **THEN** the store file on disk contains only DPAPI-encrypted ciphertext for that cookie value

#### Scenario: No plaintext cookie in any store file
- **WHEN** the store file is inspected with a text editor or hex viewer
- **THEN** no plaintext cookie value is findable in the file content

#### Scenario: Legacy plaintext entry is not treated as valid
- **WHEN** a stored credential entry cannot be decrypted because it was written in an earlier plaintext format
- **THEN** the entry is reported as requiring re-authentication
- **AND** the app never falls back to using the raw stored value as a credential

### Requirement: one-click clear all login data
The settings page SHALL provide a "Clear All Login Data" action that deletes all stored credentials from `tauri-plugin-store`. This action SHALL require user confirmation before execution and SHALL irreversibly remove all encrypted credential entries.

#### Scenario: User clears all login data
- **WHEN** the user clicks "Clear All Login Data" and confirms the dialog
- **THEN** all credential entries are deleted from the store
- **AND** all platform status indicators change to "Not Logged In"

#### Scenario: Clear action requires confirmation
- **WHEN** the user clicks "Clear All Login Data"
- **THEN** a confirmation dialog is shown before any data is deleted

#### Scenario: Clearing credentials makes publish report not_configured
- **WHEN** a platform's credentials have been cleared and a publish is attempted
- **THEN** the publish result reports the credential as not configured
- **AND** no request is issued using a stale credential