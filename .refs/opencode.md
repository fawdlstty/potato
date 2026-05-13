# OpenCode Serve HTTP 协议

## 概述

OpenCode serve 是 OpenCode 的无头 HTTP 服务器，通过 `opencode serve` 命令启动。它暴露一组 OpenAPI 3.1 规范的端点，用于与 OpenCode 进行程序化交互。

**默认配置**：
- 主机：`127.0.0.1`
- 端口：`4096`
- 完整地址：`http://127.0.0.1:4096`
- OpenAPI 文档：`http://127.0.0.1:4096/doc`

## 核心 API 端点

### 1. 健康检查

```
GET /global/health
```

**响应示例**：
```json
{
  "healthy": true,
  "version": "1.14.39"
}
```

### 2. 创建 Session

```
POST /session
```

**请求 Body**：
```json
{
  "title": "会话标题"
}
```

**响应示例**：
```json
{
  "id": "ses_1dfca0700ffe1X4CRrSQOlVQ5A",
  "slug": "misty-river",
  "version": "1.14.39",
  "projectID": "...",
  "directory": "/path/to/project",
  "path": "",
  "title": "test",
  "time": {
    "created": 1778656868608,
    "updated": 1778656868608
  }
}
```

**关键字段**：
- `id`：session 唯一标识，后续所有消息发送都需要此 ID

### 3. 发送消息（非流式）

```
POST /session/{sessionID}/message
```

**请求 Body**：
```json
{
  "parts": [
    {
      "type": "text",
      "text": "用户消息内容"
    }
  ],
  "model": {
    "providerID": "provider-id",
    "modelID": "model-id"
  }
}
```

**字段说明**：
- `parts`：消息内容数组，每个 part 包含 `type` 和 `text`
  - `type`: 固定为 `"text"`
  - `text`: 实际的文本内容
- `model`：模型配置对象
  - `providerID`: 提供商 ID（如 `opencode`、`su8-5-10`、`right-5-10` 等）
  - `modelID`: 模型 ID（如 `gpt-5.3-codex`、`deepseek-v4-flash-free` 等）

**响应示例**：
```json
{
  "info": {
    "id": "msg_e2039e69c0015uT231F7xYoKdg",
    "parentID": "msg_e2039e68f001cjQh9WRt0RvUzB",
    "role": "assistant",
    "mode": "build",
    "agent": "build",
    "path": {
      "cwd": "/path/to/project",
      "root": "/path/to/project"
    },
    "cost": 0,
    "tokens": {
      "total": 15397,
      "input": 22,
      "output": 2,
      "reasoning": 13,
      "cache": {
        "write": 0,
        "read": 15360
      }
    },
    "modelID": "deepseek-v4-flash-free",
    "providerID": "opencode",
    "time": {
      "created": 1778657126044,
      "completed": 1778657129635
    },
    "finish": "stop",
    "sessionID": "ses_1dfca0700ffe1X4CRrSQOlVQ5A"
  },
  "parts": [
    {
      "type": "step-start",
      "id": "prt_e2039ef3b001vEpODXI0D4uoD7",
      "snapshot": "0739572c09d71ceffedae9cbc9448937cf997af9",
      "sessionID": "ses_1dfca0700ffe1X4CRrSQOlVQ5A",
      "messageID": "msg_e2039e69c0015uT231F7xYoKdg"
    },
    {
      "type": "reasoning",
      "text": "思考过程...",
      "time": {
        "start": 1778657128264,
        "end": 1778657129508
      },
      "id": "prt_e2039ef48001nRObH25XwRAKPk",
      "sessionID": "ses_1dfca0700ffe1X4CRrSQOlVQ5A",
      "messageID": "msg_e2039e69c0015uT231F7xYoKdg"
    },
    {
      "type": "text",
      "text": "实际的回复内容",
      "time": {
        "start": 1778657129520,
        "end": 1778657129527
      },
      "id": "prt_e2039f430001C5xnXLzCpnCRPI",
      "sessionID": "ses_1dfca0700ffe1X4CRrSQOlVQ5A",
      "messageID": "msg_e2039e69c0015uT231F7xYoKdg"
    }
  ]
}
```

**响应字段说明**：
- `info`：消息元信息
  - `id`：消息唯一标识
  - `role`：角色（`assistant`）
  - `modelID`：实际使用的模型 ID
  - `providerID`：实际使用的提供商 ID
  - `tokens`：token 使用统计
  - `finish`：结束原因（`stop` 表示正常结束）
- `parts`：消息内容数组，按类型分类
  - `type: "text"`：实际的文本回复内容，在 `text` 字段中
  - `type: "reasoning"`：模型的思考过程
  - `type: "step-start"`：步骤开始标记

## 与 OpenAI 协议的差异

| 特性 | OpenAI | OpenCode Serve |
|------|--------|----------------|
| 会话管理 | 客户端维护 messages 数组 | 服务端维护 session 状态 |
| 创建会话 | 无需创建 | 需要先 `POST /session` |
| 发送消息 | `POST /v1/chat/completions` | `POST /session/{id}/message` |
| 请求格式 | `{"messages": [{"role": "...", "content": "..."}]}` | `{"parts": [{"type": "text", "text": "..."}]}` |
| 模型参数 | `"model": "gpt-4"` | `"model": {"providerID": "...", "modelID": "..."}` |
| 响应格式 | `{"choices": [{"message": {"content": "..."}}]}` | `{"info": {...}, "parts": [{"type": "text", "text": "..."}]}` |
| 流式响应 | SSE 格式 | 暂不支持（或需使用其他端点） |

## 获取模型列表

```
GET /config/providers
```

**响应用途**：
- 查看所有可用的 provider 及其模型
- 获取正确的 `providerID` 和 `modelID`

**示例模型配置**：
```json
{
  "providers": [
    {
      "id": "su8-5-10",
      "models": {
        "gpt-5.3-codex": {
          "id": "gpt-5.3-codex",
          "name": "su8-gpt5.3",
          "providerID": "su8-5-10"
        }
      }
    }
  ],
  "default": {
    "su8-5-10": "gpt-5.3-codex"
  }
}
```

## 认证

如需密码保护，设置环境变量：

```bash
OPENCODE_SERVER_PASSWORD=your-password opencode serve
# 用户名默认为 opencode，或设置 OPENCODE_SERVER_USERNAME 覆盖
```

## 注意事项

1. **Session 状态**：OpenCode serve 在服务端维护完整的对话历史，客户端无需重复发送历史消息
2. **模型格式**：model 参数必须是 `{providerID, modelID}` 对象，不是简单的字符串
3. **Provider ID**：模型可能属于不同的 provider（如 `su8-5-10`、`right-5-10`、`opencode` 等），需要通过 `/config/providers` 查询确认
4. **流式响应**：当前 `/session/{id}/message` 端点返回完整响应，如需流式需查看 SSE 相关端点
