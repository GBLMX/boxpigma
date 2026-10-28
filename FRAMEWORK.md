# 框架与结构调整

这份文档记录 boxpigma 里"把自写实现换成成熟方案"的决策、依据与代价，以及**同样重要**的
**被否决的候选**。判据只有一条：

> **新增依赖必须能对应到一个已量到的问题；量不到就不加。**

反面例子（本仓库已经拒绝过的）见下面「已否决」一节。判断"哪些必须继续自己写"时用同一条尺子：
成熟库确实不提供，或换过去会改变契约 —— 那就保留自写。

---

## 一、已采纳

| 方面 | 改前 | 改后 | 依据（实测） | 位置 |
| --- | --- | --- | --- | --- |
| GUI 后端状态 | iced 的 `App` 自己持有 `ApiService`、队列、导航和登录状态，与 TUI 各一份；刷新靠窗口内重新拉取接口 | `ferriso-core` 的 actor：一个任务独占后端状态，前端只发语义 `Command`、收 `Arc<View>` 快照 | 歌单详情 100 条、切页、搜索、登录和 64 次空 tick 全走 actor；GUI 不再建第二个 runtime，也不再持有 `ApiService`/`PlaybackEngine`。窗口关闭时 actor 的 `shutdown` 保存队列与 cookie | `src/backend/`、`github.com/GBLMX/Ferriso`（`ferriso-gui` 的 iced adapter） |
| 日志 | 手写 `log::Log` 实现：`Mutex<File>` 追加写，**无轮转、无上限**，调用线程持锁同步写 | `tracing` + `tracing-subscriber` + `tracing-appender`：按天轮转、保留最近 7 个、行内带模块路径与本地时间 | 旧日志实测已达 **2,892,695 B** 且无上限；替换后**135 个 `log::*!` 调用点一行未改**（`tracing-log` 桥接 `log` 记录） | `src/logger.rs` |
| 对比度 | 手写 WCAG 亮度/对比度（`relative_luminance` / `contrast_ratio`），且 `ui/contrast_audit.rs` 的审计测试里还有**第三份**拷贝 | `palette` 的 `Wcag21RelativeContrast`；全仓颜色数学收敛到一处 | 全 **2²⁴ 个 8 位 sRGB 颜色**逐值对拍，最大亮度偏差 7.29e-5（palette 用 CIE/Lindbloom 全精度系数，旧代码用 WCAG 取整系数）；**19 个内置主题的 `on_accent` 结果全部一致**；唯一分歧带是黑白锚对比度打平处（合成扫描的 0.005%，可读性相同，无内置主题落在那） | `src/config/theme.rs`、`src/app/theme.rs`、`src/ui.rs`、`src/ui/contrast_audit.rs` |
| 桌面通知 | 只用 `OSC 9`（单串正文） | kitty 走其自有的 `OSC 99`：标题与正文分开（`p=`）、Base64 负载（`e=1`）、`f=` 声明应用名便于过滤、`i=`/`d=` 分块并避免堆叠 | kitty 官方文档：kitty 实现 `OSC 99`，同时**兼容**旧式 `OSC 9`；其余终端保持 `OSC 9` **逐字节不变**（有测试钉住） | `src/utils/terminal/`（按主题分成 capability / color / background / sequences） |
| 网络超时 | 只有 `ncm-api` 设了 30s；搜索、封面下载、音频流**一处超时都没有** | 统一 `connect_timeout` 10s + `read_timeout` 30s；短请求另加 30s 总时限 | **音频流刻意不加总超时**（会截断正在播放的下载）—— 用 34 秒持续下载的探针实证不会被切断；`read_timeout` 是每次读的超时、读完即重置，语义已在 reqwest 源码里核实 | `src/app/builder.rs` |
| 页面结构 | 绘制分发、页面按键、键位表三处各自维护 | 一张表：`PageSpec { name, key, render }` + `Page::spec()/opened_by()/on_key()` | 新增一个页面从改 **9 个文件降到 2 个**；`ui.rs` 生产代码里的页面匹配臂 5 → 0、`layout.rs` 3 → 0；等价性用旧/新代码各渲染 14 个场景比对缓冲区哈希验证 | `src/state/page.rs`、`src/ui.rs`、`src/layout.rs` |
| 铺底色 | `Block::default().style(bg)`（用一个无边界的 `Block` 只为刷背景） | `Fill::new(" ")` | `Fill` 的渲染就是 `set_symbol + set_style`，像素等价；换完之后编译器指出 `Block` 在该文件已无其它用途 | `src/ui.rs` |
| 启动画面字形 | 手绘 3 行 ASCII 字（B 的下面两行相同、X 的交叉挤在一行里，都读不清） | FIGlet 字体 **`Calvin S`** 的渲染结果，3 行 × 23 列，作为常量内嵌 | 用自写的 `.flf` 解析器（含 kerning 布局）比较 13 款字体后选定；运行时**不带字体依赖** | `src/ui/splash.rs` |
| 采样率转换 | 全部交给音频后端（系统混音器自己重采样） | 设备与文件采样率不一致时用 `rubato` 的同步 FFT 重采样（比例固定，正是播放一首歌的情形） | 44.1k→48k 的流实测产出 ≈48000 帧并报告设备采样率（测试）；**采样率一致时不构造任何适配器**，样本原样到达 sink = 位完美路径；适配器构建失败或设备不给该格式时把源原样交回 | `src/playback/chain.rs`、`src/playback/player.rs` |
| 音色与电平 | 无任何处理 | `biquad` 参量峰值 EQ（每声道一份滤波状态）+ `ebur128` 瞬时响度归一化（100 ms 一块，每块只走 1/10 的平滑量） | 100 Hz 正弦过 +6 dB 频段幅度 ≈2×、过 −6 dB ≈0.5×；−46 dBFS 的曲子被提升 >2×、响亮曲子被压低到 <0.8×（测试）；低于 −70 LUFS 的窗口视为静音不矫正；seek/换曲时重置滤波器与响度计 | 同上 |
| 主题 | 主题名取自 `HashMap`，`:theme` 循环顺序每次启动都不同 | 名字排序固定；新增 `random`（启动时落定一次、`:theme random` 重掷并写回） | 排序后 `:theme` 顺序稳定；`random` 只在**设置主题时**解析，绝不在每帧的 `resolve_theme` 里掷（否则每帧换色） | `src/config/theme.rs` |

