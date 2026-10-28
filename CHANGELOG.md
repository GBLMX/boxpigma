## [unreleased]

### 🚀 Features

- *(update)* **更新通道与原生清单对齐 Ferriso Launcher**：`boxpigma update` 的 `latest` 不再只靠 GitHub latest 跳转，而是读取 `boxpigma-updates.json`（stable/beta/alpha；镜像 `https://updates.bionickernel.com`，GitHub 最新 release 兜底）。清单提供版本说明、发布时间、平台资产和 SHA-256；`[update] channel`、`BOXPIGMA_UPDATE_CHANNEL`、`--channel` 三级覆盖，`BOXPIGMA_UPDATE_URL` / `--manifest` 可指定完整清单。固定 `--version` 仍走 tag 资产。release workflow 同时 stage 版本目录、部署到 Aliyun 镜像，并由 `boxpigma-update-sync` 生成三个通道 feed。
- *(framework)* **Ferriso 0.1.0 → 0.3.1**：`ferriso-core` / `ferriso-gui` / `ferriso-tui` 从 `e021c99` 升级到 `55d17458`。后端计时器迁移到 `Subscription::interval` + `Internal::Tick`，不再使用已弃用的 `tick_interval` / `tick` 兼容钩子；GUI 的 `Settings` 补上 `exit_on_close_request`。同时获得 0.3.1 的 keyed-command 批次去重修复（同批次同 key 只启动最后一条）。
- *(framework)* **Ferriso `55d17458` → `8a2e201`**：升级到移除废弃
  `App::tick_interval` / `App::tick` 兼容层的版本；boxpigma 已经使用
  `Subscription::interval`，不再依赖 actor 自有计时器。
- *(framework)* **Ferriso `8a2e201` → `6791fa7` → `1924a60`**：依赖升级到 Ferriso 新增 `CoreContext`、`Sidecars`、`run_with_app_services`、`request_exit` 与 `ferriso-tui::art` 的版本；boxpigma 当前桥接行为不变，后续可把 IPC/MPRIS 迁移到统一的 sidecar 生命周期。
- *(tui)* **交互式终端迁到共享 actor 后端**：`boxpigma` 不再启动旧的 `App` 渲染循环，而是与 `boxpigma-gui` 共用 `boxpigma::backend`，由 `ferriso-tui` 管理 Tokio runtime、actor、终端与鼠标捕获。终端持有的是 `Arc<View>` 快照，键盘/鼠标只产生语义 `Message`；导航、歌曲/歌单/排行榜/歌手/热搜、分页加载、歌手三栏、队列、歌词、QR/密码/短信登录、签到、设置、命令模式、封面/二维码、点击 seek/音量与瞬时提示都走同一条消息路径。迁移时固定到 `e021c99`（`ferriso-core` / `ferriso-gui` / `ferriso-tui`）。后端补了 `SetTheme` / `SetQuality` / `Search` / 登录字段追加与退格消息，避免快速输入时用旧快照覆盖整…
- *(tui)* **私人 FM 接入 actor**：复用 `ApiService::personal_fm` / `fm_trash`，新增 `OpenPersonalFm` / `TrashFm` 消息与 `:fm` / `:fmtrash` 命令；`:fm` 拉取后立即开播首曲并展示列表，当前曲目可拉黑。
- *(tui)* **actor IPC / MPRIS 桥接**：交互式 TUI 现在启动 Unix socket，`status` / `list` / `search` / `msg` 与 MPRIS 都读写同一份 `View` 快照；`switch-list` 走 actor 的端点解析，不再只属于守护进程。
- *(tui)* **`[keys]` 真正作用于 actor TUI**：命令表派生的 `Keymap` 已在终端输入路径生效，单键与多键序列、`ctrl+` / `alt+`、解绑都与守护进程一致。
- *(tui)* **旧命令兼容层**：恢复 `:simi` / `:simiplaylist` / `:signin` / `:sms` / `:smslogin` / `:saveonplay` / `:pane` 与相对音量（`:volume +5` / `-10`）；`:lyrics` 恢复为歌词样式切换，页面改用 `:lyrics-page`。
- *(tui)* **歌词样式与 ASCII 字形**：actor TUI 支持 `window` / `one_line` / `ktv` / `flow` / `plain`；`[symbols]` 的 ASCII 预设现在作用于导航、音量、频谱、spinner 与译文前缀。
- *(tui)* **高级美术效果**：播放条使用 aurora 动态背景与 pulse 边框，当前歌词使用 shimmer 扫光；全部由现有 tick 驱动，颜色从主题调色板推导。

### 🐛 Bug Fixes
- *(tui)* **歌词 shimmer 按显示宽度推进**：此前按 `char` 计数，CJK 等双宽字符会把扫光速度减半；现在按终端显示列推进，宽字符/组合字符不会再错位。

- *(cache)* **内容/歌词/封面缓存改为原子写**：下载内容、歌词和封面此前用 `fs::write` 原地覆盖，进程崩溃或其他实例读取时可能看到空文件/半截 JSON。现在统一走 `utils::fs::write_atomic`（同目录临时文件 + `fsync` + `rename`），并复用已有测试。此修复对应上游 `4089ecd` 的缓存硬化。
- *(cache)* **内容缓存时间戳回退不再下溢**：如果墙钟被调回，`now - cached_at` 会在 debug 构建中 panic；现在未来时间戳直接按过期处理。
- *(cli)* **`status -L` 输出歌曲 id 和队列 key**：文本行尾增加歌曲 id，可直接喂给 `boxpigma msg play <id>`；`QueueSnapshot` 增加带 `serde(default)` 的 `key` 字段，旧守护进程仍可被解析。该输出复用 `msg list` 的渲染器，两个入口保持一致。

- *(update)* **`--mirror` 覆盖清单资产 URL**：`latest` 通过清单解析出的 GitHub
  资产地址现在会做前缀替换并规范到 `GBLMX/boxpigma`，更新客户端改用 HTTP/1.1
  （对代理镜像更稳）；下载失败时错误信息会带上底层 reqwest 原因，不再只显示
  “下载中…失败”。

## [1.7.0] - 2026-10-21

### 🚀 Features

- *(gui)* **桌面界面 `boxpigma-gui`（iced 0.14）**：在共享后端上补齐配置导航树、歌单/专辑/电台/排行榜详情、歌手页、单曲/歌单/歌手/专辑四类搜索、分页加载、播放队列、歌词与 MV 海报、二维码/密码/短信登录与签到、喜欢/下一首/上传云盘、主题/音质/边听边存/通知/译文/频谱/音高设置，以及带封面、进度拖动、音量、实时频谱和播放模式的底部播放条；切歌/出错按 `[notify]` 调 `notify-send`。窗口只渲染快照，后端由 `boxpigma::backend` 的 actor 独占；配置、cookie、封面/歌词/队列缓存与 TUI/守护进程共用。快捷键只在带 `Ctrl` 时生效（`Ctrl+空格` 播放/暂停、`Ctrl+←/→` 切歌、`Ctrl+L` 喜欢、`Ctrl+M` 模式）。

### 🐛 Bug Fixes

- *(terminal)* **tuios 窗格里封面画不出来，启动还要白等两秒**：tuios（终端工作区管理器）和 herdr 一样把窗格里的终端**自己仿真**了 —— 它只解析 kitty 图形协议、再自己画到外层终端 —— 而窗格里的进程带着的是**外层**终端的变量（`TERM_PROGRAM`、`KITTY_PID` 都是外层的）。封面协议本应由「问终端」定，可 tuios 的回答是私有形式 `CSI ? 0 n`（标准是 `CSI 0 n`），`ratatui-image` 的回答解析器只认后者，于是 `Picker::from_query_stdio()` **永远不返回**：查询线程停在 `read()` 里直到进程退出，boxpigma 那 2 秒预算到点只换来一句 WARN 和半块字符画。现在 tuios 窗格内**不问**：据 `TUIOS_ENV`（tuios 在每个窗格都设）先于外层终端那条修正判定，一律用 kitty —— 与 1.6.1 的 herdr 同一套处理，因为窗格里的变量同样会骗过 `TERM_PROGRAM` 修正（daemon 从 WezTerm 起，窗格里就会读出 WezTerm）；格子尺寸改从窗格自己的 pty 读，tuios 把**客户端**格子的像素尺寸写进 `TIOCGWINSZ` 的像素字段（实测 0.8.1：18x6 格、客户端格子 10x22 px 的窗格报 180x132 px，与 daemon 日志里该客户端的 `cell=10x22 pixels` 一致），所以封面按真实格子缩放，而不是库的 10x20 猜测；pixel 字段要等客户端把窗格布局完才写进去（实测新建窗格 t=0 报 0x0、t=0.34s 才报 10x22），读不到时会重读，最多等 0.5 秒

## [1.6.3] - 2026-09-29

### 🐛 Bug Fixes

- *(update)* **取不到 `SHA256SUMS` 时不再默认安装**：`boxpigma update` 与 `install.sh` 此前都只打印一句「这次没有校验就安装」然后照装 —— 自更新器静默放弃完整性校验，而且恰好发生在最需要它的场景里（`github.com` 连不上、只能走镜像时，`SHA256SUMS` 最可能取不到）。现在两者默认**拒绝安装**并给出两条出路：`--checksums <URL|本地文件>` 自己指定一份，或 `--insecure` 明确放行；`--dry-run` 仍只报告、不中止（真跑会被拒这件事本身就是计划的一部分）。两个参数在两个入口同名。

### 📚 Documentation

- *(readme)* `--mirror` 是**前缀替换**这件事写清楚：自建镜像给 `https://mirror.example`，ghproxy 这类把完整原 URL 当路径的代理要给 `https://proxy.example/https://github.com` —— 只给主机名会拼出 `…/GBLMX/pigma/…` 而 404（实测）

## [1.6.2] - 2026-09-29

### 🐛 Bug Fixes

- *(playback)* **本地音乐按曲名排，唱片的顺序会被打乱**：`scan_local_music` 收尾那句 `songs.sort_by(|a, b| a.name.cmp(&b.name))` 把音轨号整个丢掉了 —— 一张 13 轨、`TRACKNUMBER=01..13` 的碟进队列后是 `Airport Arrival` 排在 `Airport Take Off` 前面，中文曲名再按 Unicode 码位一路 `再见(518D) → 十七岁(5341) → 心乱飞(5FC3) → … → 飞机场(98DE)` 排下去，碟序一轨不剩。现在按**专辑 → 碟号 → 音轨号 → 路径**排：碟号与音轨号取自 `ItemKey::DiscNumber` / `TrackNumber`（`01`、`3/13`、`1 of 2` 这类写法都认，解析不出来的退到下一级），没有音轨标签的曲库退化为按路径排 —— 原来的兜底其实是 `read_dir` 的顺序，同一个目录两次扫描都可以不一样
- *(ipc)* **切到别的队列之后 `msg list` 还在吐上一个队列**：队列版本号（`PlaylistQueue::version`）每次变更自增，`App::last_queue_version` 就靠「版本变没变」决定要不要重建 IPC 队列快照 —— 而切换端点是 `Engine::load_songs`，它把整个 `PlaylistQueue` **换掉**，新实例的计数器又从 0 开始，于是新队列的版本可能正好是消费者手里那个（0 == 0）：快照被判成「没变」，`boxpigma msg list` 继续打印旧队列，直到某次无关的变更把版本推过去。现在版本号由一个进程级的 `AtomicU64`（`playback::queue::NEXT_VERSION`）发放，任何一次替换或变更都拿到一个此前没被用过的号

