# Tasks

## 1. 平台注册表单一来源

- [x] 1.1 在 `packages/core/src/platforms/registry.ts` 定义 `PlatformDefinition` 类型与 `PLATFORM_REGISTRY` 常量，条目覆盖 cnblogs、juejin、csdn 及既有占位平台，含 `id` / `name` / `credentialKind` / `loginUrl` / `available`；验证 `pnpm --filter @openpub/core typecheck` 通过
- [x] 1.2 在 `packages/core/src/index.ts` 导出注册表与类型；验证 `apps/desktop-app` 可从 `@openpub/core` 导入且类型正确
- [x] 1.3 修改 `App.tsx` 删除本地 `PLATFORMS` 常量，改为从注册表派生渲染与 `WEBVIEW_URLS` 来源；验证 `pnpm --filter desktop-app typecheck` 通过，且平台面板显示条目与注册表一致

## 2. Rust 侧平台模块拆分与失败分类

- [x] 2.1 新建 `src-tauri/src/platforms/mod.rs`，定义 `PlatformError { class: FailureClass, message: String }` 与 `FailureClass` 枚举（`serde(rename_all = "snake_case")`，六类变体）；验证 `cargo check` 通过
- [x] 2.2 将 `publish_cnblogs` 原样迁入 `src-tauri/src/platforms/cnblogs.rs`，保留原有 MetaWeblog 常量与请求体；验证 `cargo check` 通过，比对请求 URL 与参数名无变化
- [x] 2.3 在 cnblogs 适配器入口加入三字段非空检查，缺失时返回 `not_configured` 并指名缺失字段；XML-RPC fault 归类为 `platform_error` 并把 fault 文本放进 message；验证手工提交空缺配置时提示为"未配置"且未发出网络请求
- [x] 2.4 将 `publish_juejin` 原样迁入 `src-tauri/src/platforms/juejin.rs`，保留原 user agent / referer / origin 常量；验证 `cargo check` 通过
- [x] 2.5 在 juejin 适配器入口加入 Cookie 非空检查（`not_configured`），登录态/权限错误码归类为 `auth_invalid`，网络错误归类为 `network_error`；验证手工在无 Cookie 状态下发布时提示为"未配置"
- [x] 2.6 扩展 `PublishResult` 增加 `operation` / `isPublished` / `failure_class` 字段并同步所有构造点；`platform_publish` 改为委托 `platforms::dispatch`，未实现平台与未知 id 分别返回 `not_implemented` 与 unknown platform 错误；验证 `cargo check` 通过且未知 id 的错误文本包含该 id
- [x] 2.7 为 `FailureClass` 与 `truncate_chars` 编写 Rust 单元测试（含中文超长字符串截断不 panic）；验证 `cargo test` 通过

## 3. 文章工作区（稳定文章标识）

- [x] 3.1 新建 `src-tauri/src/articles.rs`，定义 `Article { id, title, content, createdAt, updatedAt }`；`create_article` 在 Rust 侧生成 UUID 作为 `id`，不依赖标题或正文；验证 `cargo check` 通过
- [x] 3.2 在 store 中实现 `articles:<uuid>` 的单条读写与 `articles:index` 列表索引，保证重启后可按 id 读回全部文章；验证写入后重启应用仍能列出全部文章
- [x] 3.3 新增 Tauri 命令 `list_articles` / `create_article` / `update_article` / `delete_article`；`delete_article` 同步清理该 UUID 的全部 `syncmap:<uuid>` 记录；验证删除后 `load_sync_map` 查询该文章返回空
- [x] 3.4 单元测试断言文章标识的稳定性：修改标题与正文后 id 不变、重启后 id 不变、两个标题相同的文章 id 不同；验证 `cargo test` 通过
- [x] 3.5 该 UUID 不依赖外部 crate 也可实现（若引入 `uuid` 需在 Cargo.toml 声明并验证离线构建可行）；验证 `cargo check` 通过

## 4. 文章映射与发布状态存储

