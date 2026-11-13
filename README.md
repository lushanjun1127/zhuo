# 酌（Zhuo）

酌是一款纯 Web、本地优先的角色叙事工作台。它把角色卡、对话、世界设定和记忆留在自己的设备上，通过 Ollama 或其他兼容接口调用语言模型。

酌的目标不是做一个复杂的平台，而是让开始一段故事尽可能直接：导入角色、选择模型、开始聊天；需要时再逐步启用世界信息、Persona 和长期记忆。

## 核心理念

### 纯本地，数据由你掌握

- 前端是浏览器中的 Web 应用，后端是本机 Rust 服务，不需要云端账号。
- 默认连接本机 Ollama；对话、角色卡、设置和记忆保存在本机。
- SQLite 数据库默认位于项目根目录的 `zhuo.db`，不应提交到代码仓库。
- 也可以在设置中连接其他 OpenAI 兼容服务；使用远程服务时，请自行确认服务商的数据处理方式。

### 双轨记忆

- **对话记忆**：保存每个角色的会话和消息，切换会话后仍可继续。
- **长期记忆**：可选后台提取模型把较早的对话整理成可检索的记忆，为后续聊天补充重要信息。
- 长期记忆提取模型可在设置面板单独选择，也可以关闭。提取任务固定使用 CPU（`num_gpu: 0`），避免与主聊天模型争抢显存。
- SQLite 使用 WAL 日志模式和 5 秒 busy timeout，减少读取历史与写入记忆同时发生时的数据库锁冲突。

### 极致轻量

- 浏览器 + Rust/Axum + SQLite；不需要桌面壳、容器或独立数据库服务。
- Ollama 与 Web、后端可通过一个命令启动。
- 不安装或启用额外模块也能完成基本角色聊天；更复杂的上下文和记忆功能按需使用。

## 功能一览

- 导入 PNG 或 JSON 角色卡，支持 Character Card V2 的常见字段、首条消息和内嵌角色世界书。
- 同一角色建立多个会话，流式接收回复、查看历史、重新生成最后一条回复。
- 管理 Persona，并把 Persona 与本地 World Info 作为独立 System Prompt 上下文注入，不混入用户消息。
- 关键词触发 World Info；支持常驻条目、主/次关键词、大小写、全词匹配和扫描深度等字段。
- 为角色保存并检索长期记忆及角色状态。
- 从 SillyTavern 资产导入独立世界书、Chat Completion/API Preset 和 Regex JSON。
- 导出角色卡 JSON；PNG 角色卡可重新写入角色数据并导出。
- 选择 Ollama 主聊天模型和后台记忆提取模型，设置主模型的 GPU 层数。
- 使用 SQLite 持久化会话、消息、设置和记忆，并自动迁移旧版单会话数据库。

## 新手快速启动

以下步骤以 Windows 为例；macOS 和 Linux 用户可安装相同工具，再在项目目录运行对应的 npm 命令。

### 1. 安装 Rust