- *(ncm-api)* **两个实例同时写 `cookies.json` 会互相丢 cookie，而且谁登出、谁登录对方都要重启才看得见**：上游 akirco/pigma 的 `1da4003` 把「整份覆盖 + `truncate` 直写」换成「读盘 → 只覆盖自己的改动 → 临时文件 + `rename`」，读方再看不到半截文件（实测 5.8% 的读是空的/撕裂 → 0/21164），但它仍假设「内存里的那份就是权威」：自己启动之后文件里发生的事一概不知 —— 文件里 `MUSIC_U` 已经删掉，守护进程还拿着旧 token 发请求；TUI 刚登录，已经在跑的守护进程仍是匿名的；两个同时启动的进程各生成一个 `deviceId`，文件里只留一个、另一个继续发自己的。而且「读盘 → `rename`」之间没有互斥，对方的 `rename` 会把刚写下的键盖掉、自己的 `dirty` 又已清空，于是那两个键再也不会被写回：两个实例各写 60 个键、交替 flush 的压测里**永久丢 11/120**。根因是 v1 格式分不清「别人删了」和「自己没写过」，所以现在把文件当**登记表**、实例只是它的**缓存**：发请求、查登录态、写盘之前先看一眼文件（250ms 内至多一次；文件才几百字节，一次读可以忽略），凡不是自己改过的项一律跟随文件 —— 别人写的值采纳、别人删掉的项就地丢弃、绝不会被下一次 flush 复活；`deviceId` 一旦落盘即归文件所有（自己刚生成的让位），同时启动的两个进程因此无需重启就收敛。写盘在跨进程锁（`cookies.lock`：`create_new` 原子创建、无新依赖、5 秒陈旧锁自动打破、拿不到就降级为无锁合并）内「重读 → 只应用自己的改动 → 临时文件 + `rename`」，同一个压测 **0/120**（连跑 10 次全 0；不给这把锁是 52–60/120），无事可写时连读都省掉；文件读不出来（半截/被改坏）时先改名成 `cookies.json.corrupt-<时间戳>` 留档再重写，而不是把它清掉。用例：`concurrent_instances_do_not_lose_keys`、`external_removal_is_not_rolled_back`（这两个都先验证过「撤掉修复即失败」）、`another_instances_login_is_adopted`、`another_instances_logout_is_adopted`、`a_deleted_file_is_restored_from_memory`、`an_unreadable_file_is_kept_aside`，`flush_does_not_clobber_another_instances_cookies` 也随之改成「B 开机即可见文件」并断言两边互相同意 `deviceId`

- *(playback)* **频谱的短窗让 radix-2 FFT 越界 panic**：`Spectrum::analyze` 把 `mono.len().min(WINDOW)` 直接当作变换长度，可那个长度跟着音频环形缓冲的填充走 —— 一首歌刚开始时环里只有几个 512 样本的块，立体声源会拿到 **768**（既不是 2 的幂，也不在 `dsp::transform` 的比特反转索引范围内），`data[start + k + len / 2]` 于是索引越过 `n`，进程直接 panic；5.1 等多声道源第一时间就触发（约 85 样本的窗口）。现在不足 `WINDOW` 的窗口一律**补零到 `WINDOW`** 再变换：长度恒为 2 的幂，band 边界（本就按 `WINDOW` 算）不再错位，幅度归一化仍除以真实样本数所以数值不变；`dsp::fft` 那句"长度须为 2 的幂"由文档提升为 `debug_assert`。回归测试扫过 `2..=WINDOW` 的每一种长度（撤掉修复即失败），并钉住短窗仍落在 440 Hz 该在的 band 上
- *(playback)* **本地音乐扫描撞上符号链接环会把同一首歌重复进队列**：`scan_local_dir` 用 `path.is_dir()` 递归，而它会**跟随符号链接** —— `~/Music/loop -> ~/Music`（或两个互相链接的专辑目录）被逐层重走：内核的符号链接上限（40）让它停在第 41 层，于是同一首歌在队列里出现 **41 次**，不崩溃、只悄悄错。现在按目录的**真实路径**（`canonicalize`）去重，每个物理目录只走一次；从别处链接进来的专辑目录照样会走到（那是常见布局，不是错误），只是不再重复。两个 `#[cfg(unix)]` 测试分别钉住"环不重复"与"链接目录只走一次"
- *(ipc)* **锁被 poison 时 IPC 会 panic**：`ipc/mod.rs` 的四处 `snapshot.lock().unwrap()` / `queue.lock().unwrap()` 是全仓唯一还把"别的线程 panic 过"当成自己 panic 的取锁点（`state/avatar`、`state/mv`、`cache/*` 一律 `unwrap_or_else(into_inner)`）。一次无关线程的 panic 之后，`boxpigma status` 的每一次轮询都会把整个进程带走。现在统一走 `utils::sync`
- *(ncm-api)* **四个变更类接口把业务失败当成功返回**：`parse_msg` **从不报错**（永远是 `Ok(Msg { code, msg })`），所以只调它的接口无论服务端回什么码都返回成功。`album_like` / `song_list_like` / `fm_trash` / `recommend_song_dislike` 现在与同模块的 `like`、`new_albums`、`album_sublist` 一样先 `check_api_code` 再解析；`auth` 的登录/二维码状态、`daily_task`、`homepage`、`report_play` **不动** —— 它们的返回类型本就承载服务端码，由调用方判断
- *(config,cache)* **配置与缓存索引是原地覆盖写的**：`Config::save`/`load_from` 与 `cache::index::write_index` 都用 `fs::write` **原地截断** —— 写到一半断电，或另一个进程/下一次启动正好在那一刻读，看到的就是空文件或半截 JSON（配置与整个下载索引一起没了）。`cookie.rs` 早在并发那次修复里换成了"临时文件 + `rename`"，应用侧这两处一直没跟上；现在统一走 `utils::fs::write_atomic`（同目录唯一临时文件 → `sync_all` → `rename`；跨文件系统不原子，所以临时文件必须同目录）。两个单测钉住"整体替换"与"不留临时文件"

### 🚜 Refactor

- *(tui)* **设置触发器收口到命令表**：`translation` 现在是 `COMMANDS` 的一行（`y` 由 keymap 分发），不再有 `input/main.rs` 里绕过命令表的 `y` 分支；删除未使用的 `CommandEvent::ToggleBordered`，边框只保留 `border` 命令一条路径。设置页新增一致性测试：每一行都必须对应一个可被调色板执行的命令，`y`/`:translation`/设置页共用同一个 `ExCommand` 处理器。

- *(backend)* **按 Ferriso 的 actor 模型抽出 GUI 后端**：新增 `boxpigma::backend`，直接依赖 `ferriso-core` 与 `ferriso-gui`（MIT，Git 上游 2799496）：core 提供 `ActorApp` / `Command` / `CoreHandle` / `Snapshots` / `spawn`，gui 提供 iced adapter（`Renderer` / `Message::Local` / `run_with_app`），一个任务独占 `ApiService`、`PlaybackEngine`、缓存、导航、登录和设置；iced 只持有最新 `Arc<View>` 快照并发送语义 `Message`，不再自己建 runtime、拉接口或操作队列。Tokio runtime、actor 启停和窗口生命周期改由 Ferriso 管理；GUI 由此补齐了配置导航树、歌单/专辑/电台/排行榜详情、歌手页、四类搜索、分页加载、队列切歌/清空、二维码/密码/短信登录与签到、喜欢/加到下一首/上传云盘、主题/音质/边听边存/通知/歌词译文设置；`Backend` 的 tick 统一消费播放事件与登录轮询，关闭窗口时仍走保存队列和 cookie 的 shutdown。

- *(utils)* **"取锁"与"原子写"各收敛到一处**：取锁的 poison 决策此前有四种写法（`unwrap`、`unwrap_or_else(into_inner)`、`if let Ok` 静默跳过、`map_err` 转 `io::Error`），其中 `ipc` 的 `unwrap` 会在别的线程 panic 后把进程带走；原子写则只有 `cookie.rs` 一处。现在 `utils::sync::{mutex,read,write}` 收拢了 `ipc`、`cache/audio`、`cache/index`、`state/avatar`、`state/mv` 的十余处取锁（"其他线程的 panic 与这份数据无关，取走它"只写一遍），`utils::fs::write_atomic` 把"临时文件 + rename"从一个地方供给 `config` 与 `cache/index`；`playback::player` 把 poison 转成 `io::Error` 的刻意例外保留，并在 `utils::sync` 的模块文档里写明为什么它是例外

- **[breaking] 只保留 Linux，其余平台整体删除**：Windows / macOS 的支持连同它们的代码一起走 —— WASAPI 独占输出整块（`src/playback/exclusive.rs` 599 行、`[audio] exclusive`、`wasapi` 依赖），IPC 的命名管道传输（`ipc/transport/windows.rs`），Windows 控制台背景探测（`terminal/background/windows.rs`），自更新里的 `.zip` 解包与 `mklink /J` junction 分支，`install.ps1`（570 行），发布矩阵（六个目标 → `x86_64` 与 `aarch64` 两个 Linux gnu 目标）与 CI 的三平台矩阵，以及**全部** `#[cfg(unix)]` / `#[cfg(windows)]` / `cfg!(windows)` 分支（这些分支现在无条件编译）。`FreeBSD`/`NetBSD`/`OpenBSD`/`DragonFly` 也一并从 cpal 的 host 选择里消失。**刻意保留 `#[cfg(all(target_os = "linux", target_env = "gnu"))]`**（`engine.rs` / `player.rs` / `source.rs` 的 `[HEAP]` 诊断）：它编码的是真实的 glibc/ALSA 约束（读 `/proc/self/status` + `libc::malloc_trim`），不是可移植性开关 —— 它是唯一一个在 Linux 上仍可能为假的门。发布资产仍是 `boxpigma-<triple>.tar.gz` 与裸名 `SHA256SUMS`，`install.sh` 的 `awk` 查找与 `src/update.rs` 的解析契约逐字节未动（这是最容易静默弄坏的一条）。代价：随文件删除的 3 个单测（`a_float_device_gets_the_samples_unchanged`、`a_sixteen_bit_device_gets_clamped_pcm`、`the_tracks_rate_is_offered_before_the_devices_own`）测的是只为 WASAPI 设备服务的编码/协商逻辑，其被测对象已不存在。同一工具链前后对拍：二进制 **−7,896 B（−0.07%）**（11,220,552 → 11,212,656 B）—— 删掉的 Windows 代码本来就不进 Linux 构建，只有 `exclusive.rs` 的平台中立辅助逻辑此前会被编进去


## [1.6.1] - 2026-09-24

### 🐛 Bug Fixes

