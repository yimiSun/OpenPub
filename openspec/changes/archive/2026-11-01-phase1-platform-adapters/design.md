# Design

## Context

Phase 0 归档后，两条真实发布链路（博客园 MetaWeblog XML-RPC、掘金 REST 草稿）与 DPAPI 加密已在实际代码中跑通，但它们的形态是"内联在 `lib.rs` 的命令集合"：

- `platform_publish` 用一个大的 `match platform.as_str()` 分派，未实现平台返回纯字符串错误（见 [lib.rs:483-497](file:///f:/快发布/apps/desktop-app/src-tauri/src/lib.rs#L483-L497)）
- 前端 `PLATFORMS` 是手工维护的常量数组，含 `configType` 字面量（见 [App.tsx:7-16](file:///f:/快发布/apps/desktop-app/src/App.tsx#L7-L16)）
- `PublishResult` 只有 `success` / `post_id` / `edit_url` / `message`，没有失败分类
- 日志在前端是扁平的 `LogEntry`，没有阶段与结果分类

这导致三类具体问题：(1) 加 CSDN 时必须在 `PLATFORMS`、`match` 分支、`WEBVIEW_URLS` 三处同时改；(2) 用户看到"CSDN 适配器开发中"与"Cookie 已过期"是同一种红色文本，无法自助排查；(3) 平台请求逻辑与壳层命令耦合，无法单测。

## Goals / Non-Goals

**Goals**
- 平台元数据单一来源，前端与 Rust 路由从同一份定义派生
- 发布结果带失败分类（六类枚举），前端据此渲染不同的引导文案
- 每次同步产出结构化日志条目，成套出现（时间/平台/阶段/结果/消息/草稿链接）
- 把 `publish_*` 从 `lib.rs` 移到 `platforms/` 模块，`lib.rs` 只负责注册与路由
- 为 CSDN 预留"注册即可见、未实现即明确报 not_implemented"的路径

**Non-Goals**
- 不做 Markdown 到平台 HTML 的转换优化（Phase 2）
- 不做批量并发发布的线程池调度（Phase 2）
- 不做图片上传（Phase 3）
- 不引入前端状态库；沿用现有 `useState` 结构

## Decisions

### Decision 1: 平台注册表以 TypeScript 为权威，Rust 侧用生成式静态映射

**选择**：在 `packages/core/src/platforms/registry.ts` 定义权威注册表；Rust 侧 `platforms/mod.rs` 维护一个 `match` 硬映射，但由 tasks 中的校验脚本/测试断言两侧 id 集合一致。

**替代方案 A**：注册表放 Rust，TS 通过 `list_platforms` 命令运行时获取。
- 优点：真正单一来源。
- 缺点：前端首次渲染需等待异步 invoke，平台面板会出现空窗；离线/命令失败时 UI 完全不可用。
- 结论：否决。

**替代方案 B**：注册表用 JSON，两侧各自读取。
- 缺点：打包进 Tauri 资源需要额外配置，且 TS 侧失去类型推导。
- 结论：否决。

**理由**：前端要求"渲染即完整"，因此注册表必须是编译期可用的 TS 模块；Rust 侧只关心 `id → adapter` 分派，用 `match` 是最朴素的实现。双侧漂移风险用测试兜底而非架构兜底，成本最低。

### Decision 2: 失败分类用 Rust 枚举序列化为 snake_case 字符串

`FailureClass` 定义为 `#[serde(rename_all = "snake_case")]` 的 Rust 枚举，随 `PublishResult` 一起返回。前端 TS 侧定义同名联合类型做穷尽检查。

**替代方案**：用整数错误码。
- 缺点：前端需维护映射表，日志导出后可读性差。
- 结论：否决。

**理由**：字符串枚举在日志导出里自解释，TS 联合类型可让 `switch` 具备穷尽性检查，编译期就能发现漏分支。

### Decision 3: 判断凭据缺失放在适配器入口，不放在路由层

`platform_publish` 路由只负责 `id → adapter` 分派与"未实现即 not_implemented"；"配置是否齐全"由每个适配器自己的第一步检查。

**理由**：不同平台的必填字段差异巨大（博客园三字段、掘金单 Cookie），路由层无法泛化判断；把判断放在适配器入口，新增平台时责任边界自然清晰。代价是每个适配器都要写一遍配置检查，但这部分代码极少且必须逐个平台定制。

### Decision 4: 日志在前端内存中维护，导出走 Rust 文件写入命令

日志条目由前端在每次同步流程中生成（因为阶段进度只有前端知道），导出时序列化为 JSONL 传给 Rust 的写文件命令。

**替代方案**：日志在 Rust 侧集中记录。
- 缺点：前端阶段（预处理、Markdown 解析）不在 Rust 视野内，日志会缺阶段信息。
- 结论：否决。

**理由**：日志的完整性依赖前端对流程的掌控；Rust 只做"写盘"这件它擅长的事。

### Decision 5: CSDN 在本次变更中只注册不实现

注册表加入 CSDN 条目（id `csdn`、cookie 凭据），`platforms/mod.rs` 的 `match` 不为其提供 adapter，因此走 `not_implemented` 分支。

**理由**：这样 CSDN 在前端立即可见、状态可配置、日志有明确提示，而真正的适配器实现作为下一次变更（`phase1-csdn-adapter`）单独交付，避免本次变更范围膨胀。

### Decision 6: 文章以本地生成 UUID 为稳定身份，正文哈希降级为辅助校验

每篇文章在创建时由 Rust 侧生成一个 UUID v4 作为永久标识，存入 `articles` 集合。`sync_map` 以该 UUID 为唯一键。正文哈希仍会计算，但只存为每个平台记录里的 `bodyHash` 字段，用于回答"同步后是否又改过正文"，**绝不参与映射查找**。

**替代方案 A**：用规范化内容的 SHA-256 作为映射键。
- 致命缺陷：现有 UI 是"粘贴一段 Markdown 然后同步"，用户为了更新而修改正文时内容必然变化，指纹随之改变，导致查不到映射、**又新建一篇**。核心需求"再次同步即更新"会 100% 失效。
- 结论：否决（这是实现前审查发现的地雷）。

**替代方案 B**：用文件路径作为键。
- 缺点：重命名即失联；且当前应用没有文件打开流程。
- 结论：否决。

**替代方案 C**：仅前端内存保持当前文章。
- 缺点：重启即丢失全部映射，桌面软件不可接受。
- 结论：否决。

**理由**：UUID 与内容完全解耦，改正文、改标题、重启都不影响身份；正文哈希保留下来还能在 UI 上提示"本篇同步后又有改动"。代价是必须引入文章集合与文章管理视图（见 Decision 10）。

### Decision 7: sync_map 存于 Rust 侧 store，与凭据同库不同键前缀，键为文章 UUID

`sync_map` 写入既有的 `platform_config.json` store，键名采用 `syncmap:<articleUuid>`，其值为 `{ "<platformId>": { "id": "...", "published": bool, "bodyHash": "..." } }`。

**替代方案**：把 sync_map 放前端内存或 localStorage。
- 缺点：前端内存随会话消失，重启即丢失全部映射，功能退化为"永远新建"；localStorage 与凭据存储割裂，且 Tauri WebView 的存储可能被清理。
- 结论：否决。

**理由**：映射与凭据同属"跨会话必须存活"的业务数据，复用同一个 store 插件实例可减少插件注册数量；键前缀隔离（`syncmap:`）避免与既有 `cnblogs_config` / `juejin_config` 混淆，也让 `clear_cookie` 不会误删映射。以 UUID 为键还带来一个额外好处：删除文章时可用一次前缀匹配清空该文章的全部平台记录。

### Decision 8: `is_update` 与 `publish_status` 是两个正交维度，不做合并

| 组合 | 行为 | 调用接口 |
|---|---|---|
| `is_update=false, publish_status=false` | 新建草稿 | `newPost` / `draft/create` |
| `is_update=false, publish_status=true` | 新建并直接上线 | `newPost`(true) / `create`+发布 |
| `is_update=true, publish_status=false` | 更新草稿（不改变线上状态） | `editPost` / `draft/update` |
| `is_update=true, publish_status=true` | 更新并覆盖线上 | `editPost`(true) / `article/update` |

**替代方案**：合并成单一枚举 `Mode { CreateDraft, CreateLive, UpdateDraft, UpdateLive }`。
- 缺点：语义上确实等价，但会让命令入参与前端复选框状态之间多一层映射，前端"两个控件"到"一个枚举"的转换容易出错。
- 结论：否决（但保留作为后续可选优化）。

**理由**：前端恰好有两个独立控件（"是否更新"由映射记录自动决定、"是否覆盖线上"由用户勾选决定），两个布尔值能得到最直接的对应关系。

**安全约定（硬约束）**：
1. `publish_status` 默认 `false`，且 Rust 侧不得存在任何把它置为 `true` 的兜底逻辑。
2. 博客园 XML-RPC 的 publish 布尔值**必须**由 `publish_status` 直接驱动，禁止硬编码 `0` 或 `1`。
3. 前端在 `publish_status = true` 时必须先弹不可逆覆盖确认框，用户确认后才发起 invoke。
4. 更新路径失败时**绝不降级为新建**，必须原样返回失败。

### Decision 9: 掘金草稿更新走 `article_draft/update`，请求体字段已实测锁定

已发布的掘金文章与草稿走不同接口。实测结论分两部分：

**已验证（草稿分支）**：`POST https://api.juejin.cn/content_api/v1/article_draft/update` 携带 `draft_id` 返回 `err_no: 0`，且掘金后台草稿数量不增加（确认为原地更新而非新建）。其请求体字段契约如下，任一字段名写错都会得到 `err_no: 2`（参数错误）：

- `title`：标题
- `mark_content`：Markdown 原文（**不是** `markdown`）
- `content`：HTML 正文（与 `mark_content` 同时提供，缺一不可）
- `category_id`：必须为掘金官方分类 ID（本项目取 `6809637773935378440`），传 `"0"` 或空串会触发 `err_no: 2`
- `tag_ids`：数组，无标签时传空数组 `[]`
- `brief_content`：空字符串（**不是** `brief`）
- `edit_type`：数字 `10`
- `draft_id` 与 `id`：双传同一个 ID（老接口对 `id` 有依赖）

**仍未验证（已发布分支）**：`article/update` 携带 `article_id` 这一假定尚未在真实已发布文章上验证。因此 `existing_published = true` 时走该分支属于已知未验证路径，首次使用若失败应优先怀疑 URL 或字段名，并同步更新本条决策。

**理由**：把未验证的事实显式标注为假定，比在规划里写成既定事实更安全——避免实现阶段把假设当成文档依据。草稿分支的字段契约已由实测收敛，故从「假定」升级为「已验证事实」。

### Decision 10: 左侧编辑器升级为「文章列表 + 当前文章」，文章集合由 Rust 持久化

左侧区域由单一 Markdown 文本框改为两段式：上方文章列表（按标题区分，空标题显示「未命名文章」占位），下方当前文章的标题输入与正文编辑区。切换文章前先落盘当前文章的改动。`articles` 集合由 Rust 侧 `articles.rs` 负责增删改查与持久化，前端只做视图。

**替代方案 A**：保持单文本框，仅在前端加一个「文章标识」下拉框。
- 缺点：无法表达"新建文章"，用户仍会困惑于"我现在在改哪篇"。
- 结论：否决。

**替代方案 B**：文章存成磁盘上的 `.md` 文件，由 Rust 读写目录。
- 优点：用户可直接用外部编辑器管理。
- 缺点：需要处理用户手动增删文件、编码、路径安全等额外问题，超出本次范围。
- 结论：留作 Phase 2 候选（非目标）。

**理由**：先用 store 里的结构化集合把"文章身份"这件事做对，成本最低且与现有 `platform_config.json` 存储体系一致；磁盘文件化是独立的产品决策，不该混在本次重构里。

**约束**：
- 删除文章必须二次确认，且同步删除该文章的全部 `syncmap:` 记录。
- 空工作区必须显示创建引导，不能是一片空白。

### Decision 11: 掘金「新建即发布」降级为 not_implemented，但前端给出可读引导

掘金当前只验证过 `article_draft/create`，真正发布上线通常还需额外调用发布接口（未经抓包确认）。因此 `is_update=false + publish_status=true` 这一组合在掘金侧返回 `not_implemented`，message 明确写出「掘金目前仅支持保存草稿，请到掘金后台手动发布」。

**替代方案**：让掘金忽略 `publish_status` 静默只建草稿并返回 `success: true`。
- 缺点：用户以为已经发布上线，实际只是草稿，属于**谎报成功**，比报错更糟。
- 结论：否决。

**理由**：明确失败 + 可读引导，用户能立刻采取正确行动；静默降级会破坏用户对"发布状态"的信任，也污染 `sync_map` 的 `published` 字段语义。

**前端配合**：用户对掘金勾选"直接发布"时，应在提交前就给出提示（而非等请求回来才报错），并在日志中记录该平台为 `not_implemented`。


## Risks / Trade-offs

| 风险 | 影响 | 缓解 |
|---|---|---|
| 双侧注册表漂移 | 前端显示的平台 Rust 侧路由不到，或反之 | 增加一致性测试，断言 TS 与 Rust 平台 id 集合相同 |
| 掘金风控导致的偶发失败 | 自动化测试无法稳定覆盖 | 验收标准定为"错误归类正确"而非"请求成功"；用真实 Cookie 手工验证 |
| `publish_*` 迁入 `platforms/` 时误改 URL 常量 | 掘金草稿链接拼错 | 迁移时逐字段对照现有实现，保留原常量值不变 |
| DPAPI 旧明文条目 | 用户升级后凭据读取得到乱码 | `load_cookie` 解密失败即视为需重新登录，绝不回退用原值 |
| CSDN 注册但不可用 | 用户误以为已支持 | 前端对该平台置灰并显示"即将支持"；publish 返回 not_implemented |
| 误将 `publish_status` 置 true 导致线上文章被覆盖 | 不可逆的线上内容损坏 | Rust 侧无任何 true 兜底；前端强制不可逆覆盖确认框；博客园布尔值直连参数并有单测断言 false 场景 |
| 更新失败后静默降级为新建 | 平台出现重复文章，用户难察觉 | 更新路径禁止 fallback，失败直接返回；单测断言更新失败时不产生 create 请求 |
| 正文大幅改动导致映射丢失 | 被判为新文章而重复创建 | 映射键是文章 UUID 而非内容哈希，改正文不影响查找；`bodyHash` 仅用于提示"同步后又有改动" |
| 掘金 `article/update` 接口假定有误 | 更新已发布文章直接失败 | 安排独立抓包验证任务，按真实结果修正 URL 与字段 |
| 掘金「新建即发布」不可用 | 用户勾选直接发布后失败 | 返回 `not_implemented` 并给出可读引导；前端在提交前就提示，不静默降级 |
| `syncmap:` 键与凭据键混淆 | 清除凭据时误删映射，或反之 | 键前缀严格隔离；`clear_cookie` 只处理 `*_config` 键，有单测覆盖 |
| 文章集合与映射残留不一致 | 删除文章后残留孤儿映射 | 删除文章时同步清理该 UUID 的全部 `syncmap:` 记录 |
| 编辑器重构破坏既有发布交互 | Markdown 输入或平台面板不可用 | 重构时保留原输入控件与平台面板，仅新增列表与标题层 |

## Migration Plan

1. 新增 `packages/core/src/platforms/registry.ts`，内容为现 `PLATFORMS` 数组的等价物（含 CSDN 的 `available: false`）
2. `App.tsx` 删除本地 `PLATFORMS` 常量，改为从 `@openpub/core` 导入
3. Rust 侧新建 `platforms/mod.rs`，把 `publish_cnblogs` / `publish_juejin` 原样迁入子模块，`lib.rs` 的 `platform_publish` 改为委托
4. `PublishResult` 结构与 `lib.rs` 中的所有构造点同步加上 `failure_class` 字段；掘金/博客园的错误分支按 Decision 2 归类
5. 前端 `LogEntry` 扩展 `stage` / `outcome` / `failureClass`，日志面板按三态渲染
6. CSDN 条目加入注册表，保持 `available: false`
7. 新增 `articles` 集合与 Rust 侧 `articles.rs`，提供创建/列表/更新/删除命令，UUID 在创建时生成
8. 新增 `sync_map` 存储与 `load_sync_map` / `save_sync_entry` / `clear_sync_entry` 命令，键为 `syncmap:<articleUuid>`
9. 左侧编辑器重构：文章列表 + 当前文章标题/正文编辑区；切换前落盘；空工作区显示创建引导
10. `platform_publish` 入参扩展 `article_id` / `existing_id` / `is_update` / `publish_status`，`PublishResult` 扩展 `operation` / `isPublished`
11. 博客园与掘金适配器分别补 `editPost` / `draft-update` / `article-update` 路径；博客园 publish 布尔值改为动态；掘金 create-live 返回 `not_implemented`
12. 前端新增"覆盖已发布文章"复选框与不可逆覆盖确认框，并在有映射时切换按钮文案

**回滚**：本次变更为纯增量重构，若出现问题，回滚到 Phase 0 归档时的提交即可——凭据数据格式未变（仍在原 store 键位）。`articles` 与 `syncmap:` 键为纯新增，回滚后成为孤儿键，不影响功能。

## Open Questions

- 掘金已发布文章的更新接口（假定 `content_api/v1/article/update`）需抓包验证，见 Decision 9
- 掘金「新建即发布」的真实发布接口：待抓包后决定是否补齐（当前按 Decision 11 返回 `not_implemented`）
- 日志导出的默认目录（建议 `appConfigDir/logs/`，待实现时确认）