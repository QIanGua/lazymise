# lazymise

<p align="center">
  <a href="README.md">English</a> | <strong>简体中文</strong>
</p>

一款 Vim 优先、受 `lazygit` 启发的 [`mise`](https://mise.jdx.dev/) 终端工作台 —— 在一个非阻塞 TUI 中统一管理 **Tasks（任务）**、**Tools（工具）**、**Env（环境变量）**、**Config & Health（配置与健康检查）** 与 **Command Log & Jobs（命令日志与后台作业）**，深度对齐 `mise` 官方演进路线。

## 界面布局概览

在宽终端（`>= 96` 列）下，`lazymise` 渲染 5 面板工作台：左侧手风琴边栏（36%）、右侧多子视图详情区（64%）以及可折叠的底部命令日志与作业抽屉。按 `+` / `_` 可对当前焦点面板进行三级缩放（`Normal` ↔ `Half` ↔ `Full`）；在窄终端（`< 96` 列）下会自动降级为顶部紧凑 `1-5` 徽章栏。

```text
┌─ lazymise │ ~/workspace/repo │ ENV:[default] │ [⠋ outdated] ─────── [+:zoom ?:help] ─┐
│┌─[1] TASKS (12 • 1 running)────┐┌─[Spec]  Output   Deps   History ─── [ ] switch ───┐│
││ ▾ [root] (4)                  ││ NAME     check                                    ││
││   ● check   [1.2s]            ││ TAGS     #ci    CACHE: ⚡ on                      ││
││   ✓ test    [0.8s • 14 runs]  ││ DEPENDS  tidy, test, lint                         ││
││   ✓ build   [2.1s]            ││ ───────────────────────────────────────────────── ││
││ ▸ exp (5)                     ││ SCRIPT / COMMAND (cached, zero render I/O)        ││
││ ▸ [global] (3)                ││ ...                                               ││
│└───────────────────────────────┘│                                                   ││
│┌─[2] TOOLS (8 • ↑2 • ⚠1 CVE)───┐│                                                   ││
││ ✓ rust 1.90.0  ↑ node 22→23   ││                                                   ││
│└───────────────────────────────┘│                                                   ││
│┌─[3] ENV (24 • 3 redacted)─────┐│                                                   ││
││ OUT_DIR, RUST_LOG, 🔒TOKEN... ││                                                   ││
│└───────────────────────────────┘│                                                   ││
│┌─[4] CONFIG & HEALTH (3 • 1W)──┐│                                                   ││
││ ✓ mise.toml  ⚠ doctor:1 warn  ││                                                   ││
│└───────────────────────────────┘└───────────────────────────────────────────────────┘│
│┌─[5] COMMAND LOG & JOBS (@:toggle) ─ 1 running ─────────────────────────────────────┐│
││ 13:20:04  ⠋ RUN  [USR] mise run check              [1.2s] (K:kill  Enter:inspect)  ││
││ 13:19:50  ✓ 0    [INT] mise outdated --json        [0.37s]                         ││
│└────────────────────────────────────────────────────────────────────────────────────┘│
│ Enter:run  b:bg  A:args  w:watch │ x:menu  /:filter  E:env  ::palette  ?:help  q:quit│
└──────────────────────────────────────────────────────────────────────────────────────┘
```

## 五大面板与子视图

| 面板 | 快捷键 | 左侧列表 / 摘要卡片 | 右侧详情子视图（`[` / `]` 切换） |
| --- | :---: | --- | --- |
| **Tasks** | `1` | 可折叠命名空间 / Monorepo 任务树，带运行状态、历史耗时、缓存（`⚡`）与 `#tag` 徽章 | `Spec` · `Output`（实时 ANSI）· `Deps` · `History` |
| **Tools** | `2` | 已安装与已配置工具列表，支持 5 档范围筛选（`All`/`Active`/`Project`/`Missing`/`Prunable`）、可升级箭头与 CVE 徽章 | `Details` · `Outdated` · `Audit`（OSV.dev）· `Lock` |
| **Env** | `3` | 环境变量列表，支持 6 档来源筛选（`All`/`Project`/`Local`/`Global`/`Tool`/`Redacted`）与敏感值锁图标 | `Value & Source` · `PATH` · `Vars` |
| **Config & Health** | `4` | 生效配置继承链，带 `[TRUSTED]` / `[UNTRUSTED]` / `[OVR]` 状态与诊断计数 | `File & Explain` · `Doctor` · `Settings` · `Daemons & Deps` |
| **Command Log & Jobs** | `5` | 实时后台作业与透明的 `mise` CLI 命令审计日志（`@` 切换抽屉状态） | 完整 ANSI 作业输出与命令详情 |

## 核心特性

- **零阻塞异步加载**：首帧立即渲染，不因外部子进程阻塞；高频核心域（`Tasks`、`Tools`、`Env`、`Config`、`Templates`）并行拉取，次级域（`Registry`、`Outdated`、`Doctor`、`Audit`、`Lock`、`Settings`、`Daemons`、`Deps`）后台或按需加载，配合基于 generation 的过期响应丢弃机制，空闲时 0% CPU 占用。
- **`MISE_ENV` 配置档切换（`E`）**：自动发现工作区中的 `mise.<env>.toml` 配置档（支持逗号组合如 `dev,local`），顶部实时展示 `ENV:[profile]` 徽章，并在所有查询与执行命令中一致注入 `-E <profile>`。
- **内置工具注册表搜索与安装（`2` → `a`）**：在内存中模糊搜索 `mise` 1,079+ 官方工具，异步加载 `mise ls-remote` 版本列表，选择项目或全局作用域并在确认精确 `mise use` 命令后执行。
- **供应链安全与锁文件治理**：集成 OSV.dev 漏洞审计（`mise audit --json`）、一键升级受影响工具（`mise upgrade --vulnerable`）、预览 `mise lock --dry-run` 差异并在确认后更新 `mise.lock`。
- **任务工作流与 Monorepo 支持**：前台 TTY 移交（带 RAII 终端恢复与 `$EDITOR` shell 词法解析）、多后台并发作业、`#USAGE`（`usage_spec`）参数表单（`A`）、`--force` 强制执行（`f`）、`mise watch`（`w`，自动检测 `watchexec`）、`mise tasks validate`（`v`）、隐藏任务切换（`H`）及 Monorepo `--all` 发现（`M`）。
- **环境变量治理**：合并 `mise env --redacted --json` 与启发式敏感词检测；默认脱敏隐藏敏感值，按 `S` 临时明文显示（30 秒后自动重新隐藏），复制敏感值前发出提示，并支持 `mise set` / `mise unset` 增删改。
- **配置治理与脚手架生成**：内联展示 `mise config explain` 诊断、结构化 `mise doctor` 仪表盘、可搜索的 `mise settings`、实验性 `[daemons]` 与 `[deps]` 管理，以及统一的 `g` 脚手架生成弹窗（覆盖全部 7 个官方 `mise generate` 目标与 `$MISE_TEMPLATES_DIR` / `~/.config/mise/templates` 自定义模板）。
- **多作业进程组管理与 ANSI 渲染**：独立 `JobId` 隔离、Unix 进程组级终止（`SIGTERM` → `SIGKILL`，退出时自动清理并防误关运行中作业）、`\r` 进度条原地覆写，以及零依赖 16/256/TrueColor ANSI SGR 渲染。
- **安全的版本级卸载与配置移除区分（`d`）**：在 `[2] TOOLS` 中，对非激活、未引用或待清理的旧版本按 `d` 仅执行 `mise uninstall <tool>@<version>`，不会误改 `mise.toml`；对活跃的已配置工具按 `d` 才执行 `mise unuse <tool>`，并在确认弹窗中明确显示受影响的配置文件路径。
- **零记忆负担的交互发现与上下文 80/20 底部提示栏**：单一真相来源（Single Source of Truth）键位注册表驱动动态底部提示栏，随当前面板、子视图和光标选中项实时切换高频操作（用 `│` 分隔上下文操作与全局快捷键，窄屏自动保底 `x:menu ?:help q:quit`），同时提供可搜索执行的 `?` 帮助、上下文 `x` 操作菜单、`:` / `Ctrl-p` 命令面板与完整鼠标交互（点击面板/子标签/配置档徽章、双击主操作、悬停滚轮滚动）。
- **高可访问性终端美学**：透明背景画布，兼容 `NO_COLOR` / `CLICOLOR=0` / `TERM=dumb` 与 256/16 色降级，提供非颜色依赖的冗余状态图标、精简过滤后的工具 `OPTIONS` 元数据展示，以及 `unicode-width` 安全的中文/Emoji 截断。

## 安装与快速开始

需要 Rust `1.88+` 且 `PATH` 中已安装 `mise`。

```bash
# 通过 mise 从源码编译并全局安装
mise run install

# 或直接使用 Cargo 安装
cargo install --path . --locked
```

### 命令行用法

```bash
lazymise                          # 在当前目录启动，默认进入 Tasks 面板
lazymise -C /path/to/repo         # 指定检查目标目录（长选项：--dir）
lazymise -E dev                   # 以 MISE_ENV=dev 启动（长选项：--env）
lazymise -E dev,local -t tools    # 以组合配置档启动并直接聚焦 Tools 面板
lazymise --tab config             # 启动后聚焦 Config & Health（兼容旧别名：--tab templates）
lazymise --tab jobs               # 启动后聚焦 Command Log & Jobs
```

| 参数 | 可选值 / 默认值 | 说明 |
| --- | --- | --- |
| `-C`, `--dir <DIR>` | 当前目录 | 指定要检查的工作目录 |
| `-E`, `--env <ENV>` | 无（`default`） | 初始 `MISE_ENV` 配置档（如 `dev`、`prod` 或 `dev,local`） |
| `-t`, `--tab <TAB>` | `tasks` \| `tools` \| `env` \| `config` \| `jobs`（默认 `tasks`） | 启动时聚焦的面板（`templates` 作为 `config` 的兼容别名） |
| `-h`, `--help` | — | 打印命令行帮助 |
| `-V`, `--version` | — | 打印版本号 |

## 快捷键说明

所有快捷键均定义在单一注册表（[`src/app/keymap.rs`](src/app/keymap.rs)）中，统一驱动事件分发、动态底部提示、`?` 帮助、`x` 上下文菜单与 `:` 命令面板。

### 全局快捷键

| 快捷键 | 操作 |
| --- | --- |
| `1` – `5` | 切换活动面板（`Tasks` / `Tools` / `Env` / `Config & Health` / `Jobs & Log`） |
| `j` / `k`（或 `↓` / `↑`） | 移动光标或上下滚动详情区 |
| `gg` / `G`（或 `Home` / `End`） | 跳到顶部 / 底部（Config 面板下 `g` 打开脚手架生成；`Home` 全局可用） |
| `Ctrl-d` / `Ctrl-u`（或 `PgDn` / `PgUp`） | 向下 / 向上翻半页 |
| `h` / `l`（或 `←` / `→`）、`Tab` / `Shift-Tab` | 在边栏、主详情区与底部日志抽屉间切换焦点（Tasks 中 `h`/`l` 亦可折叠/展开分组） |
| `[` / `]` | 切换右侧详情区子视图 |
| `+` / `_`（亦支持 `=` / `-`） | 放大 / 缩小当前视图（`Normal` ↔ `Half` ↔ `Full`） |
| `@` | 循环切换底部日志抽屉状态（`Hidden` → `Compact` → `Expanded`） |
| `/` | 对当前面板进行模糊筛选（`Enter` 应用，`Esc` 清除） |
| `Esc` | 清除当前筛选或关闭弹窗 |
| `x` | 打开当前选中项的上下文操作菜单 |
| `:` 或 `Ctrl-p` | 打开全局命令面板 |
| `E` | 切换当前 `MISE_ENV`（`-E`）配置档 |
| `R` | 异步刷新全部 `mise` 状态 |
| `r` | 重新运行上次任务（或在 Jobs 面板中重跑选中命令） |
| `?` | 打开可搜索的快捷键帮助（按 `Enter` 直接执行高亮操作） |
| `q` / `Ctrl-c` | 退出（有后台作业运行时 `q` 会二次确认；3 秒内连按两次 `Ctrl-c` 强制退出） |

### 面板 `[1]` Tasks

| 快捷键 | 操作 |
| --- | --- |
| `Enter` | 前台 TTY 运行任务（或在分组头上折叠/展开；存在必填参数时自动弹出表单） |
| `b` | 后台运行任务并实时查看 ANSI 输出 |
| `A` | 带参数 / `#USAGE` 标志表单运行任务 |
| `f` | 使用 `--force` 运行任务（跳过 `sources`/`outputs` 缓存） |
| `w` | 通过 `mise watch -t <task>` 监听重跑 |
| `Space` 或 `za` | 折叠 / 展开任务命名空间分组 |
| `H` | 切换显示隐藏任务（`--hidden`） |
| `M` | 切换显示 Monorepo 全部子包任务（`--all`） |
| `v` | 校验任务定义（`mise tasks validate`） |
| `e` | 在 `$EDITOR`（回退 `$VISUAL`，默认 `nvim`）中编辑任务源文件 |
| `y` | 复制 `mise run <task>` 命令到剪贴板 |
| `F` | 在 `Output` 视图切换自动滚底跟踪 |

### 面板 `[2]` Tools

| 快捷键 | 操作 |
| --- | --- |
| `a` | 搜索 `mise` 注册表并添加工具（`mise use`） |
| `i` | 安装已配置工具（`mise install`） |
| `u` | 升级选中工具（`mise upgrade <tool>`） |
| `U` | 升级全部可更新工具（`mise upgrade`） |
| `V` | 升级存在漏洞的工具（`mise upgrade --vulnerable`） |
| `d` | 卸载选中的非激活/旧版本（`mise uninstall <tool>@<ver>`），或从配置移除活跃工具（`mise unuse <tool>`） |
| `p` | 清理未使用的旧版本（`mise prune`） |
| `L` | 预览 `mise lock --dry-run`（通过 `x` 菜单应用 `mise lock`） |
| `A` | 运行 OSV.dev 安全审计（`mise audit --json`） |
| `O` | 刷新可升级工具检查（`mise outdated --json`） |
| `f` | 循环切换工具范围（`All` / `Active` / `Project` / `Missing` / `Prunable`） |
| `o` | 在浏览器中打开工具主页或发布页链接 |
| `e` | 编辑选中工具来源的 `mise.toml` |

### 面板 `[3]` Env

| 快捷键 | 操作 |
| --- | --- |
| `a` | 新增环境变量（`mise set`） |
| `e` | 编辑选中环境变量（`mise set`） |
| `d` | 移除选中环境变量（`mise unset`） |
| `S` | 切换明文显示敏感值（30 秒后自动重新脱敏） |
| `y` | 复制环境变量值到剪贴板 |
| `Y` | 复制 `KEY=VALUE` 键值对到剪贴板 |
| `f` | 循环切换来源筛选（`All` / `Project` / `Local` / `Global` / `Tool` / `Redacted`） |

### 面板 `[4]` Config & Health

| 快捷键 | 操作 |
| --- | --- |
| `Enter` / `e` | 在 `$EDITOR` 中编辑选中配置文件 |
| `t` | 信任 / 取消信任选中配置（`mise trust` / `--untrust`） |
| `F` | 格式化 `mise.toml`（`mise fmt`） |
| `D` | 刷新 `mise doctor --json` 诊断结果 |
| `g` | 打开脚手架生成菜单（`mise generate` + 自定义模板） |
| `s` | 启动 / 停止选中守护进程（位于 `Daemons & Deps` 子视图） |
| `S` | 同步项目依赖（`mise deps`） |

*（更多诊断与服务操作，如 `mise reshim`、`mise doctor project` 与守护进程重启，可在 `x` 上下文菜单中直接触发。）*

### 面板 `[5]` Jobs & Command Log

| 快捷键 | 操作 |
| --- | --- |
| `Enter` | 在主详情区查看选中作业的完整输出 |
| `K` | 终止运行中的后台作业进程组 |
| `r` | 在后台重新运行选中命令 |
| `y` | 复制精确 CLI 命令到剪贴板 |
| `c` | 清空已完成的命令日志条目 |
| `i` | 切换显示内部只读/刷新类 `mise` 命令 |
| `F` | 切换作业输出的自动滚底跟踪 |

## 开发与校验

所有可复用工作流均通过 [`mise.toml`](mise.toml) 暴露：

```bash
mise tasks        # 查看项目全部任务
mise run dev      # 调试模式运行 lazymise（cargo run）
mise run check    # 运行工作区规范检查（含中英文 README 同步校验）+ 测试 + clippy（-D warnings）
mise run install  # 编译 release 版本并安装到 ~/.cargo/bin
```

更多关于界面约定、色彩降级、布局规则、安全护栏与并发架构的详细设计，请参考 [`docs/design.md`](docs/design.md)。