- *(terminal)* **在 herdr 的窗格里封面一张都画不出来**：herdr（终端工作区管理器）把窗格里的终端**自己仿真**了 —— 它只解析 kitty 图形协议、再自己把图片画到外层终端 —— 而窗格里的进程带着的是**外层**终端的 `TERM_PROGRAM`（这里是 `WezTerm`），于是 `auto` 命中「WezTerm 改用 iTerm2 协议」那条修正，把 `OSC 1337 File=` 送给从不解析它的 herdr，图片被静默丢掉。现在窗格内一律用 kitty 协议：据 `HERDR_ENV`（herdr 在每个窗格都设）判定，且先于外层终端那条修正；`[playerbar] image_protocol` 仍可写死覆盖。实测（外层 WezTerm 20260716 / herdr 0.9.1，无头 KWin 截图）：同一张红色测图，kitty 协议画得出来（8800 像素），`OSC 1337` 什么都不显示
- *(cli)* **`boxpigma update --check` 也走镜像**：`--check` 是「先看一眼有没有新版」用的命令，可它把主机写死成 `HOST`（github.com）—— 对最需要镜像的人（根本连不上 github.com）来说，这条命令永远只会失败，而 `--mirror` 明明已经解析出了主机。现在 release 查询与资产探测都走它；顺带把两处把 Windows 分隔符写进格式串的路径输出换成 `Path::join`（非 Windows 上原本打出 `releases\1.5.0-…\boxpigma` 这种混着反斜杠的路径）

## [1.6.0] - 2026-09-22

### 🚀 Features

- *(ipc)* **`boxpigma msg` 新增两个动作：`seek` 与 `clear`**，语义直接复用命令层（`:seek` / `:clear`）而不是另写一份 —— 跳转吃 `+15` / `-30` / `50%` / `90` 四种写法，清空队列等价于 `:clear`。seek 的**值语法**抽成一份共享解析（`playback::seek::parse_seek`），`:seek` 与 CLI 共用，于是 `msg seek abc` **在客户端就被拒绝**：此前它会被送到实例里、只弹一条调用方看不见的提示，而动作回包照样是 `{"ok":true}` —— 与 `msg play abc` / `msg volume 150` 的行为对齐；顺带把 `f64` 能解析的 `nan`/`inf` 挡在门外（NaN 会污染进度，之后每一帧都受影响）。两个动作都出现在 `capabilities`、`msg --help` 与补全里（同一张表生成）
- *(systemd)* **systemd 用户单元**（`systemd/boxpigma.service`）：README 的 Plan 里缺的这一项。`Type=simple`、`Restart=on-failure`，`KillSignal=SIGTERM` 沿用守护进程自己的保存逻辑，所以 `systemctl --user stop` 与注销登录都不丢进度；`ExecStart` 默认对应安装脚本的布局，README 写了另外两种安装怎么改，以及 `loginctl enable-linger` 让其未登录也常驻
- *(config)* **配置迁移改为"就地编辑用户自己的文档"**：迁移现在只在 schema 要求的地方动用户的文件 —— 删掉新 schema 已经没有的键、就地改写 `config_version`（沿用该行原有的行尾注释与间距），值、顺序、缩进与**手写注释逐字保留**。判据不另造键表：某个键还算不算 schema 的，交给 `Config` 自己的 `Deserialize` 回答（四种形状的探针值全被忽略即为已删除），迁移后的文本还会被回读成 `Config`，于是"跑的就是文件现在说的"

### 🐛 Bug Fixes

- *(ui)* **设置页的 `Esc` / `Tab` / 鼠标**：三处都缺——
  - `Esc` 关不掉：`Esc` 的页面特例从主键位表移走后，歌手页补上了自己的层而**设置页漏了**，于是落到「面包屑返回」——而设置页不在面包屑栈里，返回无事发生。现在它自己的层里处理 `Esc`（页面切换而非 restore）。回归测试先验证过：撤掉修复即失败（`left: Settings / right: Main`）
  - `Tab` 无意义：以前落到全局的「切换导航区块」，动的是页面**背后**的侧栏。现在 `Tab`/`⇧Tab` 在「分组列 ↔ 条目列」之间切换焦点，`↑↓` 跟随焦点走（分组列上走分组、条目列上走条目），分组列上 `→`/`Enter` 进入该组条目；两列各自显示自己的选中态（当前焦点列用强调色加粗，另一列只留强调色）
  - 鼠标完全不响应：页面现在在绘制时登记**命中区**（分组列与条目列各一行一个），主分发按页路由点击与滚轮——点分组跳到该组、点条目选中它、**在同一条目上再点一次即切换该项**、滚轮移动光标（与其它列表一致，走完会绕回）。命中区是「当前显示的那一组」的行，所以点击位置要经该组换算回全表下标（这一条是测试抓出来的）

- *(config)* **`config.example.toml` 的 4 个歌词键其实从来没生效**：`lyric_gradient` / `lyric_style` / `lyric_ktv_color` / `lyric_translation` 写在 `[notify]` 表头之后 —— TOML 里它们因此属于那张表，而 `NotifyConfig` 没有这几个字段，serde 一直静默忽略。搬到第一个表头之前（顶层区）即修好
- *(terminal)* **Windows 控制台探测的两个条目对父模块可见**：拆模块时漏了 `pub(crate)`，非 Windows 构建因此报 unused 告警

### 🚜 Refactor

- **[breaking] 移除多源兜底（`sonar` / `y7dl`），只保留网易云与本地**：删掉两个子模块与它们的 crate（含 `.gitmodules`、workspace 成员、依赖与 examples），播放链收敛为 **NCM → 云盘**（一次重试与"云盘未命中时报 NCM 错误"的优先级逐字保留），移除第三方搜索（`SearchProvider`、搜索栏 provider 段与 `Tab` 切换、第三方搜索队列），缓存里的 `thirdparty` 标记与 `thirdparty_source.json` 取消（磁盘上的旧条目与旧队列 id 降级为忽略），配置 v1 → v2 删除 `[source_fallback]` 与 `proxy_target`（`normal` 迁移为直连，`reversed`/`both` 保留 `proxy`）。**已发布的消费者不受影响**：`msg search` 的 `source` 字段留在契约里（恒为 `netease`），本地文件 id 的掩码保留（id 已写进磁盘队列）
- *(terminal,ipc,playback)* **按主题拆开终端 / IPC / 播放设备三个"万能模块"**：`utils/terminal.rs`(1483 行) 拆成能力探测 / 颜色 / 背景 / 图像等，`ipc` 与 `playback` 的同类拆分跟上；纯搬运，行为不变
- *(input,state,ui)* **按键类型出栈，crossterm 只留在事件边界**：应用自有 `key::{KeyPress, KeyCode, Modifiers}`，翻译只发生在事件边界，于是键位表与各处分发不再依赖终端库的类型
- *(ui)* **`ui.rs` 的测试仪表拆出**：`shots` / `contrast_audit` / `theme_background` / `frame_bench` 各成一个文件，`ui.rs` 1003 → 444 行只留外壳
- *(config)* **配置迁移改成"启动即重写"**：`config_version` 升级以前只改内存、等下一次 `save()` 才落盘，于是被新 schema 删掉的键一直躺在用户的文件里 —— 现在迁移在备份之后当场升级**加载时那个文件**（不是 `save()` 自己的目录），并沿用空文档保护；已是当前版本、来自更新版本、解析失败的文件都不动

### 🧪 Testing

- *(playback)* **音频链基准 `chain_bench`**：解码 → 重采样 → EQ + EBU R128 逐段量出开销（解码 **414.60–433.24 µs/音频秒**、加重采样 **902.03–1001.49**、再加 EQ 与响度 **1898.94–2007.04**，整条 ≈ **0.19–0.20% 单核**；同长度内存源对照 44.80–46.23 说明几乎没有"框架税"）
- *(ipc)* **`ipc` 新增文档与动作表对齐的守卫**：每个已发布动作都必须在 README 与 SKILLS 里留下痕迹（今天正是这条守卫对账出 README 缺 `:clear` 与 `msg capabilities` 两行）
- *(playback)* **Windows 独占输出测试加平台门禁**：它此前只有 `#[ignore]`、没有 `#[cfg(windows)]`，于是 `cargo test --release --lib -- --ignored` 这条"用来复现性能数字"的命令在 Linux 上**必然失败**

### 📚 Documentation

- *(page)* 安装说明补上版本化布局（`releases/` + `current` + `install.lock`）与 `--rollback`
- README / SKILLS：`msg seek` / `msg clear` 的用法与示例、`msg capabilities` 补进 msg 表、Plan 勾选
- FRAMEWORK：记录音频路径实测，以及配置迁移两轮调整的改前改后、依据与代价

### 🎨 Styling

- *(terminal)* 三个终端模块的 `use` 块按 nightly rustfmt 收拢；`config` 按 clippy 合并嵌套 `if`；迁移那批新代码按 rustfmt 收拢

### ⚙️ Miscellaneous Tasks

- *(ci)* **CI 把 clippy 警告当守卫**：clippy job 一直在"报告"（只有退出码非 0 才判红，而 clippy 对 warning 默认退出 0），现在 `args` 末尾加 `-- -D warnings`；同时清掉它立刻拦下的 5 条 —— 全在 `playback/exclusive.rs`，该文件的实体是 `#[cfg(windows)]` 的 WASAPI 实现，纯逻辑在 Linux 上"看起来死"，改用带 reason 的 `expect(dead_code)` 精确门禁（删掉等于弄坏 Windows 独占输出）。本地命令与 CI 对齐（README / CONTRIBUTING）
- *(pages)* Pages workflow 自行开启 Pages（`enablement: true`），省掉仓库设置里的手工一步
- *(pkg)* `PKGBUILD` 的版本与校验和升到已发布的 1.5.0

## [1.3.0] - 2026-09-21


### 🚀 Features

- *(ui)* **弹层的边框/标题/页脚也读主题**：`[popup]` 一节此前只有默认值没有读取者；现在 `CornerBlock` 能按样式给标题、并能单独给边框上色（此前所有面的边框色都只能由主题的 `border` 一个键决定），help / `:messages` / `:tasks` 三个弹层都用上了
- *(ui)* **图标与标记收进 `[symbols]`**：弹层与歌手页标题的箭头（`► … ◄`，新助手 `bordered_title` 一处收口）、命令面板的 `▸`/`▶`、通知分级标记（`·`/`!`/`✗`）、任务状态标记（`…`/`✓`/`✗`）此前都是散在各处的字面量——终端画不出来也没法换。现在它们和字形预设（`nerd`/`unicode`/`ascii`）一起在 `[symbols]` 里，逐键可覆盖；任务状态也不再自己知道该画什么（`TaskState::marker` 删除，画什么由 UI 从字形表里取）。测试：三档预设下每个字形都非空且占一格、每个预设对每个键都有值
- *(docs)* **主题文件的 `theme.schema.json`**：把主题可写的每个键（顶层色 + `[table]`/`[tabs]`/`[lyrics]`/`[popup]`/`[notify]` 各节 + 样式对象的 `fg`/`bg`/修饰符）写成 JSON Schema，编辑器的 TOML 插件可据此校验与补全；一条测试把 `Theme` 的序列化与 schema 对照，字段一旦漏写进 schema 就红（这正是 Yazi 用 `#:schema` 做的事）

