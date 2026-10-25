# Spec Delta

## Purpose

提供 OpenPub 共享的 React UI 组件库，基于 Tailwind CSS，为 Tauri 桌面 App 和未来的其他端提供一致的暗黑模式优先极客风界面。

## ADDED Requirements

### Requirement: React component library scaffold
The `packages/ui` package SHALL contain a React component library with at minimum: `Button`, `Input`, `Select`, `TagInput`, `StatusBadge`, and `Card` components, all styled with Tailwind CSS.

#### Scenario: Components render with Tailwind classes
- **WHEN** a UI component is rendered in the Tauri Webview
- **THEN** Tailwind CSS utility classes are applied and the component displays correctly

### Requirement: dark mode as default with Tailwind dark variant
The UI components SHALL use Tailwind's `dark:` variant for all color and styling decisions, and the default theme SHALL be dark mode. No component SHALL use hardcoded light-mode-only colors.

#### Scenario: Default rendering is dark mode
- **WHEN** a UI component is rendered without explicit theme override
- **THEN** the component displays with dark mode colors via `dark:` variant classes

#### Scenario: All color tokens use dark variant
- **WHEN** a component's source code is inspected
- **THEN** every color-related class includes a `dark:` variant counterpart

### Requirement: data display components provide Loading Empty Error states
Any component that displays data from an async source (lists, tables, detail views, status panels) SHALL render distinct UI for three boundary states: Loading (skeleton or spinner), Empty (illustration and guidance text), and Error (error message with retry action). These states SHALL NOT be omitted or replaced with a single "happy path" only.

#### Scenario: List component shows loading state
- **WHEN** a data list component is rendered while data is being fetched
- **THEN** a Loading state (skeleton or spinner) is displayed

#### Scenario: List component shows empty state
- **WHEN** a data list component receives an empty array
- **THEN** an Empty state with illustration and guidance text is displayed

#### Scenario: List component shows error state
- **WHEN** a data list component receives an error from the data source
- **THEN** an Error state with error message and retry action is displayed

### Requirement: package exports for Tauri desktop consumption
The `packages/ui` package SHALL be buildable as a library (not an application) and export all components from a single entry point, consumable by `apps/desktop-app` and future workspace apps.

#### Scenario: Tauri desktop app imports UI components
- **WHEN** the Tauri desktop app's React frontend imports from `@openpub/ui`
- **THEN** all shared components are accessible and render correctly in the Tauri Webview context