**结构性调整**（不属于依赖层面，但同样是"把决定只写一次"）：

- **启动画面的高度由字形决定**：此前布局写死 3 行、渲染按 0..2 手工索引 —— 加到第 4 行就 panic。
  现在 `layout::splash` 接收行数、渲染从 `LOGO.len()` 推导，字形是唯一写下来的地方。
- **日志调用点零改动**：`tracing-log` 桥接意味着换栈不必改 135 处调用点 —— 这是选它的首要理由。
- **主题解析只在"设置主题"时发生**：`resolve_theme` 每帧都跑，任何随机性都不能放在那里。
- **多源兜底链移除（2026-09-22）**：第三方兜底 crate 与它依赖的子模块、配置里的兜底源一节整体
  删除。播放解析链收敛为 **网易云 → 云盘兜底 → 报错**（本地文件仍走自己的路径），搜索也只剩
  网易云一个源；代理目标键随之删除，并入唯一的 `proxy`（旧文件由 `config_version` v1 → v2
  迁移处理）。记在这里而不是悄悄删掉：上面「网络超时」一行里指向兜底 crate 的位置已不存在，
  那一行记录的是当时的改动。
- **终端 / IPC / 播放设备三个"万能模块"按主题拆开（2026-09-22）**：`utils/terminal.rs`(1483 行)
  → `capability`（能力探测、图像协议、光标）/ `color`（颜色数学与调色板）/ `background`（明暗
  探测）/ `sequences`（转义序列发射）；`ipc.rs`(1100 行) →
  协议 + `transport`（协议半边不再有 `cfg`）；`player.rs` 的 Linux
  ALSA stderr 守卫 → `player/device.rs`。公开路径由
  facade 重导出保持不变，**调用点零改动**（`git diff` 未触及任何使用方）。代价：模块内多了一层
  `super::super::`，test-only 的 re-export 需要一处 `#[allow(unused_imports)]`（注释说明原因）。
