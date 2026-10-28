---
name: boxpigma
description: 网易云音乐 TUI 客户端 boxpigma 的 CLI 控制技能（status / msg 子命令与 IPC 控制）
---


# boxpigma – 网易云音乐客户端（CLI 控制）

## Overview
**boxpigma** 是一个网易云音乐 TUI 客户端（ratatui）。不带子命令直接运行即进入交互界面；同时提供
`boxpigma status` / `boxpigma msg` 两个子命令，通过 IPC（Unix socket
`~/.cache/boxpigma/boxpigma.sock`）查询或控制**正在运行**的实例
（交互界面或 `-d` 守护进程均可）。适合脚本、状态栏（Waybar）、远程控制。

---

## 安装与启动

```bash
cargo build --release
# 将 target/release/boxpigma 加入 PATH
```

- **无子命令** → 启动交互式 TUI。
- **守护进程**（无头后台）：
  ```bash
  boxpigma -d                # 等价于 boxpigma -d liked
  boxpigma -d toplist        # 加载指定端点
  boxpigma -d toplist:3      # 歌单/榜单端点选第 3 个（1 起始）
  ```
  守护进程只加载队列，**不自动播放**，需显式 `boxpigma msg play`。
  `SIGINT`/`SIGTERM` 会保存会话并干净退出。

---

## 全局选项（`boxpigma --help`）

| 选项 | 说明 |
|---|---|
| `-v, --version` | 打印版本号并退出 |
| `-d, --daemon [<ENDPOINT[:N]>]` | 无头守护进程模式；省略 ENDPOINT 默认 `liked`，`:N` 选第 N 个歌单（可与子命令冲突，见下） |
| `--socket <SOCKET>` | 自定义 IPC socket 路径 |
| `-h, --help` | 显示帮助 |

> `args_conflicts_with_subcommands = true`：`-d`/`--socket` 等全局选项不能与子命令混用；
> 但 `status`/`msg` 子命令内部自带 `--socket`，可指定目标实例。

---

## 命令自动补全（`boxpigma completions`）

为常见 shell 生成补全脚本（打印到 stdout，请重定向到你的补全目录）：

```bash
# bash
boxpigma completions bash > /usr/share/bash-completion/completions/boxpigma
# zsh（按 fpath 或补全目录）
boxpigma completions zsh > "${fpath[1]}/_pigma"
# fish
boxpigma completions fish > ~/.config/fish/completions/boxpigma.fish
```

支持 `bash` / `zsh` / `fish` / `elvish` / `powershell`。补全会列出子命令与选项；
`boxpigma msg <Tab>` 会补全动作名（`previous`/`next`/`play`/`switch-list`/`list` 等，含别名）。

---

## `boxpigma status` – 查询状态

```bash
boxpigma status [OPTIONS]
```

| 选项 | 说明 |
|---|---|
| `--template <TEMPLATE>` | 自定义 plain 输出。占位符：`{name}` `{artist}` `{album}` `{current}`/`{position}` `{duration}` `{volume}` `{status}` `{mode}` `{id}` `{liked}`。覆盖配置 `cli_status_template` |
| `--json` | JSON 输出（优先于配置 `cli_status_format`） |
| `-L, --list` | 列出当前播放队列（`▶` 标记当前曲目，行尾带歌曲 id）；配合 `--json` 输出原始 `QueueSnapshot`（含 `key`） |
| `--socket <SOCKET>` | 指定实例的 socket（覆盖全局/默认） |

`{status}` 取值：`playing` / `paused` / `stopped`；`{mode}` 取值：`sequential` / `repeat_one` /
`repeat_all` / `shuffle` / `heartbeat`。

```bash
boxpigma status                                   # plain（默认模板来自配置）
boxpigma status --json | jq '.name'
boxpigma status -L                                # 队列列表
boxpigma status -L --json                         # 队列原始 JSON
boxpigma status --template "{artist} – {name} [{status}] vol {volume}%"
```

---

## `boxpigma msg list` – 列出播放队列

查询**正在运行**的实例的当前播放队列（复用 TUI 队列表格的显示逻辑），
`▶` 标记当前曲目，行尾带歌曲 id：

```bash
boxpigma msg list            # 列出当前播放队列（行尾 id 可直接用于 msg play）
boxpigma msg list --json     # 原始 QueueSnapshot（key/current_index/songs；song 含 id/name/singer/album/duration_ms）
```

选项：`--json`、`--socket <SOCKET>`。

---

## `boxpigma msg <ACTION> [VALUE]` – 控制播放

```bash
boxpigma msg [OPTIONS] <ACTION> [VALUE]
```