- *(ui)* **主题可按组件细化，值是「样式」而不只是颜色**：此前主题只有 7 个平铺颜色（`bg`/`surface`/`text`/`accent`/`muted`/`border`/`error`），一个控件想知道「选中行该长什么样」只能自己挑一个色，主题也就无法表达「强调色做背景、上面压一个可读前景、再加粗」。现在照 Yazi 的 `theme.toml`（按组件分节、每项是 `{ fg, bg, bold, … }`）与 spotify-player 的「调色板 + 逐组件覆盖」：新增 `[table]`/`[tabs]`/`[lyrics]`/`[popup]`/`[notify]` 五个节，顶层补 `warn`（警示色，此前通知的 warn 级借用了强调色）。**默认值由主题自己的颜色推出**（`selected` 默认 = `on_accent` 压 `accent`，`secondary` = `muted` + dim…），所以旧主题文件一个字不改仍是完整的一套，新文件只写想改的那一项；已接上线并真正生效的表面：列表的**表头与选中行**、队列**标签**、**歌词译文**、**通知分级**（toast 与 `:messages`）。颜色名不认识时告警一次并回退，与配置里其它颜色同规


- *(ui)* **键位表支持多键序列**（`[keys]` 里 `spin = "z z"`）：此前值只能是一个字符，现在是一串键——第一个键先按住等后续（`Pressed::Wait`），按不下去就把待定清空并把该键交还给其余按键表（`Pressed::FallThrough`），`Esc` 放弃半截序列；精确匹配优先于更长前缀（`z` 仍按表里的 `navpos` 立刻执行，除非把它也改掉）。`ctrl+`/`alt+` 可写，不需要超时——模态键位里「等下一个键」本身就是语义
- *(ui)* **键位表：键从命令表派生，`[keys]` 可重绑**：每个命令默认的键本来就在命令表里写着（`:help`、命令面板都读它），现在**键盘也读它**——`Keymap::from_config` 由该列派生键位，`input/main.rs` 里那几条手写分支随之删除。`[keys]` 写 `命令名 = "一个字符"` 即换键（旧键随即空出）、写 `""` 即解绑（只留 `:` 行与面板）、名字不认识或不是单键则告警一次并保持原样。同时把**页面自己的按键**收进页面表（`PageSpec::keys`，设置页是第一层）：`↑↓/←→/空格` 的含义由所在页面决定，再不用在全局 keymap 里 `if page == X` 串。测试用**脚本化按键**驱动真实入口：按下 `v` 改到 `:visualizer` 声明的那个配置键、重绑后旧键失效、空格在设置页那一行真的改到它声明的键

- *(ui)* **设置页：全部开关一张表**（`,` / `:settings`）：左侧分组（外观 / 歌词 / 播放条 / 通知 / 缓存）、右侧条目与当前值，`↑↓` 选择 · `←→` 修改 · `空格` 开关 · `Esc` 返回。每行改动用的是**该设置自己的 `:` 命令**，所以既不会出现「页面改了但副作用没生效」（鼠标捕获、光标形状、`random` 主题照旧），也不会长出第二套实现。行里的值从 `config.toml` 的**该键路径**读回来——字段改名或删掉是测试红，而不是留一个空行；行的选项取自命令自己的补全（新增预设无需改页面）。测试钉住：每个键都存在且是值、每条命令都能解析、改一行真的改到它声明的键（往返回原值）、光标能走遍整张表、页面真的把分组 / 条目 / 值画出来

- *(ui)* **`ktv` 歌词样式：单色（蓝）卡拉OK填充**：滚动窗口不变，扫光从渐变换成一种纯色 —— KTV 字幕机「唱到哪盖到哪」的样子。颜色由 `lyric_ktv_color` 定，写主题字段（`accent`／`text`…）或颜色本身（`blue`／`#4da6ff`／ANSI 序号）都行：前者沿用 playerbar 那套 `Theme::field_color`，后者新增 `Theme::resolve_color`，两条都**只在第一次读不出来时告警**（沿用 `report_unknown_field_once`，否则会重演 `unfilled_color_cached = "warning"` 那种每帧一行日志）。顺带把 `:lyrics` 的裸循环从手写 match 改成由 `LyricStyle::ALL` 推导 —— 新样式漏进循环正是这个改动之前的样子（`Tab` 补全与帮助本来就取自 `ALL`）
- *(ui)* **译文与原文一眼分得开，且可用 `y` ／`:translation on|off` 关掉**：译文此前只靠斜体区分，而**多数终端无法倾斜 CJK 字形**，于是中文译文和它上面那行英文长得一模一样，滚动时看不出哪个是原文。现在译文行前有一个标记（`[symbols] translation`，默认 `>`）—— ASCII 是刻意选的：更好看的箭头与制表符属于「宽度歧义」字符，CJK 终端会按两格画，而这个布局按一格算。开关在 `View` 构建处统一过滤，四个会画译文的样式一起生效
- *(ui)* **flow 的流动速度跟着当前这句走**：此前固定 8 秒一圈，与本句快慢无关；现在一句一圈，唱得快的句子颜色也流得快。相位因此不能再由帧计数推导（速率随句变化，必须跨句保持，否则每到新行颜色跳回起点），改由 `ui::draw_lyrics` 按墙钟推进，单帧最多 1/4 圈（卡顿或后台挂起不会把调色板整圈滑走），暂停即冻结

- *(ui)* **进度条样式预设：一种样式一个词**（`progress_style` / `:progress`）：拼出「斜线 + 彩虹渐变」那种观感此前要写三个键（`filled_symbol`、`unfilled_symbol`、`gradient_preset`），而想要的样子通常是已知的。现在 `progress_style` 一句选好：`thick`（默认，与本仓库一直画的一模一样）/ `segment`（斜线分段 + 彩虹渐变）/ `line` / `blocks` / `plain`；三个键退化为覆盖（`filled_symbol`/`unfilled_symbol` 不设置就用样式的）。`gradient_preset` 因此变成**三态**：不设置=跟样式、`""`=强制关闭、写预设名=强制该预设 —— 少了第三态就分不出「没写」和「关掉」，保存配置会把样式的渐变吞掉（回归测试钉住：写回的文件里 `Unset` 不落盘、`Off` 落成 `""`、名字原样）。`:progress` 裸调用轮流切换、`Tab` 补全，与 `:lyrics`/`:layout` 同形；渲染测试断言 `segment` 真的画出 `/` 与多种渐变颜色，而默认样式仍是一种颜色。

- *(ui)* **框内面板可拖拽改尺寸、可折叠开关（外框不动）**：以前顶栏/侧栏/播放条的尺寸是硬编码的（3 / 26 / 5），终端不够宽时侧栏无声消失，也没有任何办法给它多一点地方。现在它们由 `[panes]` 配置：**拖面板内边界**改尺寸（跟随鼠标、松手保存一次）、**双击同一条边界**折叠该面板、再双击原样还原（尺寸保留，对应 Herdr 的 `zoom` / `collapsed_space_keys` 语义）、`Ctrl+↑↓←→` 是同一批边界的键盘等价物（每次 2 格）、`:pane <面板> [on|off|toggle]` 给命令行（`Tab` 补全）。**外框不是可拖边界**：能拖的只有框内的分割线。尺寸写进 `config.toml`（本仓库的持久化约定；Herdr 存 session），并做上下界钳制（顶栏 1–6 行、播放条 3–12 行、侧栏 ≥12 列且内容留 ≥40 列、MV 栏 ≥8 列）——窄终端既不会 panic 也不会把内容挤没（这条是被既有测试逼出来的：50 列时钳制上下界颠倒会直接 panic）。歌词页的 MV 海报栏接进同一套边界（`[panes] mv`，0 仍按海报自动）。

### 🚜 Refactor

- *(ui)* **歌词子系统按「状态 / 选项 / 呈现 / 组件」分层**：`ui/lyrics.rs` 一个 1500 行文件里原来装着四件事 —— 整页状态（当前行缓存、flow 相位）、纯时间学（句长、填充进度）、五种画法、以及与歌词无关的 MV 海报栏。现在：`state/lyrics.rs` 回答「歌在歌词的什么位置」（`LyricsState` 由 `App` 持有、`&mut` 传进页面，即 ratatui 那套 `StatefulWidget` 形状），`config/lyrics.rs` 负责选项装配（`LyricsConfig` + `Config::lyrics_config`，颜色在装配处按主题解析一次），`ui/lyrics/{view,styles,panel}` 各管 View、五种呈现、MV 栏。顺带**删掉了线程局部的当前行缓存和 `AppState` 上两个 flow 字段** —— 改成显式传递的状态对象，于是「增量扫描的提示、seek 回退、换歌重置、暂停不被当成一大帧」这些原来完全没测过的行为现在都有测试。
- *(ui)* **背景决策收敛成一处，`GradientPreset` 会说自己的名字**：`ui::style()` 每帧解析一次「要不要铺主题背景」（见上面的 `paint_background`），结果放进 `BlockStyle::base` —— 整帧、每个 `CornerBlock`、骨架屏都改用它，所以透明与否是一处决定、处处一致，而不是只看整帧那一次。`GradientPreset` 补上 `name()` / `ALL`（此前 ex.rs 为 `:lyricgradient` 维护了一张重复的名字表，注释里也写着「能解析名字但说不出自己」），解析、补全、`ProgressGradient` 的序列化现在都读枚举本身。
- *(ui)* **样式改为表驱动**：`LyricStyle` 的 `name` / `parse` / `describe` 与别名合并成一张 `SPECS` 表，`:lyrics` 的裸循环由 `ALL` 推导；页面侧新增 `PRESENTATIONS` 注册表与 `every_style_has_a_presentation` 测试 —— 以前加一种样式要改 6 处 `match`，漏一处会**静默画成窗口**，现在漏了是测试失败。

### 🐛 Bug Fixes

- *(ui)* **歌词最后一行的两处老毛病：越界 panic 与瞬间填满**：`window`／`flow` 取「当前行的下一行时间」时**直接下标 `lyrics[cur + 1]`**，当前行正好是最后一行就越界 —— release 档 `panic = "abort"`，唱到结尾会把整个进程带走；`one_line` 那边用 `unwrap_or_default()` 拿到 0 时长，进度恒为 1，**最后一行一出现就填满**。两处统一到 `line_duration_ms`：一句唱到下一句开始，最后一句唱到歌尾，歌长未知时退回 4 秒。回归测试在 55s..60s 的歌里把播放位置放到 57.5s，断言最后一行画得出来、且填充边界落在句中（改前这条路径 panic）
- *(input)* **切换导航栏位置后，不该可点的区域不再劫持点击**：鼠标命中区（`nav_hits`）只在绘制它的那一帧有效，但没人清 —— 终端窄于 60 列时侧栏按设计**不绘制**、歌词/队列/歌手页也从不绘制导航，可上一帧的区域留着，而 `handle_click` **先问导航**，于是那些不可见的条目继续吞掉本该给内容的点击。实测（50 列、侧栏隐藏的那一帧）：在内容区坐标上点击，激活的却是导航项「每日推荐」。现在 `ui::draw` 每帧先清空命中区，由真正绘制它的视图重建；回归测试断言「不绘制导航的那一帧不留下任何命中区，且点在原坐标不再触达导航」。
- *(input)* **滚轮在导航栏上会移动导航光标**：与内容区一致（内容区滚轮本来就是移动光标）。此前导航只跟随键盘滚动 —— 侧栏靠 `ListState` 偏移、行式靠 `scroll_x`，所以**滚出视口的条目鼠标完全够不到**（实测 120 列、导航置顶时 15 个条目只有 11 个有命中区，剩下的既没画也没有任何鼠标途径）。回归测试滚完一圈，断言每个条目都能被滚轮选到。