- **按键类型出栈（2026-09-22）**：应用自有 `key::{KeyPress, KeyCode, Modifiers}`，翻译只在事件
  边界（`app::event`）发生一次；`input` / `state` / `ui` 的签名里不再出现终端库的类型，
  crossterm 的键类型只剩 `key.rs`（转换）与 `app/event.rs`（边界）。依据：按键是应用自己的
  词汇，不该由终端库定义。代价：Super / Hyper / Meta 不再区分（没有任何绑定用到它们），
  `modifiers == KeyModifiers::CONTROL` 的严格含义改由 `is_ctrl_only()` 表达。
- **`ui.rs` 的测试仪表拆出（2026-09-22）**：`shots`（自绘预览图）、`contrast_audit`、
  `theme_background`、`frame_bench` 各成一个文件，`ui.rs` 1003 → 444 行只留 shell。测试路径与
  `#[ignore]` 全保留；**预览图无需重生成**（五个场景都不含活动搜索框，也不含已删除的第三方队列
  标签，逐张核对过）。
- **配置迁移改成"启动即重写"（2026-09-22）**：`config_version` 升级以前只改内存、等下一次
  `save()` 才落盘 —— 于是被新 schema 删掉的键会一直躺在用户的文件里。现在迁移在**备份之后**
  当场升级**加载时那个文件**（不是 `save()` 自己的目录），并沿用 `save()` 的空文档保护。
  已经是当前版本的文件一个字节都不动，来自更新版本、以及解析失败的文件同样不动。
- **迁移从"整份重写"改成"就地编辑用户自己的文档"（2026-09-22，同一天的第二轮）**：第一轮把
  struct 序列化后覆盖（`Config::to_toml()`），于是**用户手写的注释与排版在迁移那一次会丢**。
  现在 `migrate_document` 直接在用户文件的 `toml_edit::DocumentMut` 上改：值、顺序、缩进与注释
  逐字保留，只有 schema 要求的那几行动（删掉新 schema 已经没有的键、就地改写 `config_version`
  —— 后者还沿用该行原有的行尾注释与间距）。迁移后的文本会被**回读成 `Config`**，所以"跑的就是
  文件现在说的"、规则只有一份。判据不另造键表：某个键还算不算 schema 的，交给 `Config` 自己的
  `Deserialize` 回答 —— 换个四种形状的探针值（数字/字符串/数组/表）重读文件，四种都被忽略即为
  已被删除；前提是配置图里不存在"四种通吃"的 catch-all 字段（已逐个核对），而这条前提由
  `the_sweep_keeps_every_key_the_config_has` 与**随仓库发布的 `config.example.toml`** 一起钉住。
  代价：被删掉的键**连同它自己的注释**一起走（备份 `config.toml.bak-v{旧版本}` 里都在）；
  `save()` 仍是整份序列化，"运行中改设置再保存"依旧不保留注释 —— 这一条只关于启动时的迁移。

