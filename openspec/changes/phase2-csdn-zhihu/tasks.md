# Tasks

## 1. 契约验证（先钉死契约，再写代码）

- [x] 1.1 用浏览器抓取 CSDN 草稿保存请求，记录端点 URL、请求头（含 `x-ca-*` 签名头名与 CSRF 头名）、请求体字段名与必填项，并验证记录中不含 `TODO` 之类的占位值
- [x] 1.1b 确认 CSDN 发布走的是同一端点 `v3/mdeditor/saveArticle`，草稿与发布仅差请求体 `status`：`2` = 保存草稿，`0` = 提交发布（进审核队列）。状态字典由文章列表接口确认：`draft` = 草稿箱、`audit` = 审核中/未通过、`all_v2` = 已发布。注意列表返回的是字符串状态、请求传的是数字参数，两套取值不可混用；源码注释中「`1` 表示直接发布」的说法已作废
- [ ] 1.2 用浏览器抓取知乎建稿与发布两组请求，记录端点、`_xsrf` 头名、请求体字段名，并验证 `_xsrf` 值确认为百分号编码
- [ ] 1.3 把两家的契约结论写入 `design.md` 的新 Decision 章节，按「已验证（草稿分支）/（发布分支）」标注未验证部分，并验证每条字段名都能在抓包记录中逐字对应
- [ ] 1.4 判定知乎是否需要 Cookie 之外的额外签名；若需要且无法复现，按 design Decision 7 记录降级决定，并验证该结论已写入 `design.md` 与 `proposal.md` 的影响说明

## 2. 共享工具上提（纯重构，掘金行为不变）

- [x] 2.1 新建 `platforms/shared/` 模块并注册到 `mod.rs`，验证 `cargo build` 通过且模块可被引用
- [x] 2.2 实现 `shared/cookie.rs` 的按键取值，覆盖「正常提取」「重复键取最后」「缺键报配置失败」「跳过无等号片段」四种情况，验证对应单测全绿
- [x] 2.3 实现 `shared/csrf.rs` 的 token 提取与 URL-decode，验证 `abc%3D%3D` 解码为 `abc==`、无编码值原样返回、缺键报配置失败三条单测通过
- [x] 2.4 把 `juejin.rs` 的 `escape_html` / `to_html_content` 迁入 `shared/markdown.rs`，验证空源产出单个空段落、特殊字符被转义、每行一个段落三条单测通过
- [x] 2.5 把 `juejin.rs` 与 `cnblogs.rs` 各自的 `truncate_chars` 合并为 `shared/truncate.rs`，验证「超长中文安全截断不 panic」「短文本不加标记」两条单测通过
- [x] 2.6 让 `juejin.rs` 与 `cnblogs.rs` 改为调用共享实现并删除各自的私有副本，验证掘金既有 8 个单测与博客园单测全部通过（这是重构无偏差的硬门槛）
- [x] 2.7 让 `juejin.rs` 的 `read_cookie` 改为调用 `shared/cookie.rs`，验证掘金 Cookie 读取失败仍归类 `not_configured` 且单测通过

## 3. 两段式部分成功的映射语义

- [x] 3.1 把 `lib.rs` 中 sync_map 的写入条件由 `result.success` 改为「`post_id` 非空且非占位符」，`published` 取 `result.is_published`，验证既有「成功即写映射」行为未变（掘金/博客园路径单测通过）
- [x] 3.2 补充单测断言「失败结果携带可用 ID 时仍写入 `published: false` 映射」，验证该单测通过且占位 ID 不落盘
- [x] 3.3 补充单测断言「部分成功后再同步走 update 而非 create」，验证该单测通过

## 4. CSDN 适配器