## [1.2.1] - 2026-09-21

### 🚀 Features

- *(install)* **一键安装脚本，自己判定平台**：`install.sh`（Linux/macOS）与 `install.ps1`（Windows）。判定用 `uname -s`/`uname -m`，Windows 用 `RuntimeInformation.OSArchitecture`（不是 `PROCESSOR_ARCHITECTURE`——它在 ARM64 上跑 x64 模拟 shell 时会报错平台），musl 直接挡下并建议从源码装。下载后按发布里的 `SHA256SUMS` 校验；`SHA256SUMS` 拿不到时明确打印「未校验」，**校验和不匹配则拒绝安装**。`--version`/`--dir`/`--checksums`/`--host`（镜像）与对应环境变量可覆盖，`--dry-run` 先看计划，已装同版本即 no-op（`--force` 重装）。
- *(release)* **发布产物带 `SHA256SUMS`**：由发布作业从各平台产物现算，随 release 一起上传 —— 安装脚本据此校验，也给 PKGBUILD 那类手工校验和留了权威来源。
- *(windows)* **Windows Terminal 适配**：启动路径不再依赖 Windows 上不存在或在 ConPTY 下不可用的东西 —— 终端模式（括号粘贴 `CSI ? 2004 h`、kitty 键盘协议 `CSI > 1 u`）改为**直接写字节**（crossterm 的 `PushKeyboardEnhancementFlags` 在 Windows 是**无条件返回 Err**，而上游分支用 `?` 传播 ⇒ 1.2.0 在 Windows 上启动即失败）；鼠标捕获与光标形状改为**尽力而为**（缺控制台时只告警，不再中断启动）；`background = auto` 改用控制台的背景色与调色板（`GetConsoleScreenBufferInfoEx`，跟随当前配色方案）；封面协议按环境判定（`WT_SESSION` → sixel），因为 ConPTY 不会回答图形查询
- *(ui)* **`:spin` 开关封面旋转（同 `t` 键）**：`[playerbar] spinning_cover` 此前只能改配置文件，而同类可见性开关（`:visualizer` `:pitch` `:border` `:navpos`）都有命令与按键 —— 补上这块不一致。**裸调用即取反**（与 `:visualizer` 同），`on` / `off` 显式设置，`Tab` 补全 `off` / `on`。帧每次重绘都从配置读这一项，所以翻转**当场生效**；同时写回 `config.toml`，下次启动沿用

### 🐛 Bug Fixes

- *(startup)* **Windows 上不再挂死在启动路径**：`Picker::from_query_stdio()` 自己的超时只覆盖两次 read 之间、且每次 read 后就被重置，因此 stdin 处于 EOF（`-d`、CLI 子命令、测试、`boxpigma > file`）时它**永不返回**。实测后果：任何构造 `App::new` 的测试在 Windows 上全部挂死，CI 的 `test windows-latest` 作业挂了 **6 小时 5 分**后被取消（同日 ubuntu/macOS 各 2 分钟通过），最近 25 次运行里 22 次以 `cancelled` 收场。现在只在「有终端」且非 Windows 时询问，并且在工作线程里给 2 秒预算 —— 终端不回答就退回半块并继续启动
- *(ui)* **明暗主题探测在 Linux/macOS 上从未生效**：OSC 11 的回包 `ESC ] 11 ; rgb:… ESC \` **没有换行**，而探测跑在 `App::new` 里、早于 raw mode，行规缓冲区把回包扣住 ⇒ 120 ms 的 poll 必然超时 ⇒ 永远落到「深色」默认值。现在探测期间把 tty 置为非规范模式（并屏蔽 `SIGTTIN`/`SIGTTOU`：默认动作是**停止进程**，一个被停住的启动是「无错误的挂死」），扩展到全 unix，并用一个 pty 假终端测试钉住（canonical 拿不到回包、非 canonical 立刻拿到；该测试**只在 Linux 跑** —— 它断言的是内核行为，macOS 的 pty 实现无法在本机复现，CI 的 macOS 作业正是这么暴露出来的。探测代码本身仍在全 unix 编译）
- *(audio)* **ALSA 静音从未生效，还可能挡住音频**：`let _ = StderrGuard::new()…?` 让 guard 在语句结束即析构（要覆盖的那段正是 `open_sink_impl`），且创建失败会经 `?` 变成 `DeviceSinkError::NoDevice`。现在 guard 绑定到变量跨过打开过程，创建失败只记一条告警
- *(audio)* **没有音频设备时此前完全静默**：`ensure_sink!` 丢掉 `create_sink` 的 Err，UI 已经收到 `Started` 并显示在播放、进度冻住、看门狗因 `player == None` 不介入，用户看不到任何提示。现在每次失败弹一次 toast 并写一条 error（刻意**不**走 `PlaybackEvent::Error`：那条路会进「重试/跳歌」状态机，而「没有声卡」不该导致跳歌）
- *(ncm-api,sonar)* **4 处 UTF-8 字节切片可能 panic**：DEBUG 日志对响应体做 `&result[..len.min(N)]`（智能推荐、创建/收藏歌单）与 WBI mixin key 的 `[..32]` —— 只要切点落在多字节字符中间就会 panic（key 来自 B 站接口，歌名里全是中文）。新增 `ncm-api::text::preview()` 按字符边界截断，mixin key 改为取 32 个**字符**
- *(terminal)* **panic 后终端不再残留**：release 档 `panic = "abort"` 让 `Drop` 永不执行，`TerminalGuard` 那段「panic 路径也要关鼠标、弹掉键盘协议」的清理**在发布的二进制里是死代码**（与它自己的注释相反）。改为装 panic hook（并链到 ratatui 的 hook），abort 前照样执行
- *(input)* **鼠标移动不再让整帧重绘**：`?1003h` 下每次移动都上报，而全仓没有任何 hover 逻辑消费 `Moved`，旧循环却「一个事件一帧」。现在丢弃 `Moved`，并在每帧前先排空已入队事件（上限 256），拖动 seek/音量仍走 `Drag`
- *(bench)* **曲库扫描基准不再让整条命令失败**：它硬编码了作者机器上的 `/tmp/localmusic/带标签的歌.flac`，任何别的机器上都是 `assert!` 直接 fail（README 却写着「可复现」）。现在默认扫 `~/Music`、可用 `BOXPIGMA_BENCH_MUSIC` 覆盖，缺素材时打印提示并跳过；封面两项同样如此
- *(deps)* 仓库里被 tracked 的 `.cargo/config.toml.bak` 出库（并把 `*.bak` 加进 `.gitignore`）；MSVC 下 `link.exe` 只回一句 `LNK4044: 无法识别的选项` 的 `-fuse-ld=lld` 从 Windows 条目移除；`BufReader` 的平台化 import（此前 Windows 构建多一个 unused 告警）
- *(ui)* **专辑发行日不再跟随运行机器的时区**：`release_date` 此前把毫秒时间戳读成**读者所在偏移**再取日期，于是同一张专辑在不同时区会显示不同的一天。API 发的是**中国时间的零点** —— `1657814400000`（2022-07-14T16:00Z）在北京是 **2022-07-15**，在 UTC 下却显示成 07-14（**早一天**）。现改为固定按 `+08:00` 读取，与服务的日历一致；另加一条与时区无关的回归测试钉住这个边界。CI 跑在 UTC，正是它把这个既有缺陷暴露出来（此前 main 上的运行长期积压未完成，所以一直没被发现）

### 🧪 Testing

- Windows 与 Linux 现在跑同一套结果：Windows `cargo test --workspace --all-features` **342 passed / 0 failed**（修复前：5 个测试活锁、整套跑不完），Linux(WSL Ubuntu 26.04) 同套通过，`cargo +nightly fmt -- --check` 与 `cargo clippy --workspace --all-targets` 干净
- 新增回归测试：pty 假终端验证 OSC 11 回包在非规范模式下才读得到；`ncm-api::text::preview()` 的字符边界；WBI mixin key 的多字节与短 key

## [1.2.0] - 2026-09-21

### 🚀 Features

- *(ui)* **登录页成为唯一的认证入口**：二维码 / 账号密码 / 短信验证码 / 每日签到四种方式都在登录页里，方法之间与表单字段之间可切换，密码字段掩码显示；`:login` / `:signin` / `:sms` / `:smslogin` / `:sign` 仍可用，但改为**跳到登录页并选中对应方法**并把参数预填进表单 —— 四种方式只有一份实现
- *(ui)* **顶栏右端显示登录用户自己的头像（圆形）**：`LoginInfo::avatar_url` 此前从未被使用。登录成功时（会话自动登录也一样，两条路径都走 `handle_login_success`）按 `?param=120y120` 取图，裁中心方形后套**圆形蒙版**（圆外透明），画在顶栏右端 3 行 × 6 列、与昵称之间留 1 列。**任何失败都安静降级**（403 / 超时 / 解码失败 / 空 URL 一律当作没有头像，不报错也不占位）；退出登录清空，且**迟到的取图结果不会把上一个会话的头像贴回来**（安装前核对会话序号）。窄终端若「昵称 + 间隔 + 头像」放不下就整个不显示，昵称与 VIP 标记的宽度不变。只放内存：不写磁盘、不进 `cache/covers.rs`
- *(auth)* 短信验证码的「发送」与「登录」两步**共用同一个手机号**（此前 `:sms` 只提示用户自己再输一遍 `:smslogin <手机号> <验证码>`，输错号即失败）
- *(log)* **未知主题字段不再每帧写日志**：`Theme::field_color` 跑在渲染路径上、参数取自用户配置（如 `unfilled_color_cached = "warning"`），未知名字此前**每帧**警告一次 —— 播放时约 5~6 条/秒，日志文件再次无界增长（与「日志栈换新后不再无限增长」的说法相悖）。查找必须留在渲染路径（配置驱动的颜色就是这样工作的），改为**同一名字每次运行只报一次**。同一场景实测：**96 条 → 1 条**（同一配置、同一首歌、同样约 16 秒）
- *(ncm-api)* **MV（音乐视频）链路打通：歌曲 → MV id → MV 详情 → 播放直链**。`SongInfo` 新增 `mv`（歌曲详情 / 专辑 / 歌手接口叫 `mv`，旧版搜索接口叫 `mvid`；`0` 表示没有 MV，且为 `0` 时**不进 JSON**，不改变普通歌曲的 IPC 形状）；新增 `mv_detail`（标题 / 海报 / 时长 / 歌手 / 发布时间 / 简介 / 各清晰度）与 `mv_url`（`/api/song/enhance/play/mv/url`，`/api/mv/url` 已下线；一次一个清晰度，要 1080 会被服务端按实际画质降级）。实测：歌曲 347230 的 `mv` 为 376199，详情给出海报与 317490ms，`mv_url` 返回 480p 签名直链并按 `expi=3600` 过期，直链可下载到 `video/mp4` 数据
- *(playerbar)* **黑胶旋转**（`[playerbar] spinning_cover`，默认关闭）：封面按 **20 秒一圈**缓缓转动（72 个角度步、约 3.6 次/秒），暂停时**冻结在当前角度**；角度按墙钟累计、只在播放时推进。实现上**先旋转再套圆形蒙版**（圆边不会被逐步重采样变软），并**只在角度跨格时才重建协议对象** —— 角度未变时留在原处的协议渲染是免费的（kitty 只重摆放、不重传）。半块终端不做任何编码，改为旋转字形（`◴◵◶◷`）并在右上角画两态唱针（播放 `╲` / 暂停 `│`）；图像模式下字符层会被图像盖住，故不画唱针。实测每步 ≈ 0.97 ms、3.6 次/秒 ⇒ **0.35% 单核**；封面装载 467–601 µs/首，与历史同档未变慢
- *(ui)* **MV 海报面板**：歌词页右侧让出一列，画当前歌曲的 MV 海报与标题 / 歌手 / 时长 · 发布日期 / 简介（简介按剩余行数截断）。**随歌自动加载**：切歌时清空并取 `mv_detail`、按 `?param=480y480` 取海报，解码在 `spawn_blocking` 里、锁外完成；**只有 `mv != 0` 才发请求**。海报属于「这首歌」而非「这段歌词」—— 歌词还在加载时、纯音乐页上都照画。**任何失败都安静降级**（无 MV / 详情或海报取不到 / 字节不是图片一律当作没有面板），此时让位函数返回**同一个 `Rect`**，下游逐字节走原路径；宽度或高度不够（需列宽 ≤ 页面宽 − 2 − 30、页高 ≥ 14 行）也整块不画。迟到的守卫用**代际计数**：每首歌（**包括没有 MV 的**）都自增，于是「已清空且没有新请求」天然拒绝上一首的海报 —— 若用歌曲 id 作键，新歌没有 MV 时无 id 可比。海报尺寸随页面在 9–12 行间自适应（80×24 的历史默认尺寸也能显示）；13 条新测试覆盖几何判定、四种歌词样式并排、简介截断、代际过期、死端口与非法字节

### 🐛 Bug Fixes

- *(auth)* **每日签到的结果此前常常是空提示**：`dailyTask` 成功时响应里只有 `point` 没有 `msg`，而回显直接取 `msg`，于是弹出一片空白。现在把它变成成句中文（`签到成功（云贝 N）` / `今天已签到`），业务失败则返回真正的错误并带上服务端 code 与原因
- *(auth)* **风控错误丢失了服务端的说明，也没给出路**：`parse_login_info` 只读 `msg` 而风控响应把原因放在 `message` 里，于是 `-460` 显示成无用的 `登录失败 (code=-460)`。现在两种字段都读；`-460` / `-462` 直接给出可行建议 —— **终端里做不了网易云的滑块验证，请改用二维码登录**（这是当前唯一确定可用的登录方式）
- *(auth)* **页面上的登录错误现在是一行干净的句子**：`NcmError::Parse` 的 Display 是给日志用的多行文本（`parse: …` + 原始响应体），而登录页的错误区只有一行 —— 于是既出现重复的「登录失败: 登录失败:」，又被截断，`-460`/`-462` 的提示与出路会直接被切掉。现在这类「文案本身就是给用户看的」错误用新的 `NcmError::Message` 变体原样显示，不再套内部包装名与响应体
- *(auth)* `:sign` 未登录时此前显示 `API code=301: unknown error`（服务端没有 `msg` 字段）；现在直接说「需要登录后才能签到」
- *(auth)* 登录成功后界面可能完全不刷新：此前只在「当前在登录页 / Splash」或「原本未登录且当前在主页」时才重新加载，**在排行榜、搜索页等其它页面用 `:signin` 登录成功后，页面看起来仍是未登录**
## [1.1.0] - 2026-09-21

上游 `## Features` 里剩下的四项待办一次做完（`landing page` 不做）。