- **"取锁"与"原子写"各只写一次（2026-09-29）**：取锁的 poison 决策此前散成四种写法（`unwrap`、`unwrap_or_else(into_inner)`、`if let Ok` 静默跳过、`map_err` 转 `io::Error`），其中 `ipc` 的四处 `unwrap` 会在别的线程 panic 之后把进程带走；而"临时文件 + rename"的原子写只有 `cookie.rs` 一处，应用侧的配置与缓存索引还是原地截断。现在 `utils::sync::{mutex, read, write}` 提供"其他线程的 panic 与这份数据无关，取走它"，`utils::fs::write_atomic` 提供同目录临时文件 + `rename`；唯一保留的例外是 `playback::player` 把 poison 转成 `io::Error`（调用方有那条通道），理由写在 `utils::sync` 的模块文档里。除两处刻意的行为变化（`ipc` 不再 panic、`cache` 的两处 `if let Ok` 不再静默跳过）外调用点零变化

- **只保留 Linux（2026-09-29）**：Windows / macOS / BSD 的支持连同代码一起删除 —— WASAPI 独占输出整块（`src/playback/exclusive.rs` 599 行、`[audio] exclusive`、`wasapi` 依赖）、IPC 的命名管道传输（`ipc/transport/windows.rs`）、Windows 控制台背景探测（`terminal/background/windows.rs`）、自更新里的 `.zip` 解包与 `mklink /J` junction、`install.ps1`（570 行），以及全部 `#[cfg(unix)]` / `#[cfg(windows)]` / `cfg!(windows)` 分支（现在无条件编译）；cpal 的 host 选择里 FreeBSD/NetBSD/OpenBSD/DragonFly 一并消失。**刻意保留 14 处 `#[cfg(all(target_os = "linux", target_env = "gnu"))]`**：它编码的是真实的 glibc/ALSA 约束（读 `/proc/self/status` + `libc::malloc_trim`），是唯一在 Linux 上仍可能为假的门，不是可移植性开关。发布契约未动：`boxpigma-<triple>.tar.gz` 与裸名 `SHA256SUMS` 仍是 `install.sh` 的 `awk` 查找和 `src/update.rs` 解析时的格式。同一工具链前后对拍：二进制 **−7,896 B（−0.07%，11,220,552 → 11,212,656 B）** —— 删掉的 Windows 代码本来就不进 Linux 构建，只有 `exclusive.rs` 的平台中立辅助逻辑此前会被编进去。代价：3 个只测 WASAPI 编码/采样率协商的单测随文件删除

- **Ferriso sidecar API 已可用，但 boxpigma 尚未迁移（2026-10-28）**：依赖升级到
  Ferriso `1924a60`，框架新增 `CoreContext`、`Sidecars`、`run_with_app_services` 与
  `request_exit`。boxpigma 目前的 IPC/MPRIS bridge 仍在 `Backend` 内，因为 status、
  queue 与 search 需要同一份 actor 数据；这是后续迁移候选，不是当前行为变化。

- **TUI 美术升级（2026-10-28）**：接入 `ferriso-tui::art`，播放条使用 aurora 动态
  背景与 pulse 边框，当前歌词使用 shimmer 扫光；时间来自现有 80ms tick，不增加
  独立动画线程。颜色仍从主题调色板推导，不硬编码。

## 二、已否决（附依据）