| 动作 | 别名 | 说明 |
|---|---|---|
| `play` | | 播放/恢复；`play <ID>` 跳到队列中指定歌曲并播放 |
| `search <KEYWORD>` | | 搜索并**返回**歌曲数据（请求/响应，非 fire-and-forget）；搜索网易云，每行标 `source` 与 `id`，并注册到运行实例，随后 `play <ID>` 即可播放选中的那首 |
| `toggle_play` | `play_pause` | 播放/暂停切换（停止时开始） |
| `pause` | | 暂停 |
| `next` | | 下一首 |
| `previous` | `prev` | 上一首 |
| `list` | | 列出当前播放队列（`▶` 标记当前曲目；`--json` 输出原始 `QueueSnapshot`） |
| `switch-list <ENDPOINT>` | `switch` | 动态切换队列到指定端点；歌单端点用 `--playlist N` 选第 N 个（1 起始） |
| `volume <VALUE>` | | `75`=绝对音量 0-100；`+5`/`-10`=相对 ±% |
| `seek <VALUE>` | | `+15`=向后 15 秒、`-30`=向前、`50%`=跳到一半、`90`=跳到第 90 秒；与 TUI 的 `:seek` 共用同一套语法，值不合法时**在客户端就被拒**（不会传到实例里变成一条用户看不见的提示） |
| `clear` | | 清空当前播放队列（等价于 TUI 的 `:clear`） |
| `mode` | | 切换播放模式 |
| `like` | | 喜欢当前曲目 |
| `dislike` | | 不喜欢当前曲目 |
| `toggle_like` | `unlike` `toggle` | 喜欢/取消喜欢（切换） |
| `capabilities` | `caps` | 打印本 IPC 契约（接口版本、程序版本、动作清单、socket 路径）；只读，无需登录 |

选项：`--playlist <INDEX>`（switch-list 用）、`--json`（list / capabilities 用）、`--socket <SOCKET>`。

```bash
boxpigma msg play
boxpigma msg play 187186        # 按歌曲 id 播放（`boxpigma status -L` 的行尾就是 id）
boxpigma msg search 周杰伦       # 返回: source + id + 歌名 - 歌手（在运行实例内搜索，跨实例可用）
boxpigma msg play 33894312      # 播放上面搜到的某首
boxpigma msg toggle_play
boxpigma msg next
boxpigma msg volume 75
boxpigma msg volume +5
boxpigma msg seek +15           # 向后跳 15 秒（同一套语法：+15 / -30 / 50% / 90）
boxpigma msg clear              # 清空当前播放队列
boxpigma msg list               # 列出当前播放队列
boxpigma msg switch-list toplist --playlist 2
boxpigma msg toggle_like
```

### `boxpigma msg capabilities` – 自描述的扩展契约

这是给**外部工具**（waybar、播放控制脚本、其它 agent）用的契约入口：先问一次动作清单，
再决定发什么命令，不必猜某个版本支持哪些动作、别名怎么写。它只读、不依赖登录，
守护进程一首歌都没放时也照常回答。

```bash
boxpigma msg capabilities          # 人读的表格（动作 + 别名 + 是否吃参数 + 说明）
boxpigma msg capabilities --json   # 与 IPC 返回同一份 JSON，供脚本解析
boxpigma msg caps                  # 别名，等价
```

`--json` / socket 返回的形状（`api` 是契约版本，`version` 是程序版本，`actions` 按 `name` 升序，
`socket` 是这个实例实际监听的 socket 路径；下面只留了前几个动作）：

```json
{
  "api": 1,
  "version": "1.4.0",
  "actions": [
    { "name": "capabilities", "aliases": ["caps"], "takes_value": false, "summary": "Print this contract: API version, program version, actions, endpoint" },
    { "name": "clear", "aliases": [], "takes_value": false, "summary": "Clear the playback queue (the `:clear` command)" },
    { "name": "dislike", "aliases": [], "takes_value": false, "summary": "Dislike the current song" },
    { "name": "next", "aliases": [], "takes_value": false, "summary": "Go to the next song" },
    { "name": "previous", "aliases": ["prev"], "takes_value": false, "summary": "Go to the previous song" },
    { "name": "seek", "aliases": [], "takes_value": true, "summary": "Jump in the track (`+15`, `-30`, `50%` or a second; same as `:seek`)" },
    { "name": "volume", "aliases": [], "takes_value": true, "summary": "Set the volume (0-100) or adjust it (+5/-5)" }
  ],
  "socket": "/home/you/.cache/boxpigma/boxpigma.sock"
}
```

同一份数据也能直接问 socket（回一行 JSON，见下面「IPC 协议」一节）。

