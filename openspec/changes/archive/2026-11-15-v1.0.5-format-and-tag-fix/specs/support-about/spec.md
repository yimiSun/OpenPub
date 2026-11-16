# Spec Delta

## MODIFIED Requirements

> 目标能力 `support-about` 的基线定义位于尚未归档的 `openspec/changes/v1.0.0-omni-release/specs/support-about/spec.md`；本变更在其归档时合并，或与它一起合并进 `openspec/specs/support-about/spec.md`。

### Requirement: runtime environment is displayed read-only
弹窗底部 SHALL 只读展示 App 版本、操作系统版本与运行引擎三项信息，其中 App 版本与操作系统版本从 Tauri 宿主 API 获取，运行引擎固定显示为 `Tauri v2 + Rust`；三项信息均不可编辑。操作系统版本 SHALL 依据 Build 号正确区分 Windows 11：宿主 API 报告的 Build 号大于等于 22000 时 SHALL 显示为 Windows 11（不使用底层 API 兼容性谎报的 `10.0` 主版本号），并 SHALL 尽可能附带系统版本名（如专业版）；无法识别版本名时 SHALL 显示不含版本名的 Windows 11。

#### Scenario: Environment info reflects actual host
- **WHEN** 弹窗打开
- **THEN** App 版本显示为当前应用包的实际版本号
- **AND** 操作系统版本显示为宿主实际系统版本

#### Scenario: Windows 11 is identified by build number
- **WHEN** 宿主为 Windows 11 且底层 API 报告 `10.0` 与 Build 号大于等于 22000
- **THEN** 操作系统版本显示 Windows 11 而非 Windows 10

#### Scenario: Windows edition name is included when identifiable
- **WHEN** 系统能通过产品类型 API 识别出版本名（如专业版）
- **THEN** 操作系统版本附带该版本名（形如 `Windows 11 专业版 (Build 26300)`）

#### Scenario: Unidentifiable edition degrades gracefully
- **WHEN** 产品类型无法映射为已知版本名或版本 API 调用失败
- **THEN** 操作系统版本退回为不含版本名的形态（形如 `Windows 11 (Build 26300)`），且永远不为空

#### Scenario: Environment info is read-only
- **WHEN** 用户在弹窗内与运行环境区域交互
- **THEN** 不出现任何可编辑输入控件