| 候选 | 否决理由（实测） |
| --- | --- |
| `unicode-truncate` 替换手写 CJK 截断 | **不等价**：124 万组模糊测试显示分歧全部落在 emoji/ZWJ/控制字符上（按**字素簇**边界 vs 按字符；控制字符宽度算 1 vs 算 0）。测试不能被迁就，因此保留自写，依赖已移除 ✗ |
| `nucleo` 替换搜索匹配器 | 主搜索是**远端**完成的，本地只有 20 行子串过滤，UI 也**没有匹配字符高亮** —— 换上去是**功能变更**而不是优化 |
| `palette` 替换 16/256 色量化 | `palette` **不提供**终端调色板量化（已通读其源码确认），这部分手写必须保留 |
| `colorgrad` 替换渐变预设 | 只在全局 registry 缓存里、非项目依赖；6 个预设中 5 个与自写实现逐值一致、Cubehelix 不同 —— 收益不足以引入一棵依赖树 |
| tokio `taskdump` 做进程内卡死诊断 | 接线正确（信号确实到达、快照确实被采集到），但把快照**渲染**成回溯那一步在本机不返回，且采集时的 `poll` 会卡住工作线程、外层 `timeout` 来不及生效 —— 一次信号就能让监听永久失效。改用外部抓栈（见 README 的「排障」） |
| `tracing` 的 `EnvFilter` | 需要 `regex-automata` 等一串依赖；配置只给一个全局级别，用内置的 `LevelFilter` 即可 |
| `reqwest-retry` / `backon` | 未在本机 vendor，未核实；现有手写重试只有 8 处且各有明确语义（如 NCM 解析网络失败重试一次），先量化问题再谈 |
| `candle` / `burn` 做 AI 音频超分 | 模型体积与推理延迟跟"一个终端里的音乐客户端"的定位不符；重采样/EQ/响度已经把这轮能拿的可听收益拿到 |
| 一批未经验证的音频 crate（`oximedia-audio`、`oxideav-flac`、`oxiaudio-dsp`、`libflo-audio`、`oxideav-sysaudio`、`audio_engine_core`、`fast-audio-resampler`、`lanczos-resampler`、`resample`、`coupler`、`rsynth`） | crates.io 下载量实测 **49 – 7,299**（最低的 `audio_engine_core` 49、`oxideav-sysaudio` 97），无一经过生态验证；同功能都有成熟选择（`rubato` 11.4M、`ebur128` 949k、`biquad` 387k） |
| 三个查无实物的名字（`InfiniteDSP`、`tpt-dsp`、`math_audio_iir_fir`） | crates.io 与 GitHub 均查无对应可用项目；`nih-plug` 确有 2,973★ 但**不在 crates.io**，只能 git 依赖 —— 插件宿主不是当前要做的事 |
| `insta` 快照测试 / `criterion` 基准 | `criterion` 被项目自己的注释拒绝过（"Criterion would be a dependency tree for a handful of numbers"，见 `lib.rs` 的 `bench_util`），本仓库自带的 `#[ignore]` 基准已够；`insta` **尚未采纳** —— 把 60 余条 `TestBackend` 渲染测试快照化是候选，但目前没有量到的问题驱动它 |
| `cargo doc` 的链接告警当 CI 守卫 | **暂不上，等链接本身修干净**：实测 `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` 报 **11 条真实诊断**（6 条"公开文档链到私有项"、4 条未解析链接、1 条冗余显式链接目标），散在 10 个文件（含 `crates/ncm-api`）。`--document-private-items` 只能消掉那 6 条、剩下 5 条照样红，而且它把私有实现发布成公开文档 —— 那是产品决定，不是 lint 修复。同一轮把 **clippy 警告**升成守卫则是可行的（本机 `cargo clippy --workspace --all-targets -- -D warnings` 已 0 诊断） |

**继续自己写的（成熟库确实不提供 / 换了会改契约）**：终端协议兜底判定（kitty/ghostty/sixel/tmux）、
封面圆形 alpha 遮罩、OSC 11 背景亮度、16/256 色量化、本地 DSP（FFT/YIN —— **分析侧**仍自写；输出侧的重采样 / EQ / 响度改用上面三个库，两者目的不同：一个只看不改，一个改），卡拉 OK 逐字符着色、
NCM 协议加密与签名、`:` 小语法解析、缓存淘汰与索引策略、配置版本迁移。

## 性能与占用实测

本仓库自带基准，`cargo test --release --lib -- --ignored --nocapture` 可复现：

> 曲库扫描那一项需要一份真实的音乐目录（默认 `~/Music`，可用 `BOXPIGMA_BENCH_MUSIC` 指定），封面两项需要缓存里已经有封面（先播一首歌），音频链那一项（`chain_bench`）需要一首真实文件（默认取下载缓存里的第一个 MP3，可用 `BOXPIGMA_BENCH_AUDIO` 指定）。缺素材时它们打印一行说明并**跳过**，不会让整条命令失败。