- [x] 4.1 新建 `platforms/csdn.rs`，实现 `resolve_url`：草稿意图路由到 `v3/mdeditor/saveArticle`，`publish_status = true` 与 `existing_published = true` 均在发请求前显式拒绝（`not_implemented`），验证六条路由与拒绝单测通过（含「拒绝文案必须声明未发请求、未降级为草稿」「已发布目标优先级高于 publish_status」）
- [x] 4.2 实现请求体构造，使用共享 markdown 转换产出 HTML、标签为空数组，验证请求体字段名与 1.1 抓包记录逐字一致的单测通过
- [x] 4.3 实现 Cookie + CSRF 鉴权与浏览器等价请求头，验证缺 Cookie、缺 CSRF 键两种情况均归类 `not_configured` 且不发起请求
- [x] 4.4 实现响应解析与 ID 提取（复用占位符过滤规则），验证成功返回 ID、提取失败报 `platform_error` 两条单测通过
- [x] 4.5 在 `mod.rs` 的 `dispatch` 中注册 `csdn` 分支，验证该分支被调用且未注册平台仍返回 `not_implemented`
- [x] 4.6 真机联调：新建 CSDN 草稿成功拿到真实文章 ID `170288151`，后台草稿箱确认接收；再次同步同一文章确认走草稿更新分支且 CSDN 后台文章数未增加，终端日志打印的 StringToSign、签名头、请求体与响应可用于逐字段核对
- [x] 4.7 实现阿里云网关 `x-ca-*` HMAC-SHA256 签名（`build_string_to_sign` / `compute_signature` / `sign_headers`），验证签名串格式、Content-Type 漂移敏感性、GET/POST 段数对称三条单测通过，并以 4.6 的真机成功为最终验收
- [x] 4.8 实现直接发布分支：勾选发布时请求体 `status = 0`，未勾选时 `status = 2`；保留「已发布目标 + 未勾选覆盖」的拒绝守卫；已删除 `OVERWRITE_HINTS` 的 `csdn` 项。单测覆盖 `status` 取值、发布路径复用同一端点、勾选覆盖后允许更新已发布目标、未勾选时拒绝共四条。**注：该条当初把 `is_published` 定为 `false` 并假定「进审核队列」，已被 4.10 / 4.10c 的真机结论推翻并修正**
- [x] 4.8b 按 v3 `mdeditor/saveArticle` 抓包对齐请求体：`read_type` 修正为 `readType`（既有 bug，早期靠后端宽容才侥幸成功），恢复草稿分支的 `pubStatus: "draft"`，补齐 `level` / `original_link` / `authorized_status` / `not_auto_saved`（字符串 `"1"`）/ `source: "pc_mdeditor"` / `cover_images` / `cover_type` / `is_new` / `vote_id` / `resource_id` / `creation_statement` / `creator_activity_id`；`Origin` / `Referer` 对齐 `https://editor.csdn.net`。单测断言 `read_type` 已彻底移除、官方必填字段齐全、`not_auto_saved` 为字符串
- [x] 4.9 curl 隔离验证改为「直接进草稿新建」：21 个字段与 v3 抓包逐字段核对一致后，隔离 cookie 问题不再需要单独跑 curl——第 1 步草稿新建返回 401/403 即 cookie 失效，返回业务错误则按剩余未知项排查。抓包留档见 `docs/reference/csdn-v3-mdeditor-saveArticle-draft.curl.txt`（原始 curl 原文待补）
- [x] 4.10b 发布分支兜底标签：真机发布时服务端返回 `{"code":400,"msg":"请设置文章标签"}`，证明草稿与发布的校验尺度不同。`PublishRequest` 无标签字段，发布分支填 `DEFAULT_PUBLISH_TAG = "OpenPub"`、草稿保持 `""`。单测 `publish_request_body_fills_a_tag` 与 `draft_request_body_keeps_tags_empty` 锁死两侧差异。副作用：发布文章会带固定标签，接入标签输入 UI 后改为透传
- [x] 4.10 真机验证 CSDN 发布分支：勾选发布后响应成功返回文章 ID `170289672`，CSDN 后台「已发布」列表出现该文章（时间 14:13:33），「审核中/未通过」保持 0、草稿箱由 7 减为 6。**结论：`status = 0` + `pubStatus = "publish"` 为直接对外发布，不进审核队列（或秒过）**，此前「进审核」假设作废
- [x] 4.10c 修正 `is_published`：真机证明为直接发布后，`is_published` 改为取 `request.publish_status`（此前写死 `false` 会让映射记成未发布，导致下一次未勾选覆盖的同步把线上文章打回草稿）。成功结果构造抽为纯函数 `build_success_result`，单测 `publish_success_marks_the_article_as_published` / `draft_success_is_not_marked_as_published` 双向锁死
- [ ] 4.10d 真机验证守卫复活：新建文章 → 草稿同步 → 勾选发布（`is_published = true` 写回映射）→ **不勾选覆盖**再同步一次，预期被 `resolve_url` 拦截并提示「未勾选覆盖」，不得发出任何请求；若请求发了出去，说明映射未写入 `published`，属灾难级问题
- [x] 4.11 收窄 `is_new`：只在 `Create` 分支发 `1`，`Update` 分支不带该字段（无条件发 `1` 有触发「新建而非更新」、产生重复文章的风险）。单测 `update_request_body_omits_is_new` 断言更新分支无该字段且 `id` 保留
- [ ] 4.12 真机验证草稿更新分支：对刚建的文章改内容再同步，确认不产生重复文章；若仍多出一篇，则需补抓 v3 更新包，核对主键字段名（`id` → `article_id`）与更新时的 `is_new` 取值
- [x] 4.13 ~~真机验证发布后的再次同步：确认「审核中的文章 + `status = 2` + 文章 id」不会把文章从 `audit` 打回 `draft`~~ —— 该条基于「发布后进审核队列」的假设，已被 4.10 真机结论作废（发布即直接上线，不存在审核中间态）。对应风险改由 4.10d 的守卫复活验证覆盖

