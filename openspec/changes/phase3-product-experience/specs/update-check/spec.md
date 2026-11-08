# Spec Delta

## Purpose

为桌面应用提供轻量的版本升级检测能力，使设置页能够查询远端发布的最新版本、与本地版本比对，并在发现新版本时引导用户前往 Gitee 获取安装包。

## ADDED Requirements

### Requirement: Check for updates compares remote version with app version
设置页 SHALL 提供「检查更新」按钮，请求远端版本信息 JSON 并与当前应用版本按语义化版本规则比对，返回三种结果之一：已是最新、发现新版本、检查失败。

#### Scenario: Remote version is newer
- **WHEN** 用户点击「检查更新」且远端版本号高于当前应用版本
- **THEN** 界面显示发现新版本，并展示远端版本号与当前版本号

#### Scenario: Remote version equals current
- **WHEN** 用户点击「检查更新」且远端版本号等于当前应用版本
- **THEN** 界面显示已是最新版本

#### Scenario: Version comparison follows semver segments
- **WHEN** 当前版本为 `0.1.0` 且远端版本为 `0.1.10`
- **THEN** 判定为发现新版本，而非按字符串比较判为更小

#### Scenario: Check is in progress
- **WHEN** 用户点击「检查更新」且请求尚未返回
- **THEN** 按钮进入加载状态并禁止重复点击

### Requirement: Newer version offers a Gitee download link
当检查结果为发现新版本时，界面 SHALL 提供前往 Gitee 下载的按钮，点击后通过系统默认浏览器打开项目发布页，不在应用内执行自动下载或自动安装。

#### Scenario: Download button opens browser
- **WHEN** 用户在发现新版本的提示中点击前往下载
- **THEN** 系统默认浏览器打开 Gitee 发布页
- **AND** 应用不触发任何自动更新安装流程

#### Scenario: No download prompt when already latest
- **WHEN** 检查结果为已是最新版本
- **THEN** 界面不展示前往下载的按钮

### Requirement: Update check failures are reported without blocking the app
当远端版本信息无法获取、响应不是合法 JSON 或缺少版本字段时，检查 SHALL 以失败结束并在界面显示失败原因，不得阻塞其他设置功能的使用。

#### Scenario: Network unavailable
- **WHEN** 用户点击「检查更新」且网络不可达
- **THEN** 界面显示检查失败与网络相关的原因
- **AND** 设置页其他按钮仍可正常使用

#### Scenario: Malformed remote payload
- **WHEN** 远端响应不是合法 JSON 或缺少版本字段
- **THEN** 界面显示检查失败并说明响应格式异常
- **AND** 不把缺失版本字段的响应判定为已是最新