### 🚀 Features

- *(local)* **本地音频读真实标签**：标题 / 歌手 / 专辑 / 时长改用 `lofty` 读取（`read_cover_art(false)`，全库扫描不解析内嵌封面）。取不到标签时行为与本版之前**完全一致**：标题退回文件名、歌手退回「本地」、时长退回 rodio 解码、专辑退回文件路径。本地曲目的**歌手取值顺序与云盘上传一致**（TrackArtist → AlbumArtist）
- *(local)* **本地歌词**：识别音频旁的**同名侧车 `.lrc`**（大小写不敏感 ✓ 去 BOM ✓ 无有效时间戳则不冒充歌词 ✓），复用既有的 `parse_lyric_lines` 与歌词状态管线，不另造第二套。**故意不做缓存**：读本地文件与读它的缓存成本相当，而用户改了 `.lrc` 应当下次播放就生效
- *(cli/tui)* **更多可运行时修改的配置**：`:notify song_change|errors on|off`、`:mouse on|off`、`:cursor default|block|underline|bar`、`:lyricgradient <预设>`、`:saveonplay on|off`。终端类（鼠标捕获、光标形状）直接写对应的转义序列**当场生效**，其余下一帧/下一首生效；全部立即写回 `config.toml` 并进入 `ctrl+p` 命令面板
- *(playback)* **云盘作为兜底源**：解析链变成 NCM（网络错误重试一次）→ sonar 兜底 → **云盘兜底** → 才报错；云盘用的是既有的 `/weapi/v1/cloud/get` 分页 + 本地匹配（标题去括号段、忽略大小写；歌手互相包含；时长差 ≤5s 优先）。未登录 / 未命中 / 接口报错一律**安静回退**并保留原错误
- *(ui)* **歌手详情页**：进入方式与既有表格一致（`Enter` 进入歌手），页面显示简介 / 热门曲目 / 专辑，支持 `j k g G`、`Esc` 返回、`Enter` 播放、`r` 重载、鼠标滚动与双击

### 🐛 Bug Fixes

- *(core)* 新增 `AppEvent::Repaint`：此前主循环**只在收到事件时重绘**，没有歌在播时会阻塞在事件流上，于是后台任务填好的状态（如歌手页加载完成）要等到用户按任意键才上屏 —— 画面会一直停在「正在加载」
- *(playback)* 云盘匹配对 `(Remastered 2011)` 这类括号后缀命中不了：改为先去掉括号段再比较（由一次性探针抓出）

### 💼 Other

- `SongInfo` 新增 `local_path: Option<String>`：路径此前被塞在 `album` 里，导致本地播放与云盘上传都从专辑字段取文件、专辑标签无处安放。拆开后播放/上传读路径、专辑字段放标签（缺失退回路径），并补了 2 条契约测试（网络歌曲不多出该键；设过的路径必须能往返）
- 本地音乐目录仍是硬编码的 `~/Music`（`app/navigation.rs` 与 `service.rs` 两处），本次未改为配置项

## [1.0.1] - 2026-09-21

### 🐛 Bug Fixes

- *(cli)* **`boxpigma msg search <关键词> | head` 会 core dump**：Rust 启动时忽略 `SIGPIPE`，于是向已关闭的管道写入会变成写错误，`println!` 又把写错误变成 panic —— 发布档的 `panic = "abort"` 再把它变成 `SIGABRT` 与一份 core dump（退出码 134），而别的 Unix 工具只会安静退出。现在**子命令在做任何事之前**恢复 `SIGPIPE` 的默认行为（退出码 141、stderr 干净）；**TUI 与守护进程刻意不动** —— 它们必须活着才能还原终端，也不往管道里写。**这不是 1.0.0 引入的**：0.2.14 的发布二进制表现完全相同（已用其自带守护进程对照验证）

### 📚 Documentation

- 记录**内存占用实测**（同机、读 `/proc/<pid>/status` 的 `VmRSS`、0.2 秒粒度采样 24 秒）：TUI 主界面 **14.6 MB**、守护进程 **13.7 MB**，对照旧版 14.0 MB / 13.4 MB；四种情况的**启动峰值都等于 20 秒后的稳态**，且 **RSS ≈ 二进制体积 + 约 3 MB**
- 新增 [`FRAMEWORK.md`](./FRAMEWORK.md)：每项框架调整的改前改后、实测依据与代价，以及**被否决的候选**（`unicode-truncate` 不等价、`nucleo` 是功能变更、`palette` 无终端量化、`taskdump` 在本机会卡住等）
- README 全面重写，围绕 1.0 的改名与迁移、框架调整与实测数字

## [1.0.0] - 2026-09-21

### ⚠️ 破坏性变更

- **改名 `pigma` → `boxpigma`**：包名、二进制名、配置目录（`~/.config/boxpigma`）、缓存目录（`~/.cache/boxpigma`）、IPC socket（`pigma.sock` → `boxpigma.sock`）与 Windows 命名管道（`\\.\pipe\boxpigma`）全部随之改名
- **升级不会丢数据**：首次运行会把旧的 `pigma` 目录整体搬到新名字下（配置、cookie、队列、封面缓存都在里面），日志里留一行 `adopted … from before the rename`；若搬不动（比如旧实例还在跑、权限不足）则继续使用旧目录，绝不静默从空配置开始
- 旧二进制与**正在运行的旧守护进程**不会自动升级：请重启守护进程；`~/.local/bin/pigma` 是旧文件，可自行删除
- AUR 包名一并改为 `boxpigma-gblmx-bin`（发布到 AUR 需要以新包名注册；若希望保留旧包名，改回 `PKGBUILD` 与 release 工作流里的两处即可）

### 🚀 Features

- *(theme)* `default_theme = "random"`（以及 `light_theme`）每次启动随机挑一个主题；`:theme random` 立刻重掷并把抽到的那一个写回配置；补全与命令面板都能选到 `random`（它排在主题列表最前，因此循环切换每圈遇到一次）
- *(log)* 日志改用 `tracing` + `tracing-appender`：按天轮转、保留最近 7 个，行内带模块路径与本地时间。此前是单个无限增长的文件（实测已 2.8 MB），且写入在调用线程持锁同步进行；**135 个 `log::*!` 调用点一行未改**，由 `tracing-log` 桥接
- *(notify)* kitty 终端改用其自有的 `OSC 99` 通知：标题与正文分开、Base64 负载（`e=1`）、并用 `f=` 声明应用名，便于用户过滤；其余终端保持 `OSC 9` 不变
- *(ui)* 键位表里的页面行由页面表生成，页面不可能再从 `?` 里漏掉

### 🐛 Bug Fixes

- *(theme)* `all_names()` 取自 `HashMap`，`:theme` 的循环顺序每次启动都不同 —— 现在排序，顺序稳定
- *(net)* sonar 搜索、封面下载、音频流三处的 reqwest 客户端此前**没有任何超时**；现在统一 `connect_timeout` 10s + `read_timeout` 30s，封面与搜索另加 30s 总时限。音频流**刻意不加总超时**（会截断正在播放的下载），已用 34 秒持续下载的探针验证不会被切断

### 💼 Other