- [x] 4.1 实现 `body_hash(content)`，先规范化（统一 CRLF 为 LF、裁剪首尾空白）再算 SHA-256 十六进制小写，仅作为辅助校验字段；单元测试断言"同内容两次哈希相同"、"仅空白差异哈希相同"、"正文增删后哈希不同"；验证 `cargo test` 通过
- [x] 4.2 在 store 中实现 `syncmap:<articleUuid>` 的读写：值为 `{ "<platformId>": { "id": string, "published": bool, "bodyHash": string } }`；验证写入后重启应用仍可读到相同记录
- [x] 4.3 新增 Tauri 命令 `load_sync_map`（按文章 id 查询）、`save_sync_entry`、`clear_sync_entry`；验证三个命令均可被前端 invoke 成功，且 `clear_sync_entry` 只删除指定平台记录
- [x] 4.4 断言 `syncmap:` 键与 `*_config` 键互不影响：单元测试验证 `clear_cookie` 执行后 `syncmap:` 记录仍在；验证 `cargo test` 通过
- [x] 4.5 在 `packages/core/src/platforms` 新增 `SyncMapRecord` / `SyncEntry` 类型与前端查询辅助函数（按文章 id + 平台取记录、判断是否存在映射）；验证 `pnpm --filter @openpub/core typecheck` 通过

## 5. 更新路径与动态发布布尔值

- [x] 5.1 扩展 `platform_publish` 入参为 `article_id` / `existing_id: Option<String>` / `is_update: bool` / `publish_status: bool`，`publish_status` 在 Rust 侧无任何默认为 true 的路径；验证 `cargo check` 通过并逐行审阅确认无 `true` 兜底
- [x] 5.2 博客园适配器改造：`is_update=true` 时调用 `metaWeblog.editPost`（携带 `existing_id`），否则 `metaWeblog.newPost`；publish 布尔值由 `publish_status` 直接插值，删除写死的 `0`；单元测试双重断言：`publish_status=false` 报文含 `<boolean>0</boolean>`、`true` 时含 `<boolean>1</boolean>`，且入参缺失时不得出现 `1`
- [x] 5.3 博客园更新路径前置校验：`is_update=true` 但 `existing_id` 为空时返回 `not_configured` 且不发请求；验证手工触发该分支的提示文案
- [x] 5.4 掘金适配器改造：未发布目标走 `article_draft/update`（携带 `draft_id`），已发布目标走 `article/update`（携带 `article_id`）；`is_update=false + publish_status=true` 返回 `not_implemented` 且 message 为「掘金目前仅支持保存草稿，请到掘金后台手动发布」；验证 `cargo check` 通过，且代码中不存在 create 作为 update 失败后的 fallback
- [x] 5.5 抓包验证掘金更新接口：在浏览器中分别修改草稿与已发布文章，记录真实请求 URL、方法、关键字段名；若与 `article_draft/update` / `article/update` 假定不符，按结果修正代码并同步更新 design.md Decision 9；验证修正后的更新请求返回 `err_no: 0`
- [x] 5.6 更新成功后回写映射：`save_sync_entry` 写入返回 id、`published` 状态与当前 `bodyHash`；更新失败时不得覆盖既有记录；单元测试覆盖"草稿更新变已发布则 published 置 true"；验证 `cargo test` 通过

## 6. 前端文章工作区与同步交互

- [x] 6.1 左侧编辑器重构为「文章列表 + 当前文章编辑区」：列表按标题区分、空标题显示"未命名文章"、空工作区显示创建引导；保留原有 Markdown 输入控件；验证 `pnpm --filter desktop-app typecheck` 通过
- [x] 6.2 接入文章命令：启动时 `list_articles` 恢复列表并按持久化的当前文章 id 恢复选区；新建、切换（切换前落盘当前改动）、编辑、删除（二次确认）全部走 Rust 命令；验证重启后文章列表与当前文章一致
- [x] 6.3 同步前按当前文章 id 查询映射：存在映射时该平台按钮文案变为"同步并更新线上文章"，invoke 携带 `is_update: true` 与 `existing_id`；无映射时保持新建文案且 `is_update: false`；验证切换文章后判定随之改变
- [x] 6.4 新增"覆盖已发布的文章"复选框，默认不勾选；勾选后 `publish_status = true`；验证默认态与勾选态下 invoke 入参分别正确
- [x] 6.5 实现不可逆覆盖确认框：`publish_status = true` 时强制弹窗提示"这将会直接覆盖线上文章，且不可逆，是否继续？"，用户确认后才 invoke；取消则不发起任何请求；验证两条分支（确认/取消）均目视确认，且取消后日志无新增条目
- [x] 6.6 掘金提交前提示：对掘金勾选"直接发布"时，在提交前提示"掘金目前仅支持保存草稿，请到后台手动发布"，不静默降级也不发请求；验证目视确认提示出现且无网络请求
- [x] 6.7 日志条目补充 `operation`（create/update）与 `isPublished` 字段并在 UI 显示；验证更新操作在日志中被明确标注为"更新"