| 基准 | 1.0 实测 |
| --- | --- |
| fft 512 / 1024 / 2048 点 | 4.95 / 10.13 / **21.76 µs** |
| hann 2048 点 | 13.52 µs |
| 分析帧（tap + 频谱 + 音高） | 129.57 µs → 30 fps 占单核 **0.389%** |
| playerbar 整帧（含频谱） | 79.15 µs → 30 fps 占单核 **0.237%** |
| 封面（解码 + 裁方 + 圆形蒙版 + 协议） | 488–583 µs / 首 |
| 封面旋转（旋转 + 套蒙版 + 协议 + 编码，`[playerbar] spinning_cover`） | 966–1009 µs / 角度 → 20 秒一圈、72 个角度（3.6 次/秒）占单核 **0.35%** |

内存占用（同一台机器实测，RSS，0.2 秒粒度采样 24 秒）：

| 场景 | 1.0 | 旧版 0.2.14 |
| --- | --- | --- |
| TUI 主界面 | **14.6 MB** | 14.0 MB |
| 守护进程（`boxpigma -d`） | **13.7 MB** | 13.4 MB |

四种情况的**启动峰值都等于 20 秒后的稳态**：启动不冒尖，之后平台平坦（24 秒窗口内没有增长趋势）。
RSS 大致是**二进制体积 + 约 3 MB**（1.0 的二进制 11.1 MB），换成 `tracing` 的增量在 +0.3~0.6 MB 之间。

1.1.0 复测（同机、同条件对照 `v1.0.0` 产品产物）：RSS **未上升**（TUI 13.0 / 守护 12.4 MB，对 13.4 / 12.2 MB；本轮未登录、无播放，绝对值低于上表）；二进制 **11,383,424 B**（对 11,119,096 B，**+258 KiB / +2.4%**）。基准本轮未取到可比数字（被测的 `dsp.rs` / `playerbar.rs` / `engine.rs` 相对 1.0.0 零改动，封面基准用的是本机缓存里的另一批真实图），故上表不改。

本次复测（2026-09-22，移除多源兜底 + 三处模块拆分之后；1.6.0 发布产物）：同机、同条件，`cargo test --release --lib -- --ignored --nocapture` —— 这条命令在 Linux 上此前**必然失败**，本轮 10/10 通过：fft 512 / 1024 / 2048 点 **6.03 / 10.48 / 23.33 µs**、hann 2048 点 **13.88 µs**、分析帧（tap + 频谱 + 音高）**134.24 µs**（30 fps 占单核 **0.403%**）、playerbar 整帧 **81.34 µs**（**0.244%**）、空闲主页面整帧 **266.06 µs**（播放时分析流约 30 次/秒 ≈ **0.80%** 单核）、封面 **501.73–708.69 µs/首**、封面旋转 **1062.61–1098.19 µs/角度**（3.6 次/秒 ≈ **0.38–0.40%**）；`styled_text` 解析 0.53 µs（复用缓存后 0.07 µs，**7.8×**）；曲库扫描因本机 `~/Music` 无素材跳过。与上表各行相差 **±3~15%**，而**同一台机器上两次运行（一次受并发编译干扰、一次干净）之间的散布本身就有 ±5~10%**（本轮两次的数字都记在上面这行的量级里），且本轮改动没有落在任何被测热路径上（模块搬运、按键类型在事件边界转换、配置在启动时重写），故按**无回退**记。

资源占用（同机、未登录、无播放）：release 二进制 **10,947,504 B**（对 1.5.0 发布产物 12,941,128 B，**−1,993,624 B ≈ −15.4%** —— 兜底 crate 与它依赖的子模块整体删除）；RSS 采样 24 秒 / 0.2 秒粒度：TUI **13.8 MB**、守护进程（`-d`）**12.4 MB**，两者**启动峰值 = 稳态**、窗口内无增长（守护进程与 1.1.0 的 12.4 MB 一致）。