- *(ui)* 页面分发、页面按键与键位表合并为一张表：新增一个页面从改 **9 个文件降到 2 个**（`ui.rs` 生产代码里的页面匹配臂 5 → 0，`layout.rs` 3 → 0）
- *(ui)* 铺底色改用 `Fill`，不再用一个无边界的 `Block`
- *(theme)* WCAG 亮度/对比度改用 `palette`，全仓颜色数学从三处收敛到一处；等价性用全部 2²⁴ 个 8 位 sRGB 颜色与 19 个内置主题逐一核对（`on_accent` 结果全部一致）
- *(cli)* 版本号升至 `1.0.0`

## [0.2.14] - 2026-09-12

### 🚀 Features

- *(app)* Add login status request guard and improve song management in navigation (akirco)

### 🐛 Bug Fixes

- Restore terminal state and sync startup auth (caojialin)
- Synchronize liked songs pagination (Mars160)
- 修复帮助面板滚动溢出导致向上滚动失效的问题 (lorlike)

### 💼 Other

- *(deps)* Bump log from 0.4.33 to 0.4.34 (dependabot[bot])
- *(deps)* Bump stream-download from 0.24.3 to 0.24.4 (dependabot[bot])

### 📚 Documentation

- *(README)* Enhance completion script instructions for bash, zsh, fish, and powershell (akirco)

### ⚙️ Miscellaneous Tasks

- *(ci)* Fmt (akirco)
## [0.2.13] - 2026-08-22

### 🚀 Features


- *(cli/waybar)* Remove native waybar output (`--waybar`); waybar integration now uses a standalone bash script (`waybar/pigma`) that calls `pigma status --json` per invocation, with `ensure_daemon` for auto-start
- *(cli)* Add `pigma msg play <song-id>` to jump to a song in the queue by id, and `pigma msg toggle_play` play/pause toggle (akirco)
- *(cli)* Add `pigma msg search <keyword>`: the daemon searches NCM + sonar sources, returns songs tagged by source, and registers them so `pigma msg play <id>` can enqueue and play a search result across instances (sonar synthetic ids resolve in-process) (akirco)
- *(cli)* Move queue listing into `pigma msg list` (reusing the TUI queue-table rendering, `▶` marks the current song); drop the standalone `pigma list <endpoint>` command so the CLI only talks to a running instance (akirco)
- *(cli)* Add `pigma completions <shell>` to generate shell completion scripts (bash/zsh/fish/elvish/powershell); `msg` actions are now a clap `ValueEnum`, so `pigma msg <Tab>` completes them (with aliases) (akirco)
- *(cli)* Fold the global `--playlist` into `-d` via `ENDPOINT:N` (e.g. `pigma -d toplist:3`); the old global flag stays as a hidden backward-compatible alias. `msg switch-list --playlist N` is unchanged (akirco)


### 🐛 Bug Fixes

