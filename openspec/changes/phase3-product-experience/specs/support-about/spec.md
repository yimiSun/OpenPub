# Spec Delta

## Purpose

为应用提供统一的「关于与支持」展示入口，承载开源宣发、开发者赞助渠道、可关闭的推荐推广位以及只读运行环境信息，让用户能够了解项目定位、支持作者并复制推荐链接。

## ADDED Requirements

### Requirement: About modal is reachable from the header
应用 SHALL 在顶部导航栏提供「关于/支持」入口，点击后以暗黑风格模态弹窗展示内容，弹窗可通过右上角关闭按钮、`Esc` 键或点击遮罩关闭。

#### Scenario: Open about modal from header
- **WHEN** 用户点击顶部导航栏的「关于/支持」按钮
- **THEN** 关于弹窗打开并覆盖在应用界面之上
- **AND** 背景内容被遮罩覆盖且不可交互

#### Scenario: Close about modal by any close path
- **WHEN** 用户点击关闭按钮、按下 `Esc` 键或点击遮罩区域
- **THEN** 弹窗关闭，应用界面恢复可交互

### Requirement: About modal presents project vision and repository entry
弹窗 SHALL 展示项目愿景文案与开源仓库入口，入口点击后通过系统默认浏览器打开 `https://gitee.com/tianyeHome/OpenPub`，不在应用内导航。

#### Scenario: Repository button opens external browser
- **WHEN** 用户点击「打开 Gitee」
- **THEN** 系统默认浏览器被唤起并打开 `https://gitee.com/tianyeHome/OpenPub`
- **AND** 应用窗口本身不发生页面跳转

#### Scenario: Vision text is visible without scrolling into a hidden area
- **WHEN** 弹窗打开时窗口高度较小
- **THEN** 弹窗内容区可滚动，愿景文案与仓库入口均可到达

### Requirement: About modal shows donation QR placeholders
弹窗 SHALL 并排展示支付宝与微信打赏二维码，图片来源为 `public/alipay.png` 与 `public/wechat.png`；当图片缺失或加载失败时 SHALL 展示明确占位提示而非空白区域。

#### Scenario: Both QR codes render side by side
- **WHEN** 弹窗打开且两张图片均可加载
- **THEN** 支付宝与微信二维码在同一行并排显示，各带说明文字

#### Scenario: Missing image falls back to placeholder
- **WHEN** `alipay.png` 或 `wechat.png` 不存在或加载失败
- **THEN** 该位置显示占位提示文案
- **AND** 弹窗其余内容正常展示且不报错

### Requirement: Promo section can be dismissed permanently
弹窗内 SHALL 提供「极客推荐」推广区，包含智谱 GLM 与阿里云服务器两个条目，并提供「不再显示」复选框；勾选后该推广区 SHALL 从后续打开中隐藏，该状态通过 `localStorage` 持久化并在下次启动时生效。

#### Scenario: Dismiss hides promo immediately
- **WHEN** 用户勾选「不再显示」
- **THEN** 推广区立即从弹窗中消失
- **AND** 无需关闭弹窗即可看到结果

#### Scenario: Dismissal survives restart
- **WHEN** 用户勾选「不再显示」后关闭并重新启动应用，再次打开关于弹窗
- **THEN** 推广区不再显示
- **AND** 取消勾选后推广区恢复显示

### Requirement: Copy link action writes to clipboard and shows a toast
推广区的「复制链接」按钮 SHALL 通过 Tauri 剪贴板 API 把对应链接写入系统剪贴板，并在写入成功后展示「已复制到剪贴板」的 Toast 提示，提示在数秒后自动消失。

#### Scenario: Copy succeeds and shows toast
- **WHEN** 用户点击某个推广条目的「复制链接」
- **THEN** 该条目的完整链接被写入系统剪贴板
- **AND** 界面出现「已复制到剪贴板」提示并在数秒后自动消失

#### Scenario: Copy failure is reported
- **WHEN** 剪贴板写入失败
- **THEN** 界面显示复制失败的提示
- **AND** 不出现「已复制到剪贴板」的成功提示

#### Scenario: Browser open uses the same link
- **WHEN** 用户点击同一推广条目的「浏览器打开」
- **THEN** 系统默认浏览器打开与「复制链接」完全相同的链接

### Requirement: Runtime environment is displayed read-only
弹窗底部 SHALL 只读展示 App 版本、操作系统版本与运行引擎三项信息，其中 App 版本与操作系统版本从 Tauri 宿主 API 获取，运行引擎固定显示为 `Tauri v2 + Rust`；三项信息均不可编辑。

#### Scenario: Environment info reflects actual host
- **WHEN** 弹窗打开
- **THEN** App 版本显示为当前应用包的实际版本号
- **AND** 操作系统版本显示为宿主实际系统版本

#### Scenario: Environment info is read-only
- **WHEN** 用户在弹窗内与运行环境区域交互
- **THEN** 不出现任何可编辑输入控件