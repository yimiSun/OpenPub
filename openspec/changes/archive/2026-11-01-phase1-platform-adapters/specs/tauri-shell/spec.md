# Spec Delta

## MODIFIED Requirements

### Requirement: async commands for long-running operations
Any Rust command that performs I/O (network requests, file reads/writes, image processing) or may take longer than 50ms SHALL be declared as `async` Tauri commands to avoid blocking the main thread. Platform publish commands perform network I/O and SHALL therefore be `async`.

#### Scenario: Network request does not freeze UI
- **WHEN** a Rust command performs an HTTP request
- **THEN** the command is declared `async` and the UI remains responsive during the request

#### Scenario: Platform publish command is async
- **WHEN** a platform publish command is invoked from the frontend
- **THEN** the command is declared `async` so the main thread stays responsive while the platform request is in flight

### Requirement: encrypted credential storage via tauri-plugin-store with DPAPI
The app SHALL use `tauri-plugin-store` for persisting platform credentials (cookies/tokens). On Windows, the store SHALL be configured to use DPAPI encryption so that credentials are never stored in plaintext on disk. The app SHALL expose dedicated read and clear commands so that credentials can be retrieved and removed without any frontend access to the ciphertext.

#### Scenario: Cookie stored with DPAPI encryption
- **WHEN** a platform cookie is saved via the store
- **THEN** the on-disk store file contains DPAPI-encrypted ciphertext, not the plaintext cookie value

#### Scenario: Cookie retrieved after DPAPI decryption
- **WHEN** a stored cookie is read from the store
- **THEN** DPAPI decrypts the ciphertext and the original cookie value is returned to the caller

#### Scenario: Different Windows user cannot decrypt
- **WHEN** the store file is accessed under a different Windows user account
- **THEN** DPAPI decryption fails and the credential is inaccessible

#### Scenario: Credential read and clear via dedicated commands
- **WHEN** the frontend needs a stored credential or wants to remove one
- **THEN** it calls the dedicated load and clear commands
- **AND** the frontend never receives or supplies raw ciphertext