**契约版本规则**：只有动作被**改名或删除**时才递增 `api`；新增动作、或在现有 JSON 里新增字段
都不递增 —— 所以工具可以放心忽略不认识的字段，只在 `api` 变大时才重新读一遍动作清单。

---

## IPC 协议（直接走 socket）

子命令底层就是往 socket 发一行 JSON、收一行 JSON（需换行结尾）。
`msg` 成功回 `{"ok":true}`。可用 socat / 脚本直接控制。

> `action` 是内部标签对象，必须写成 `{"cmd":"msg","action":{"action":...}}` 的嵌套形式
> （即 `boxpigma msg` 实际发送的 JSON）；写成 `{"action":"play"}` 会被服务端丢弃。
> 嵌套里的 `action` 值可以是 `boxpigma msg capabilities` 列出的任意名字或别名
> （如 `prev`、`toggle-play`、`switch`）。

```bash
# 查询状态（回一行 JSON）
printf '{"cmd":"status"}\n' | socat - "$HOME/.cache/boxpigma/boxpigma.sock"
# 列出播放队列
printf '{"cmd":"list"}\n' | socat - "$HOME/.cache/boxpigma/boxpigma.sock"
# 问能力（回一行 JSON：契约版本 + 程序版本 + 动作清单 + socket 路径）
printf '{"cmd":"capabilities"}\n' | socat - "$HOME/.cache/boxpigma/boxpigma.sock"
# 搜索（回一行 JSON 数组，结果已注册到运行实例，可直接 play 其 id）
printf '{"cmd":"search","keyword":"周杰伦"}\n' | socat - "$HOME/.cache/boxpigma/boxpigma.sock"
# 播放控制（注意嵌套的 action 对象）
printf '{"cmd":"msg","action":{"action":"next"}}\n' | socat - "$HOME/.cache/boxpigma/boxpigma.sock"
printf '{"cmd":"msg","action":{"action":"play"}}\n' | socat - "$HOME/.cache/boxpigma/boxpigma.sock"
# 音量：绝对（0.0-1.0）或相对增量
printf '{"cmd":"msg","action":{"action":"volume","absolute":0.75}}\n' | socat - "$HOME/.cache/boxpigma/boxpigma.sock"
printf '{"cmd":"msg","action":{"action":"volume","delta":0.05}}\n'    | socat - "$HOME/.cache/boxpigma/boxpigma.sock"
# 切换队列（歌单端点可用 "playlist" 选第 N 个，1 起始）
printf '{"cmd":"msg","action":{"action":"switch_list","endpoint":"toplist","playlist":2}}\n' | socat - "$HOME/.cache/boxpigma/boxpigma.sock"
```

socket 路径可用 `--socket <path>` 自定义。

---

## 桌面媒体会话（MPRIS）

除 socket 之外，TUI 与守护进程都在会话总线上提供 `org.mpris.MediaPlayer2.boxpigma`
（对象路径 `/org/mpris/MediaPlayer2`，两个接口：`org.mpris.MediaPlayer2` 与
`.Player`），命令走的是与 `boxpigma msg` 同一套实现：`playerctl`、KDE / GNOME 的媒体
控件、键盘与蓝牙耳机媒体键都能用它控制播放，属性里带封面 `mpris:artUrl`
（指向封面缓存文件）。

```bash
playerctl -p boxpigma play-pause        # 播放/暂停（同 msg toggle-play）
playerctl -p boxpigma next             # 下一首（同 msg next）
playerctl -p boxpigma volume 0.4       # 绝对音量（同 msg volume 0.4）
playerctl -p boxpigma position 90      # 跳到 90 秒（同 msg seek 90）
playerctl -p boxpigma loop Track       # 单曲循环（只有 MPRIS 能直接设，`msg` 只能轮换）
playerctl -p boxpigma metadata --format '{{title}} – {{artist}}'

busctl --user introspect org.mpris.MediaPlayer2.boxpigma /org/mpris/MediaPlayer2
busctl --user call org.mpris.MediaPlayer2.boxpigma /org/mpris/MediaPlayer2 \
    org.mpris.MediaPlayer2 Quit        # 退出（同 `q`，含会话保存与 cookie 落盘）
```

与 MPRIS 名词对不上的地方（刻意如此）：

| MPRIS | 本程序 |
|---|---|
| `Stop` | 跳到 0 秒再暂停：队列没有独立的「已停止」态 |
| `LoopStatus` / `Shuffle` | 是「顺序 / 单曲 / 列表 / 随机 / 心动」五个模式里的两位。心动上报为 `Playlist` + `Shuffle`；写这两个属性会按它们拼出的模式离开心动（`msg mode` 仍然是轮换） |
| `Position` | 可读、不广播（规范如此：客户端自己轮询）；`Seeked` 在 seek 后发出 |
| `OpenUri` / `Raise` | 空实现 |