1.6.0 的 **GitHub Release 产物**（从 Release 下载后实测，与本机 `cargo build --release` 对照）：解出来的二进制 **10,977,416 B**，比本机那份 10,947,504 B 多 **+29,912 B / +0.27%**（同一 profile，工具链与构建环境不同），两者都远低于 1.5.0 的 12,941,128 B。发布资产是压缩后的：x86_64-linux **4,830,735 B**、aarch64-linux 4,348,806（另加 625 B 的 `SHA256SUMS`），两个目标合计 **9,179,541 B**。

1.6.2 的 **GitHub Release 产物**（同样从 Release 下载后实测）：`boxpigma-x86_64-unknown-linux-gnu.tar.gz` **4,919,102 B**、`boxpigma-aarch64-unknown-linux-gnu.tar.gz` **4,430,111 B**、`SHA256SUMS` **215 B**；同一工具链下本机 `cargo build --release` 为 **11,212,656 B**。**资产只有这两个目标** —— 这一版起 Windows / macOS 的目标与产物都已删除，所以资产数从 7 降到 3。

基准输出现在也随 CI 走：`.github/workflows/ci.yml` 的 `bench` job 在 ubuntu runner 上跑同一条命令，把**原始输出**上传成 artifact（`bench-output`）并写进 job 摘要 —— 共享 runner 的数字只作趋势：同一镜像上两次相邻运行之间，同一项就能差 **25%（hann 2048：34.76 → 26.07 µs）**、封面之外的绘制项差 9–17%（空闲整帧 542.84 → 472.72、playerbar 整帧 200.30 → 218.13），这是把 artifact 当趋势而非口径的实测理由；本页引用的仍是以作者机器为准的那一档。

音频路径（本轮新增基准 `playback::chain::chain_bench`，素材取缓存里的一首真实 MP3：**24 kHz / 2 声道 / 202 秒音频**，设备 48 kHz；两个数字是**两次干净运行的范围**）：解码 **414.60–433.24 µs/音频秒**（实时占单核 **0.04%**）→ 加重采样（24k→48k，样本数翻倍）**902.03–1001.49**（**0.09–0.10%**）→ 再加两段 EQ 与 EBU R128 响度 **1898.94–2007.04**（**0.19–0.20%**）；同长度的内存源空转对照 **44.80–46.23 µs/音频秒**，也就是说这几行里几乎没有"框架税"，都是编解码器与 DSP 本身。**整条音频路径 ≈ 0.19–0.20% 单核**；与分析帧 0.403%、整帧 0.244% 相加，**播放时整机 ≈ 0.84–0.85% 单核**（30 fps）。注意这一项之前**没有量过**：上表只覆盖分析侧与绘制，音频链的开销是本轮补上的账。

二进制体积：整条日志栈换新后 **11.1 MB**（换之前 10.8 MB）。完整变更记录见 [CHANGELOG](./CHANGELOG.md)；**每项框架调整的改前改后、依据与代价（含被否决的候选）见 [FRAMEWORK.md](./FRAMEWORK.md)**。

## 三、代价与收益