- *(playback)* Rebuild the audio device when the output stream dies (#61): Bluetooth disconnect/reconnect no longer leaves playback silent. The sink is now owned by the player task; fatal cpal stream errors (`DeviceNotAvailable`/`StreamInvalidated`/backend device loss on WASAPI/CoreAudio/ALSA) and a frozen-position watchdog (suppressed during recent network underruns) both trigger a rebuild that resumes at the last position. The player also follows system default-output changes (debounced, stable-id based), so after a Bluetooth headset reconnects playback moves back to it automatically — macOS, Windows and Linux alike


## [0.2.12] - 2026-08-16

### 🚀 Features

- *(path)* Add expand_tilde function to handle home directory paths (akirco)

### 🐛 Bug Fixes

- *(cli)* List download and local music panic (akirco)

### 🚜 Refactor

- *(cli)* Streamline CLI command handling and improve structure (akirco)
- *(bilivideo)* Improve cookie handling and enhance error logging for transient failures (akirco)
## [0.2.11] - 2026-08-16

### 🐛 Bug Fixes

- *(playback)* Bundle minimal ALSA config for musl static builds (akirco)
- *(playback)* Use default hw card in bundled musl ALSA config (akirco)

### 💼 Other

- Remove musl targets (static musl cannot use PipeWire/PA plugins) (akirco)

### 🎨 Styling

- *(playback)* Fmt and wrap unsafe set_var for edition 2024 (akirco)

### ⚙️ Miscellaneous Tasks

- Generate release notes with git-cliff and remove release-drafter (akirco)
- *(ci)* Remove 'dev' branch from CI trigger paths (akirco)
# Changelog

## [0.2.10] - 2026-08-16

### 🚀 Features

- *(ci)* Build linux aarch64 and musl targets with cross in CI and release (akirco)
- Add initial configuration for TOML format rules (akirco)

### 🐛 Bug Fixes

- *(manifest)* Flatten multi-line inline tables to single-line for strict TOML parsers (akirco)
- Update JSON structure for playback control commands in documentation (akirco)

### 🚜 Refactor

- Refactor cache management and improve performance (akirco)

### ⚙️ Miscellaneous Tasks

- Trigger CI on dev branch pushes (akirco)
## [0.2.9] - 2026-08-15

### 🚀 Features

- Add IPC support for pigma status and msg commands (akirco)
- *(waybar)* Add configuration and scripts for Pigma integration (akirco)
- Enhance IPC with queue management and headless mode support (akirco)
- *(ipc)* Add Windows named pipe support and update IPC documentation (akirco)
- *(ci)* Add aarch64 and aarch64-musl build jobs to CI workflow (akirco)
- *(ci)* Remove Linux audio dependencies installation and add pre-build scripts for aarch64 targets (akirco)
- *(ci)* Replace build script with inline pre-build steps for aarch64-musl target (akirco)
- *(ci)* Add x86_64-musl build job and update pre-build steps for ALSA (akirco)

### 🐛 Bug Fixes

- *(search)* Clamp search limit to prevent exceeding API constraints (akirco)
- *(ci)* Update ALSA_URL to point to the official ALSA project site (akirco)
- *(ci)* Update pre-build steps and environment variables for aarch64 and x86_64 targets (akirco)
- *(ci)* Update ALSA version and add environment variables for cross-compilation (akirco)
- *(ci)* Streamline environment variable setup and remove unnecessary build.env section (akirco)
- *(playback)* Update cfg attributes for Linux to include GNU environment (akirco)
- *(ci)* Add 'sed' to install dependencies for ALSA build (akirco)
- *(playback)* Refine Linux target configuration to include GNU environment (akirco)
- *(ci)* Add installation of musl-tools and ALSA build steps (akirco)
- *(ci)* Update musl-tools installation and setup for ALSA build (akirco)
- *(ci)* Enhance musl-tools installation for ALSA build with additional flags (akirco)
- *(ci)* Add symlink for asm-generic in musl-tools installation (akirco)

### 🚜 Refactor

- *(sonar)* Update dependencies and improve MD5 usage (akirco)

### ⚙️ Miscellaneous Tasks

- Add VSCode settings for CSS file association (akirco)
- *(ci)* Fmt (akirco)
## [0.2.8] - 2026-08-13

### 🚀 Features

- Enhance theme configuration and navigation features (akirco)
- Updated default theme colors for better visibility and aesthetics.
- Added navigation event handling for login functionality in input handling.
- Improved login key handling to navigate to the main page upon escape.
- Enhanced main input handling with volume adjustment and navigation position cycling.
- Refined splash screen input handling to streamline user experience.
- Modified layout structures to accommodate new logo rendering in the login screen.
- Implemented API service checks for login requirements on specific endpoints.
- Introduced command actions for cycling navigation positions.
- Enhanced splash state to track display duration.
- Updated UI components for better rendering and user feedback.
- Added help text for new login functionality and volume controls.
- Improved overall code structure and readability across multiple files.

### 🐛 Bug Fixes

- *(scan)* Prevent ID collision with sonar song-id by clearing top bit (akirco)
- *(splash)* Update version display to use dynamic package version (akirco)

### 🚜 Refactor

- *(ui)* Replace create_block with CornerBlock for improved block handling (akirco)
- *(README)* Update feature list and improve markup syntax explanation (akirco)
- Refactor navigation state and UI components for improved clarity and functionality
- Updated NavState to include methods for retrieving focused section, selected index, and selected item.
- Removed unused LoginState from NavigationState and adjusted related references.
- Enhanced navigation drawing logic to utilize new NavState methods for better readability.
- Added documentation comments to clarify the purpose of various functions and structures.
- Refined text input and UI rendering components with improved comments and organization.
- Implemented a new render_gauge function to streamline gauge rendering in the player bar.
- Updated utility functions for better clarity and consistency in naming conventions.
- Enhanced gradient and path utilities with improved documentation for better understanding.

### ⚙️ Miscellaneous Tasks

- *(ncm_client)* Fmt (akirco)

### 📚 Documentation

- *(README)* add splash screen and plan sections to improve documentation clarity
- *(config)* document splash duration setting in configuration example

## [0.2.7] - 2026-08-11

### 🚀 Features

- Add Homebrew tap update workflow for tag releases (akirco)
- Enhance audio playback error handling and buffering logic (akirco)

### 🚜 Refactor

- Remove ApiEndpoint enum from api.rs and integrate it into service.rs (akirco)
- *(app)* Implement event handling and navigation improvements (akirco)

### ⚙️ Miscellaneous Tasks

- Update dependencies in Cargo.lock and Cargo.toml for ncm-api and sonar (akirco)
## [0.2.6] - 2026-08-10

### 🚜 Refactor

- Update cookie file handling to improve permissions and writing logic (akirco)
- Refactor playback and UI components for improved performance and clarity
- Updated `handle_main_key` to remove unnecessary Arc wrapping for songs.
- Introduced `current_resolve` in `PlaybackEngine` to manage in-flight song resolve tasks.
- Modified `activate_by_id` to accept a `persist_previous` flag for better queue management.
- Changed song handling in `play_songs` and `append_songs_to_key` to use `Arc<SongInfo>` directly.
- Enhanced `ApiService` to return `Arc<SongInfo>` for shared ownership across components.
- Refactored UI rendering functions to streamline theme resolution and scrollbar rendering.
- Updated help text for clearing the playback queue to use 'w' instead of 'Ctrl+L'.
- Improved overall code readability and maintainability by reducing unnecessary clones and enhancing comments.

## [0.2.5] - 2026-08-09

### 🚀 Features

- Add keyboard shortcuts for manual refresh and help panel (akirco)
- Enhance song liking functionality with improved event handling and user feedback (akirco)
- Implement liked songs functionality with cloud sync and UI updates (akirco)

### 🐛 Bug Fixes

- *(ci)* Fmt (akirco)
## [0.2.4] - 2026-08-09

### 🚀 Features

- Add 'save_on_play' configuration and related functionality (akirco)
- Add Skeleton widget for loading state representation (akirco)

### 🚜 Refactor

- *(ncm-api)* Rewrite ncm-api (akirco)
- Update imports to use playback module for PlaybackState (akirco)
## [0.2.3] - 2026-08-08

### 🐛 Bug Fixes

- *(cover)* Wt img capability check (akirco)

### ⚙️ Miscellaneous Tasks

- Adjust file and code structure (akirco)
- Update config example (akirco)
- *(ci)* Fmt (akirco)
## [0.2.2] - 2026-08-07

### 🚀 Features

- *(sonar)* New fetch playlist api(get track_ids + lazy pagination) (akirco)
- *(mode)* Add toast for mode switch (akirco)

### 🐛 Bug Fixes

- *(sonar)* Examples and new test for bibili search (akirco)
- *(styled_text)* Styled_text is overrideded by default (akirco)

### 🚜 Refactor

- *(content)* Use built-in Row instead of manually (akirco)
- *(playlist)* New data loading logic (akirco)
- *(utils)* Remove unnecessary utils export (akirco)
- *(playerbar)* Simplify code logic (akirco)

### ⚙️ Miscellaneous Tasks

- *(ci)* Fmt (akirco)
## [0.2.1] - 2026-08-06

### 🐛 Bug Fixes

- *(theme)* Unknown color name(removed) (akirco)
## [0.2.0] - 2026-08-06

### 🚀 Features

- *(musicx)* Third-party multi-source search with lyrics/cover fallback (akirco)
- *(musicx)* Lyrics/cover loading and playback queue integration (akirco)
- *(input)* Search source switching and shortcut enhancements (Tab to switch source, g/G, S to like current playing) (akirco)
- *(ui)* Help popup and proxy config support for Normal/Reversed/Both (akirco)
- *(layout)* Hide sidebar and adapt cover size on narrow terminals (akirco)
- *(theme)* Add light theme and title style (akirco)
- *(playback)* Update progress bar color on cache completion and persist musicx registry (akirco)
- *(musicx)* Register utils::musicx module (akirco)

### 🐛 Bug Fixes

- *(playback)* Bilibili stream download 403 and proxy support for stream downloads (akirco)
- *(ncm-api)* Use device id and md5-hashed password in login (akirco)

### 🚜 Refactor

- *(playback)* Remove types.rs, merge types into playback.rs (akirco)
- [**breaking**] Rename musicx crate to sonar (akirco)
- *(core)* Migrate pigma to the sonar crate (akirco)

### 📚 Documentation

- Update README.md (akirco)

### 🧪 Testing

- *(musicx)* Testing (akirco)

### ⚙️ Miscellaneous Tasks

- *(ci)* Bump checkout and ssh-agent actions (akirco)
- *(config)* Update example config and linker flags (akirco)
## [0.1.9] - 2026-08-03

### 🚀 Features

- *(crates/musicx)* Unifield fallback sound source (akirco)

### 🚜 Refactor

- *(config)* Restructuring the config structure (akirco)
- *(fallback)* Using new sound source fallback (akirco)
- *(layout)* New navigation layout (akirco)

### 📚 Documentation

- *(README)* Add aur installtion desc (akirco)
- Update README.md (akirco)

### ⚙️ Miscellaneous Tasks

- Update deps (akirco)
## [0.1.8] - 2026-08-01

### 🚀 Features

- *(navigation)* New layout (top) (akirco)

### ⚙️ Miscellaneous Tasks

- *(release)* Add aur release (akirco)
- *(state)* Nav.rs renamed to navigation.rs (akirco)
## [0.1.7] - 2026-07-30

### 🐛 Bug Fixes

- *(events)* Seeking spinner (akirco)
- *(cover)* Ratatui-image image protocol check failed (akirco)
- *(navigation)* Need not cache failed responses (akirco)

### 🚜 Refactor

- *(playback)* Optimize memory by reusing player & manual memory recycling (akirco)

## [0.1.6] - 2026-07-29

### 💼 Other

- *(deps)* Bump actions/checkout from 4 to 7 (dependabot[bot])
- *(deps)* Bump softprops/action-gh-release from 2 to 3 (dependabot[bot])
- *(deps)* Bump actions/upload-artifact from 4 to 7 (dependabot[bot])

### 🚜 Refactor

- *(config)* Rewrite config file inline ArrayOfTables. (akirco)
- *(playerbar)* Fix layout issues (akirco)
- *(playerback)* Adjust cpal buffersize,reduce the frequency of thread wakes (akirco)
## [0.1.5] - 2026-07-27

### 🚀 Features

- *(navigation)* Add saved albums navigation tab (#18) (AshGrey🥕)
- Migrate API to service calls, improve uploads, local music cloud drive, and cover caching, update contribution guide (akirco)
- fixs: cache value `accessed_at` always 0, table content overwritten during fast navigation (akirco)

## [0.1.4] - 2026-07-26

### Added

- Daily recommendation "not interested": press `d` to mark a song as not interested, telling the algorithm not to recommend similar songs
- Daily recommendation "like": press `s` to add a song to My Liked Music (available on all song pages)
- Proxy target config `proxy_target`: supports `yt` (proxy YouTube, default), `ncm` (proxy NetEase Cloud Music), `both` (proxy both)

### Changed

- Perf: per-character gradient lyric rendering eliminates per-char String allocation (zero-allocation borrowing)
- Perf: gradient preset changed from string dispatch to enum match, eliminating multiple string comparisons per frame
- Perf: table field query returns `Cow` to avoid String clone
- Perf: player bar time display reuses `format_duration_into` buffer
- Perf: cache lookup merged into a single RwLock + iteration (was 4 locks + stat)
- Perf: cache total size tracked via `AtomicU64`, evict avoids O(n) stat syscalls
- Perf: evict sorting avoids filename clone
- Perf: `collect_cached_songs` removes redundant `path.exists()` check
- Perf: storage IO (playlist saving) offloaded to blocking thread via `spawn_blocking`
- Local music now loads on demand: released when switching navigation, reloaded from disk cache or re-scanned when returning
- My Liked Music: fixed missing cache write, now writes to disk cache after first load
- My Liked Music: most recently liked songs shown at top of list (IDs reversed)
- NCM proxy fix: corrected `like` endpoint params (endpoint `/api/radio/like`, params `trackId`/`alg`/`time`)
- Daily recommendation dislike endpoint corrected to `/api/v2/discovery/recommend/dislike` (params `resId`/`resType`/`sceneType`)

### Removed

- Playback reporting feature (`report_play` API call and `pending_report` mechanism)

### Fixed

- Fixed ratatui-image loading album covers using excessive memory (request 200x200 thumbnail from NCM CDN instead of original image)

## [0.1.3] - 2026-07-25

### Added

- Album cover display: terminal album cover rendering via `ratatui-image`, auto-cropped to circle
- Multiple player bar layouts: `default`, `modern`, `minimal`, configurable via `playerbar.layout`
- Player bar component visibility config: `playerbar.visible` independently controls cover, volume, play mode, and loading animation
- Border gradient animation: `border_gradient` and `border_gradient_speed` options with clockwise flowing gradient effect
- Config example: new `config.example.toml` with complete documentation of all options
- Centralized API service layer (`service.rs`): unified endpoint resolution, cache integration, and error mapping
- Recursive local music scan: automatically scans audio files in subdirectories
- Search result limit: new `search_limit` config option
- Automatic cache eviction: LRU-based auto cleanup of cache over 2GB, with stale entry cleanup

### Changed

- Refactored all API calls from `self.api` to `self.service.client()`, decoupling business layer from API layer
- Player bar split into multi-module structure (`widgets`, `build_layout`, `default_layout`, `modern_layout`, `minimal_layout`)
- Cache index lock upgraded from `Mutex` to `RwLock` for better concurrent read performance
- NCM network retries reduced from 3 to 2 for faster fallback to YouTube source
- buffer underrun/overrun errors silently ignored, rodio auto-recovers
- `PlaybackEngine::new` now takes `CacheManager` directly instead of scattered path/template params
- Removed commented-out dev-dependencies in `Cargo.toml`

### Fixed

- Fixed cache index potentially containing incomplete download entries (now written only after download completes)
- Fixed stale entries from deleted files not cleaned in cache index (auto-cleaned on exit)
- Fixed local music scan missing audio files in subdirectories

## [0.1.2] - 2026-07-23

### Added

- Gradient progress bar (GradientLineGauge) with colorgrad preset themes
- Border config `BorderConfig` with `rounded` and `follow_corner_color` options
- Player progress bar gradient config: `gradient_enabled` and `gradient_preset`
- Cache index stores song duration to avoid decoding audio on playlist load
- Async cache methods: `load_lyrics_cache_async`, `list_cached_songs_async`
- YouTube search helper module (`utils/youtube.rs`) with traditional/simplified Chinese normalization and improved match scoring

### Changed

- Refactored event system: `AppEvent` split into five domain sub-events `SplashEvent`, `AuthEvent`, `PlaybackEvent`, `NavigationEvent`, `CommandEvent`
- Unified playback strategy into a single `Strategy` enum, removed `Box<dyn PlayStrategy>` dynamic dispatch
- Player `player::run` returns a oneshot completion signal, ensuring previous track's decoder/sink/StreamDownload fully released before next starts
- YouTube search helpers extracted from `AudioSource` into a separate module
- Removed example files in examples dir and dev-dependencies
- Added `rustfmt.toml` for unified code formatting

### Fixed

- Fixed resource leak from old player resources (HTTP connections, buffers) not released promptly on track switch
- Fixed cache index deserialization compat with old format (plain string → new object format smooth migration)

## [0.1.1] - 2026-07-21

### Added

- YouTube fallback playback via y7dl submodule
- User-created playlist API (`user_created_playlist`)
- User-collected playlist API (`user_collected_playlist`)
- `SongList` model adds `subscribed` field
- Navigation adds "My Created Playlists" and "My Collected Playlists" endpoints
- Cache manager supports indexed cache and custom filename templates
- New cache config options: `cache_dir`, `quality`, `cache_template`
- History queue limited to max 200 songs
- Heartbeat mode limited to max 500 songs with auto queue trimming
- Auto-select current song in content list during playback
- Liked Music auto-sets playlist ID to support Heartbeat mode

### Changed

- Refactored cache manager to use `cache_index.json` indexed cache
- Split "My Playlists" into "My Created Playlists" and "My Collected Playlists"
- Improved audio quality selection with configurable `SongQuality`
- Enhanced Heartbeat mode logging and error handling
- Fixed local file playback, improved local music scan using path-based unique IDs

### Fixed

- Fixed downloaded music showing 00:00 duration (reads actual duration from audio file)

### Documentation

- Updated license info to Apache-2.0 and added usage notes
- Added Windows Scoop installation instructions

## [0.1.0] - 2026-07-20

### Added

- Pigma initial release - terminal music player
- Playback engine supporting multiple audio formats (MP3, FLAC, WAV, OGG, AAC, M4A, WMA)
- NetEase Cloud Music API integration for streaming playback
- Local music scanning and playback
- Playlist management with auto save/restore
- Multiple play modes: sequential, single loop, list loop, random, heartbeat
- Volume control and progress seeking
- Lyrics display and translation support
- UI styled text rendering
- UI gradient theme support
- Downloaded/cached song management
- Search functionality
- Keyboard shortcut navigation
- Playback queue management
- Artist and album browsing
- Charts browsing
- QR code login

### Changed

- Refactored UI and utility modules for better performance and organization
- Refactored playback module and UI components
- Improved code readability and module organization
- CI workflow adds Linux audio dependency installation
- Refactored log initialization and enhanced playback features

### Fixed

- Simplified release targets for stable builds
- Fixed release workflow dependencies and artifact upload
- Ran cargo fmt to unify code style
