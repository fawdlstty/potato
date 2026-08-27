# DeepSeek Harness (DSH / DSF) API Gateway Protocol

This document records the local client protocol for the DeepSeek Harness Web
profile. The user-facing provider name in this repository is `Dsf`, while the
upstream project and packages use `dsh`.

## Sources

- GitHub repository: https://github.com/deepseek-ai/deepseek-harness
- NPM package: `@deepseek-ai/dsh@0.0.1-rc.1`
- NPM package: `@deepseek-ai/dsh-host-apiproxy@0.0.1-rc.1`
- NPM package: `@deepseek-ai/dsh-client-connection@0.0.1-rc.1`
- NPM package: `@deepseek-ai/dsh-session@0.0.1-rc.1`
- NPM package: `@deepseek-ai/dsh-llm@0.0.1-rc.1`

Important source files in the published packages:

- `@deepseek-ai/dsh-host-apiproxy/README.md`
- `@deepseek-ai/dsh-host-apiproxy/lib/types/api/rpc.d.ts`
- `@deepseek-ai/dsh-host-apiproxy/lib/types/api/rpc-map.d.ts`
- `@deepseek-ai/dsh-host-apiproxy/lib/types/api/sessions.d.ts`
- `@deepseek-ai/dsh-host-apiproxy/lib/types/api/llm.d.ts`
- `@deepseek-ai/dsh-host-apiproxy/lib/types/api/events.d.ts`
- `@deepseek-ai/dsh-client-connection/README.md`
- `@deepseek-ai/dsh-session/lib/types/types.d.ts`
- `@deepseek-ai/dsh-llm/lib/types/types.d.ts`

## Transport

The browser/local Web carrier uses HTTP for client-to-server unary calls and a
WebSocket downlink for server-to-client event streams.

The WebSocket downlink is authoritative for network clients:

- `GET /api/events.mux` and `GET /api/events.host` are WebSocket upgrade
  endpoints.
- The client sends no application messages on these sockets.
- Ordinary network `GET` requests to those paths return `426`; SSE exists only
  inside the in-process fetch carrier.

HTTP calls must use `POST`, path `/api/<method>`, and
`content-type: application/json`.

## Wire Envelope

Client request body for `POST /api/<method>`:

```json
{
  "type": "client-request",
  "rpcId": "client-generated-id",
  "method": "session.prompt",
  "payload": {}
}
```

Server response body:

```json
{
  "type": "server-response",
  "rpcId": "client-generated-id",
  "result": {
    "ok": true,
    "value": {}
  }
}
```

Business errors use the response `result` error branch:

```json
{
  "type": "server-response",
  "rpcId": "client-generated-id",
  "result": {
    "ok": false,
    "error": {
      "code": "model-unavailable",
      "message": "model unavailable",
      "details": {
        "provider": "deepseek-official",
        "model": "deepseek-chat"
      }
    }
  }
}
```

WebSocket downstream frames are `server-request` envelopes. The `method` field
is the frame payload's `type`.

```json
{
  "type": "server-request",
  "rpcId": "server-generated-id",
  "method": "session/event",
  "payload": {
    "type": "session/event",
    "sessionId": "session-id",
    "event": {}
  }
}
```

Answerable downstream frames are replied to with `POST /api/respond` using a
`client-response` envelope that echoes the server request `rpcId`.

## Methods Used By A Minimal Chat Client

### `llm.models`

Path: `POST /api/llm.models`

Payload:

```json
{}
```

Success value:

```json
{
  "groups": [
    {
      "id": "deepseek-official",
      "name": "DeepSeek",
      "models": [
        {
          "id": "deepseek-chat",
          "name": "DeepSeek Chat",
          "description": "optional"
        }
      ]
    }
  ],
  "failures": []
}
```

`groups[].id` is the provider route. `groups[].models[].id` is the provider's
model id.

### `session.create`

Path: `POST /api/session.create`

Payload:

```json
{
  "cwd": "/path/to/workspace"
}
```

`cwd` is optional; omitted means the Host cwd. `workspaceId`, `sessionId`, and
`agentPreset` are also supported upstream, but are not required for a minimal
local chat client.

Success value:

```json
{
  "sessionId": "session-id",
  "agentPreset": "optional"
}
```

### `session.models`

Path: `POST /api/session.models`

Payload:

```json
{
  "sessionId": "session-id"
}
```

Success value:

```json
{
  "current": {
    "provider": "deepseek-official",
    "model": "deepseek-chat",
    "reasoningEffort": "optional"
  },
  "routable": true,
  "groups": [],
  "failures": []
}
```

### `session.selectModel`

Path: `POST /api/session.selectModel`

Payload:

```json
{
  "sessionId": "session-id",
  "provider": "deepseek-official",
  "model": "deepseek-chat",
  "reasoningEffort": "optional"
}
```

Success value:

```json
{
  "selected": {
    "provider": "deepseek-official",
    "model": "deepseek-chat",
    "reasoningEffort": "optional"
  }
}
```

### `session.prompt`

Path: `POST /api/session.prompt`

Payload:

```json
{
  "sessionId": "session-id",
  "mode": "queue",
  "content": [
    {
      "type": "text",
      "text": "hello"
    }
  ]
}
```

`mode` is either `queue` or `steer`. A minimal client should use `queue`.

Success value:

```json
{
  "accepted": true
}
```

The assistant answer does not return in the `session.prompt` response. It is
published through `/api/events.mux`.

## Mux Frames Needed For Chat

The WebSocket `server-request.payload` is a `MuxFrame`.

### `session/subscribed`

```json
{
  "type": "session/subscribed",
  "sessionId": "session-id",
  "lastSeq": 0
}
```

### `session/event`

```json
{
  "type": "session/event",
  "sessionId": "session-id",
  "event": {
    "type": "assistant/chunk",
    "seq": 10,
    "time": 123456789,
    "data": {
      "turn": 0,
      "step": 0,
      "chunk": {
        "type": "text-delta",
        "index": 0,
        "text": "partial text"
      }
    }
  }
}
```

Relevant session event types:

- `turn/start`: opens a turn.
- `turn/end`: closes a turn. Stop reading for a prompt when this arrives for
  the target session.
- `step/start` / `step/end`: model-call boundaries.
- `user/message`: durable user-side message.
- `assistant/chunk`: token-level streaming events. For visible text, read
  `event.data.chunk` when `chunk.type == "text-delta"` and append `chunk.text`.
- `assistant/message`: assembled assistant message for the step. Visible text
  blocks are in `event.data.message.content[]` with `type == "text"`.
- `tool/call` and `tool/result`: tool activity, not assistant text.
- `stream/error`: mux stream-level failure.

### Approval Frames

`approval/requested` is answerable through `POST /api/respond`.

Response success value:

```json
{
  "sessionId": "session-id",
  "approvalId": "approval-id",
  "outcome": "rejected"
}
```

The other allowed client outcome is `allowed-once`.

### Question Frames

`question/requested` is answerable through `POST /api/respond`. To cancel an
unsupported prompt, send a `client-response` whose `result` is the `ok: false`
RPC error branch with code `cancelled`.

## Model Encoding In This Repository

For `LlmProvider::Dsf`, model identifiers are exposed as
`<provider>:<model>`, for example:

```text
deepseek-official:deepseek-chat
```

This maps to DSH's `ModelSelection { provider, model, reasoningEffort? }`.