## 7. 同步日志能力

- [x] 7.1 在 `packages/core/src/platforms` 旁新增日志条目类型 `SyncLogEntry`（`timestamp` / `articleId` / `platformId` / `platformName` / `stage` / `outcome` / `failureClass?` / `message` / `draftUrl?` / `operation?` / `isPublished?`）；验证 `pnpm --filter @openpub/core typecheck` 通过
- [x] 7.2 `App.tsx` 的 `LogEntry` 替换为 `SyncLogEntry`，批量发布时为每个平台独立生成条目并携带阶段；验证多平台发布后日志条目数等于平台数
- [x] 7.3 日志面板按 Empty / Running / Error 三态渲染（空态含引导文案、Running 显示待完成平台、全失败显示重试动作）；验证三种场景分别目视确认
- [x] 7.4 新增 Rust 命令 `export_sync_log`，接收 JSONL 字符串写入 `appConfigDir/logs/` 并返回路径；空日志时前端不调用并提示无内容；验证导出后文件存在且内容包含全部条目

## 8. 凭据命令与 CSDN 预留

- [x] 8.1 确认 `load_cookie` 在 DPAPI 解密失败时返回可区分的错误而非回退原值，前端据此提示"需重新登录"；验证手工写入一条非 DPAPI 格式数据后读取得到该提示
- [x] 8.2 `clear_cookie` 执行后前端平台状态刷新为未登录，且随后发布返回 `not_configured`；验证清除后立即发布观察结果
- [x] 8.3 注册表加入 `csdn`（`available: false`），`platforms::dispatch` 对其返回 `not_implemented`；前端将该条目置灰并显示"即将支持"；验证点击 CSDN 发布时不发出网络请求且提示符合预期

## 9. 集成验证

- [x] 9.1 增加一致性测试，断言 TS 注册表 id 集合与 Rust `dispatch` 覆盖的 id 集合相同；验证测试在故意删掉一个 Rust 分支时失败
- [x] 9.2 用真实博客园配置完整跑一次新建草稿，确认草稿链接可打开、日志条目阶段与结果正确；验证目视确认草稿页可访问
- [x] 9.3 修改该文章正文后再次同步，确认走 `editPost` 且平台上未产生第二篇文章；验证目视确认文章列表数量不变
- [x] 9.4 用真实掘金 Cookie 完整跑一次新建草稿，确认草稿链接可打开；再对同一篇再次同步，确认走草稿更新接口且草稿数不变；验证目视确认草稿列表数量不变
- [x] 9.5 验证 `publish_status = true` 全链路：勾选后必须弹确认框，取消时不发请求，确认后线上文章内容被覆盖且映射记录 `published` 变为 true；验证目视确认线上文章内容已更新
- [x] 9.6 验证重启恢复：新建两篇文章并分别只同步其中一个平台，重启后确认文章列表、当前文章与各平台映射均正确恢复
- [x] 9.7 在 `README` 或现有文档中补充"文章列表、三态日志、失败分类、覆盖已发布文章、掘金仅草稿"的用户侧说明；验证文档描述与 UI 实际文案一致

## Workflow follow-up

- 在实现完成并经审查后归档该变更。
- 启动下一次变更 `phase1-csdn-adapter`，实现 CSDN 草稿创建适配器。