## 5. 知乎适配器

- [ ] 5.1 新建 `platforms/zhihu.rs`，实现建稿请求（Cookie + XSRF 头），验证成功返回 ID 且 `isPublished: false` 的单测通过
- [ ] 5.2 实现两段式发布：`publish_status = true` 时在建稿成功后追加发布动作，验证「仅建稿」「建稿 + 发布」「发布失败仍携带草稿 ID」三条单测通过
- [ ] 5.3 实现 `is_update` 的草稿更新路径，验证「未发布目标不隐式发布」「已发布目标走文章更新」两条单测通过
- [ ] 5.4 实现缺 Cookie / 缺 XSRF 的前置拒绝，验证两种情况均归类 `not_configured` 且不发起请求
- [ ] 5.5 在 `mod.rs` 的 `dispatch` 中注册 `zhihu` 分支（若 1.4 判定降级则改为返回 `not_implemented`），验证分支行为与判定结论一致
- [ ] 5.6 真机联调：建知乎草稿 → 勾选覆盖再同步 → 确认草稿变为已发布且后台无重复文章，验证终端日志中的原始响应可用于定位失败阶段

## 6. 注册表能力位与前端提示

- [ ] 6.1 在 `PlatformDefinition` 新增 `publishCapability` 字段并为全部平台赋值（掘金 `draft_only`，其余 `live`），验证 `pnpm build` 类型检查通过
- [ ] 6.2 扩展 `auditRegistry()` 校验能力位与已实现清单的一致性，验证人为把某平台能力位写错时校验能报出问题
- [ ] 6.3 把 `App.tsx` 中 `selectedPlatforms.has("juejin")` 的硬编码提示改为按能力位渲染，验证掘金提示文案不变且新增草稿类平台无需改前端代码（**当前过渡态**：已改为 `OVERWRITE_HINTS` 映射表，文案按平台各自行为区分，但仍是硬编码映射，未引入能力位）
- [x] 6.4a 把 `ADAPTER_IMPLEMENTED_IDS` 加入 `csdn` 并把 `available` 置 `true`（在 4.6 通过后执行），验证注册表审计无告警且前端显示为可配置
- [ ] 6.4b 把 `ADAPTER_IMPLEMENTED_IDS` 加入 `zhihu` 并把 `available` 置 `true`（在 5.6 通过后执行）
- [x] 6.5a 确认 CSDN 复用既有 Cookie 配置弹窗，验证可保存 Cookie 并在重启后保持「已配置」状态（4.6 真机联调已验证）
- [ ] 6.5b 确认知乎复用既有 Cookie 配置弹窗，验证可保存 Cookie 并在重启后保持「已配置」状态

## 7. 集成验证

- [x] 7.1 运行 `cargo test --lib`（`apps/desktop-app/src-tauri`），验证全部单测通过且掘金 8 个既有单测仍在其中（当前 187 passed / 0 failed）
- [x] 7.2 运行 `pnpm build`，验证类型检查与生产构建通过
- [ ] 7.3 一次批量同步勾选博客园、掘金、CSDN、知乎，验证四条日志条目独立产生、部分成功不影响其他平台、失败条目阶段标注正确（知乎发布失败应标 `publish`）
- [ ] 7.4 断开网络后同步一次，验证四个平台均归类 `network_error` 且界面无卡死

## Workflow follow-up

- 在实现与集成验证全部通过后归档该变更。
- 归档前确认 `design.md` 中由 1.3 / 1.4 新增的契约章节已反映最终实测结论。