| 指标 | 数值 |
| --- | --- |
| 新增 crate | **7 个**（`tracing-appender`、`tracing-log` + `crossbeam-channel`、`crossbeam-utils`、`phf`、`phf_shared`、`symlink`），lock 增加 80 行 |
| 本来就在 lock 里、只是改为直接依赖 | `tracing`、`tracing-subscriber`（经 color-eyre 的 `tracing-error`）、`palette`、`base64` |
| 内存占用 | TUI **14.6 MB** / 守护进程 **13.7 MB**（旧版 14.0 / 13.4 MB）—— 增量 **+0.3~0.6 MB**，与日志栈换新同量级，未逐项归因；四种情况的启动峰值 = 稳态，24 秒内不增长 |
| 二进制体积 | 10,800,688 B → **11,145,688 B**（**+345,000 B ≈ +337 KiB / +3.2%**），含整条日志栈 |
| 性能 | **无回退**：分析帧 129.57 µs（30 fps 占单核 0.389%）、playerbar 整帧 79.15 µs（0.237%）、fft 2048 点 21.76 µs、封面 488–583 µs/首 —— 可由 `cargo test --release --lib -- --ignored --nocapture` 复现 |
| 1.1.0 复测 | RSS 未上升（TUI 13.0 / 守护 12.4 MB，同条件对照 1.0.0 的 13.4 / 12.2）；二进制 11,383,424 B（+258 KiB / +2.4%）；**基准在无并发负载下逐项复现上表**：fft 2048 点 20.73 µs、分析帧 124.55 µs、playerbar 整帧 77.98 µs（此前一次偏高 1.6~2.6× 的记录，事后查明是同机并发编译/多实例造成的污染，不是回退） |
| 整帧开销（补充） | 此前只记了 playerbar 整帧（77.98 µs），漏了顶栏/导航/列表：**空闲主页面一帧 230.66 µs**，播放时整帧 + 分析帧 ≈ 355 µs ≈ **1.1% 单核**（30 fps） |
| 自写代码 | 净减少：日志实现整体删除、颜色数学三处合并为一处、`Block` 铺底与手绘字形被替换 |

## 四、同步上游时的含义

结构性调整决定了哪些文件是"我们的地盘"：同步一律 **cherry-pick、不整体合并**，冲突时保留本仓库
版本并确认行为没有退化。逐文件清单与验收命令见 [CONTRIBUTING](./CONTRIBUTING.md) 的
「与上游同步」一节；与上游的差异也按"结构性 / 增量"两分类列在 [README](./README.md)。

**平台**是最大的一处结构性差异：本仓库只构建 Linux。上游的 Windows / macOS 代码（WASAPI 独占输出、命名管道、控制台背景探测、`.zip` 更新与 junction、`install.ps1`）以及全部平台 `#[cfg]` 分支都已删除，所以上游任何平台相关的提交**不挑**；上游若改了发布作业的目标矩阵，冲突时保留本仓库的两个 Linux 目标。

### 已在 1.1.0 之后核过但**不采纳**的性能候选

| 候选 | 为什么不采纳 |
| --- | --- |
| 主循环改成「只在需要时重绘」 | **不需要**：`handle_events` 本身就是阻塞等事件（只有拖动进度条时才 32ms 轮询），空闲时不重绘。曾误以为「每轮都画一次」，读了函数体后撤销 |
| `styled_text` 预解析 / 缓存标记 | **不值得**：实测解析 6 条标题合计 **0.21 µs**，每帧几十次也只有个位数 µs（复用预解析结果只快 5.6×，绝对值无意义）。顺带发现 `static … LazyLock::new` 要求 const 闭包，运行时解析放不进去 |
| 封面编码移出渲染线程（`ratatui-image` 的 `thread` 特性） | **暂不做**：旋转封面实测 976~983 µs/次、3.6 次/秒 = **0.35% 单核**，每次只占一帧的 3%；真出现卡顿再上 |
| kitty 图像 id 复用（先传 N 个角度、之后只重摆放） | **留作后手**：`ratatui-image` 已按 id 复用（同一张图只传一次），代价在「新角度=新图=重编码」；预传 72 个角度要数 MB 驻留内存，收益与代价都不划算 |
| 本地曲库扫描并行化（`std::thread::scope` + `available_parallelism`，纯标准库） | **不值得**：实测 300 首 **1534 µs = 5.1 µs/首**（`lofty` 只读标签帧、页缓存热），折合 5000 首约 **26 ms**，并行省下的是一次扫描里的几十毫秒；冷盘时的瓶颈是磁盘寻道而非 CPU，多线程只会让寻道更糟。扫描结果另有磁盘缓存，重复进入本地音乐不会重扫。基准保留：`playback::scan::scan_bench` |