从 [Rust 官方安装页](https://rustup.rs/) 安装 Rust stable 工具链。Windows 用户按提示安装 Rust，并在安装器要求时安装 Visual Studio C++ Build Tools。安装完成后重新打开终端，确认：

```powershell
rustc --version
cargo --version
```

### 2. 安装 Node.js

安装 [Node.js LTS](https://nodejs.org/)，建议使用 Node.js 20 或更新的 LTS 版本。npm 会随 Node.js 一起安装：

```powershell
node --version
npm --version
```

### 3. 安装 Ollama 和模型

从 [Ollama 下载页](https://ollama.com/download) 安装 Ollama，并确认终端可以运行：

```powershell
ollama --version
```

先下载至少一个本地模型。可以从较小的模型开始，根据显存和使用语言选择适合自己的模型：

```powershell
ollama pull qwen2.5:3b
ollama list
```

模型下载和首次加载需要时间。Ollama 默认监听 `127.0.0.1:11434`。启动脚本发现该端口没有服务时，会尝试运行本机的 `ollama serve`；如果 Ollama 已经在运行，脚本会直接使用现有服务。

### 4. 安装项目依赖并启动

在项目根目录执行：

```powershell
npm install
npm run start
```

`npm run start` 会同时启动 Ollama 检查/服务、Rust 后端和 Vite 前端。首次运行 Rust 后端时，Cargo 会下载依赖并编译，耗时会比之后启动更长。保持该终端窗口运行，然后打开：

- 前端：<http://localhost:3000>
- 后端：<http://127.0.0.1:1127>

退出时在启动终端按 `Ctrl+C`。

### 5. 选择模型并开始聊天

1. 在页面中选择一个角色；没有角色时，使用左侧角色栏的导入按钮导入 PNG 或 JSON 角色卡。
2. 打开页面右上角的齿轮“设置”。
3. 确认 Ollama Base URL 为 `http://127.0.0.1:11434`，点击“刷新本地模型列表”。
4. 在“主聊天模型”中选择刚刚下载的模型。若要启用长期记忆提取，再选择“后台记忆提取模型”；留空即关闭自动提取。
5. 调整显存分配层数并开始聊天。模型、连接设置和显存层数会保存到本地 SQLite，后续请求立即使用，不需要重启应用。

## 连接、模型与显存设置

- Ollama 本地 Base URL 默认是 `http://127.0.0.1:11434`。模型列表通过后端代理 Ollama 的 `/api/tags` 获取。
- 主聊天模型在设置面板中选择；未从模型列表选择前，可先运行 `ollama pull <模型名>`。
- 主模型 `num_gpu` 默认是 `15`，适合从 6GB 显卡开始尝试；12GB 以上显卡可以尝试 `99`。实际可用层数取决于模型大小和系统显存，出现显存不足时应调低。
- 后台记忆提取模型强制使用 `num_gpu: 0`，在 CPU 上运行。选择较小的模型可减少提取延迟。
- 设置面板的选择保存在 `app_settings` 数据表中，并优先于环境变量里的模型默认值。
- 也可在设置面板连接其他 OpenAI 兼容服务。远程服务的 Base URL、密钥和模型要求以服务商文档为准。

## 环境变量

项目根目录的 `.env` 是本机配置，不会被 Git 跟踪。可以从示例复制：

```powershell
Copy-Item .env.example .env
```

大多数新手无需修改 `.env`；推荐在网页设置面板配置模型。`.env.example` 仅包含可公开的示例值，不要把自己的 API Key、令牌或其他凭据写入示例文件或提交到版本库。

常用变量如下：

| 变量 | 默认值/用途 |
| --- | --- |
| `ZHUO_API_BASE` | 推理服务地址；本地 Ollama 示例为 `http://127.0.0.1:11434/v1` |
| `ZHUO_API_KEY` | API 密钥；本地 Ollama 通常不校验，可使用示例值 `ollama` |
| `ZHUO_MODEL` | 初始主模型回退值；一般在网页设置中选择模型即可 |
| `ZHUO_SUMMARY_MODEL` | 初始记忆提取模型；留空时不启用，数据库中保存的设置优先 |
| `ZHUO_MAIN_NUM_GPU` | 主模型 GPU 层数初始值，默认 `15`；网页设置可覆盖 |
| `ZHUO_OLLAMA_NATIVE` | 是否使用 Ollama 原生接口，默认 `1`；设为 `0` 使用 OpenAI 兼容接口 |
| `ZHUO_SUMMARY_ROUNDS` | 触发一次记忆提取所需的未处理消息数量，默认 `8` |
| `ZHUO_DB` | SQLite 数据库文件路径，默认项目根目录的 `zhuo.db` |
| `ZHUO_CARDS_DIR` | 角色卡目录，默认 `examples` |
| `ZHUO_WORLD` | 初始世界设定 JSON 路径，默认 `examples/world.json` |
| `ZHUO_MAX_CHARS` | 提示词上下文字符上限，默认 `12000` |
| `ZHUO_PORT` | Rust 后端监听端口，默认 `1127` |
| `ZHUO_NO_USAGE` | 任意非空值会关闭流式响应中的 usage 统计请求 |
| `OLLAMA_CONTEXT_LENGTH` | Ollama 服务启动时使用的上下文长度；示例默认 `4096` |
| `OLLAMA_FLASH_ATTENTION` | Ollama 服务启动时是否启用 Flash Attention |
| `OLLAMA_KV_CACHE_TYPE` | Ollama 服务启动时的 KV 缓存类型 |

Ollama 专用变量只会在启动脚本负责启动 Ollama 时传入。若 Ollama 已由其他程序启动，需要重启 Ollama 服务后这些变量才会生效。

## 数据与隐私

- 默认本地数据库为 `zhuo.db`；项目也会产生 SQLite 的 `zhuo.db-wal` 和 `zhuo.db-shm` 文件。这些文件属于运行数据，不要提交到 Git。
- 角色卡文件保留在配置的角色卡目录中；默认目录为 `examples`。
- 删除或移动数据库前请先关闭应用，并自行备份需要保留的对话与记忆。
- 连接远程推理服务时，发送给该服务的提示词和消息会离开本机；请按自己的隐私要求选择服务。
- `.env` 不会由应用界面回显 API Key 明文。不要把本机 `.env` 上传到公共仓库。

## 开发与测试

```powershell
npm run typecheck
npm run build
cargo fmt --all -- --check
cargo test --workspace
```

本地只运行前端时可使用 `npm run dev`；单独运行后端可使用 `npm run backend`。开发服务器默认在 `3000` 端口，后端默认在 `1127` 端口，前端会把 `/api` 请求代理给后端。Vite 会忽略 SQLite 数据库及 WAL/SHM 文件，避免数据库写入触发页面热更新。

## 项目结构

```text
src/                         React 前端
scripts/                     本地开发启动脚本
crates/zhuo-server/          Axum HTTP 服务和 API 路由
crates/zhuo-llm/              模型服务请求与流式响应
crates/zhuo-memory/           SQLite 会话、记忆与设置存储
crates/zhuo-card/             角色卡解析与导入导出
crates/zhuo-session/          会话辅助逻辑
examples/                    示例角色卡和世界设定
```

## 常见问题

### 模型列表为空或无法刷新

确认 Ollama 正在运行，并在浏览器打开 <http://127.0.0.1:11434/api/tags>。如果 Ollama 安装在其他端口或地址，请在设置面板修改 Base URL，再刷新列表。确认模型已通过 `ollama pull <模型名>` 下载。

### 页面提示后端连接失败

确认 `npm run start` 的终端中 Rust 后端已启动并监听 `1127` 端口。停止旧的开发进程后重新运行 `npm run start`；Vite 的配置变更需要重启开发服务器后生效。

### 显存不足或模型加载失败

先降低设置面板中的 `num_gpu`，或选择较小的主聊天模型。模型大小、上下文长度、KV 缓存以及显卡上其他程序都会影响显存占用。记忆提取模型始终使用 CPU。

### 自动记忆提取没有运行

打开“连接与生成”设置，确认“后台记忆提取模型”已选择且本地可用。留空表示关闭自动提取；提取也会在达到未处理消息数量阈值后触发。

## 许可证

本项目采用 MIT License，详见根目录的 [LICENSE](./LICENSE)。
