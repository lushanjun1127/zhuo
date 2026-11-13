# Zhuo

本地优先的角色叙事工作台，使用 React、Rust/Axum、SQLite 与 OpenAI-compatible API。

## 启动

```powershell
npm install
npm run start
```

浏览器访问 `http://localhost:3000`（**必须**用 `localhost`，用 `127.0.0.1` 会被后端 CORS 拒绝）。默认连接本机 Ollama，也可在右上角设置中配置其他 OpenAI-compatible 服务。

## GPU / CPU 分流

后端默认走 Ollama 原生 `/api/chat`（其 OpenAI 兼容层会丢掉 `options` 字段），保证 `num_gpu` 真正生效：

- `ZHUO_MAIN_NUM_GPU`（默认 `99`）：主对话模型尽量全部卸载到 GPU。
- `ZHUO_SUMMARY_NUM_GPU`（默认 `0`）：后台记忆提取模型强制纯 CPU，不抢显存。
- `ZHUO_OLLAMA_NATIVE`（默认 `1`）：设为 `0` 可切回标准 `/v1/chat/completions`（非 Ollama 端点时使用）。

`.env` 由启动脚本注入进程环境，`ZHUO_API_KEY` 的优先级高于设置面板里保存的值；密钥不会明文返回给浏览器。

## 当前能力

- PNG/JSON 角色卡导入与首条消息
- 每个角色多会话、流式生成和会话持久化
- Persona、关键词触发的 World Info、长期记忆和角色状态
- Ollama/OpenAI-compatible 连接及生成参数
- 旧版单会话数据库自动迁移

## SillyTavern 资产迁移

打开右上角“设置”面板，在“SillyTavern 资产”区域可以：

- 导入独立世界书 JSON；支持常驻条目、主/次关键词、大小写、全词匹配和扫描深度。
- 导入 Chat Completion/API Preset JSON；会应用 temperature、top_p、frequency/presence/repetition penalty 和最大输出长度。
- 导入单个或数组形式的 Regex JSON；AI 回复流结束后会在浏览器中按 ST JavaScript 正则语义处理，历史消息再次打开时同样处理。
- 将当前角色导出为 Character Card V2 JSON；PNG 来源的角色还可以重新写入 `chara` tEXt 块后导出 V2 PNG。

导入的 V2 角色卡会应用 `system_prompt`、`post_history_instructions` 和内嵌 `character_book`，未识别字段会在 JSON/PNG 往返导出时保留。