会话总线不可用（无 `DBUS_SESSION_BUS_ADDRESS`、私有 bus、权限被拒）时只写一行
`MPRIS 媒体会话不可用` 警告，播放不受影响。

同一台机器上跑第二个实例时，**新实例接管**这个名字（zbus 的 `Builder::name()` 默认带
`ReplaceExisting` + `AllowReplacement`，与 mpv、浏览器一致），先启动的那个收到 `NameLost`
后不再持有媒体会话——它自己的播放与 IPC 照常，只是媒体键、浮层归最新实例。

---

## 端点参考（与导航项一致）

| 端点 | 类型 | 说明 |
|---|---|---|
| `liked` | 歌曲（默认） | 我喜欢的音乐，需登录 |
| `recommend_songs` | 歌曲 | 每日推荐歌曲，需登录 |
| `user_cloud_disk` | 歌曲 | 我的云盘，需登录 |
| `download` | 歌曲 | 本地下载 |
| `local_music` | 歌曲 | 本地音乐（扫描 `~/Music`） |
| `recent` | 歌曲 | 最近播放，需登录 |
| `recommend_resource` | 歌单 | 每日推荐歌单，需登录 |
| `toplist` | 歌单 | 排行榜 |
| `top_song_list` | 歌单 | 热门歌单 |
| `user_radio_sublist` | 歌单 | 我的电台，需登录 |
| `user_song_list` | 歌单 | 用户歌单，需登录 |
| `user_created_song_list` | 歌单 | 创建的歌单，需登录 |
| `user_subscribed_song_list` | 歌单 | 订阅的歌单，需登录 |
| `album_sublist` | 歌单 | 收藏的专辑，需登录 |
| `search` | 其他 | 搜索热榜（无可播队列） |
| `top_singers` | 其他 | 热门歌手（无可播队列） |

歌单类端点解析为「一组歌单」，默认加载第一个；`-d` 用 `ENDPOINT:N`（如 `toplist:3`），
`msg switch-list` 用 `--playlist N` 选第 N 个（1 起始）；
序号与 TUI 中列表显示的顺序一致，可先在 TUI 里查看。需登录的端点未登录时返回 `未登录`。

---

## 配置（简要）

配置文件默认 `~/.config/boxpigma/config.toml`（Linux）。与 CLI 相关的项：
- `cli_status_template`：`boxpigma status` 默认 plain 模板。
- `cli_status_format`：`plain` 或 `json`；命令行 `--json` 优先。
- 其他项见 `config.example.toml`。

---

## 典型用法

```bash
# 1. 后台守护 + 开始播放
boxpigma -d
boxpigma msg play

# 2. Waybar 状态模块（bash 脚本，每秒轮询；参考 waybar/boxpigma）
waybar/boxpigma                              # 主状态：song – artist | vol% · mode
waybar/boxpigma --icon like                  # 按钮图标：like | play | prev | next
boxpigma status --json                       # 原始 JSON 输出（脚本内部调用）
boxpigma status --template "{artist} – {name} [{status}]"

# 3. 音量快捷键
boxpigma msg volume +5
boxpigma msg volume -5

# 4. 动态换队列
boxpigma msg switch-list recommend_songs

# 5. 多实例（自定义 socket）
boxpigma -d --socket /tmp/music.sock
boxpigma status --socket /tmp/music.sock
boxpigma msg --socket /tmp/music.sock play
```

---

## 故障排查

| 问题 | 解决 |
|---|---|
| `boxpigma status` 连接失败 | 确认实例在运行（TUI 或 `boxpigma -d`）；检查 `--socket` 路径 |
| `-d` 后无播放 | 必须显式 `boxpigma msg play` |
| 提示未登录 | 需登录端点（`liked` 等）要求已登录，先在 TUI 登录（`L` 键） |
| 队列为空 | 实例未加载歌曲队列；先 `boxpigma msg switch-list <endpoint>` 切换或 `boxpigma -d` 加载端点 |
| `--playlist N` 越界 | 歌单序号与 TUI 列表顺序一致，先在 TUI 里确认 |
| 未知动作 | `boxpigma msg --help` 查看合法动作，或 `boxpigma msg capabilities` 问运行中的实例 |

---

## Notes
- 所有子命令都是即发即走（非阻塞），走本地 socket，无需网络。
- 交互界面与守护进程都暴露同一 IPC；守护进程可挂 systemd / Waybar。
- 补全脚本：`boxpigma completions bash|zsh|fish|elvish|powershell`。
- 完整帮助：`boxpigma --help`、`boxpigma status --help`、`boxpigma msg --help`。