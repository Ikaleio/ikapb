# ikapb API 文档

本文档详细描述了 ikapb 画板后端提供的所有 API 接口，包括公共 HTTP API、WebSocket 实时通信协议以及管理 API。

---

## 目录

- [公共 HTTP API](#公共-http-api)
  - [GET /api/paintboard/getboard - 获取画板数据](#get-apipaintboardgetboard---获取画板数据)
  - [POST /api/auth/gettoken - 获取认证 Token](#post-apiauthgettoken---获取认证-token)
- [WebSocket API](#websocket-api)
  - [连接建立](#连接建立)
  - [消息格式](#消息格式)
  - [连接生命周期](#连接生命周期)
- [管理 API](#管理-api)
  - [认证说明](#认证说明)
  - [填充接口 /admin/fills](#填充接口-adminfills)
  - [Token 管理 /admin/tokens](#token-管理-admintokens)
  - [封禁管理 /admin/bans](#封禁管理-adminbans)
  - [连接统计 /admin/ws/connections](#连接统计-adminwsconnections)
  - [错误响应格式](#错误响应格式)

---

## 公共 HTTP API

### GET /api/paintboard/getboard - 获取画板数据

获取当前画板的完整像素数据。

#### 请求

```http
GET /api/paintboard/getboard
GET /api/paintboard/getboard?enc=zstd
```

#### URL 参数

| 参数  | 值     | 说明                         |
| ----- | ------ | ---------------------------- |
| `enc` | `zstd` | 可选，启用 Zstd 压缩返回数据 |

#### 响应头

| Header           | 值                         | 说明                               |
| ---------------- | -------------------------- | ---------------------------------- |
| `Content-Type`   | `application/octet-stream` | 二进制数据流                       |
| `X-Width`        | `1000`                     | 画板宽度（像素）                   |
| `X-Height`       | `600`                      | 画板高度（像素）                   |
| `X-Pixel-Format` | `RGB24`                    | 像素格式（每像素 3 字节）          |
| `X-Encoded`      | `zstd`                     | 仅当 `enc=zstd` 时返回，表示已压缩 |
| `ETag`           | `"rev-{tick_index}"`       | 版本标识，用于条件请求             |

#### 响应体

- **未压缩时**：原始 RGB24 像素数据，共 **1,800,000 字节**（1000 × 600 × 3）
  - 行优先顺序（从左到右，从上到下）
  - 每个像素 3 字节：`[R, G, B]`
- **压缩时**：Zstd 压缩后的像素数据

#### 条件请求

支持 `If-None-Match` 请求头。若 ETag 匹配，返回 `304 Not Modified`，无响应体。

#### 示例

```bash
# 获取原始画板数据
curl -o board.raw http://localhost:8775/api/paintboard/getboard

# 获取压缩画板数据
curl -H "If-None-Match: \"rev-12345\"" http://localhost:8775/api/paintboard/getboard?enc=zstd
```

---

### POST /api/auth/gettoken - 获取认证 Token

获取用于绘画操作的认证 Token。`/api/paintboard/gettoken` 是该端点的别名，功能完全相同。

#### 请求

```http
POST /api/auth/gettoken
Content-Type: application/json

{
    "uid": <number>,
    "access_key": "<string>"
}
```

| 字段         | 类型   | 必填 | 说明                                                |
| ------------ | ------ | ---- | --------------------------------------------------- |
| `uid`        | number | ✓    | 用户唯一标识符                                      |
| `access_key` | string | ✓    | 访问密钥（当 `no_access_key_verify=true` 时可为空） |

#### 成功响应 (200)

```json
{
	"code": 200,
	"data": {
		"token": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
	}
}
```

#### 错误响应 (403)

```json
{
	"code": 403,
	"data": {
		"error": "<error_type>"
	}
}
```

| 错误类型             | 说明                 |
| -------------------- | -------------------- |
| `MISSING_ACCESS_KEY` | 访问密钥为空         |
| `INVALID_ACCESS_KEY` | 访问密钥无效         |
| `UID_MISMATCH`       | UID 与访问密钥不匹配 |
| `BANNED_UID`         | 用户已被封禁         |
| `UNKNOWN_ERROR`      | 服务器内部错误       |

#### 说明

- 每个 UID 同时只能持有一个有效 Token
- 重复请求会生成新 Token 并替换旧 Token
- 被封禁的 UID 无法获取 Token
- Token 的冷却时间默认使用全局配置 `global_cooldown_ms`

---

## WebSocket API

WebSocket 用于实时双向通信，支持绘画操作和画板更新广播。

### 连接建立

#### 端点

```
ws[s]://{host}/ws[?params]
```

#### URL 参数

| 参数        | 值                 | 默认值 | 说明                                      |
| ----------- | ------------------ | ------ | ----------------------------------------- |
| `enc`       | `none`, `zstd`     | `none` | 服务端到客户端的压缩模式                  |
| `dict`      | `<id>`             | -      | Zstd 字典标识符（仅当 `enc=zstd` 时有效） |
| `readonly`  | `1`                | -      | 只读模式，仅接收广播，不允许绘画          |
| `writeonly` | `1`                | -      | 只写模式，仅允许绘画，不接收广播          |
| `watch_ips` | 逗号分隔的 IP 列表 | -      | 管理员专用，过滤指定 IP 的绘画事件        |
| `admin_key` | `<string>`         | -      | 管理员凭证（当无 `X-Admin-Key` 头时使用） |
| `tps`       | `<number>`         | -      | 仅供客户端参考，服务端忽略                |

#### 模式规则

- 默认为**读写模式**（可绘画、可接收广播）
- `readonly=1`：只读模式
- `writeonly=1`：只写模式
- `readonly=1` 和 `writeonly=1` 互斥，同时设置返回 HTTP 400

#### 字典协商

当 `enc=zstd` 且 `dict=<id>` 存在：

1. 服务端在升级后立即发送 `0xF6` 字典帧
2. 后续所有 `0xF7` 压缩帧使用该字典

#### 管理员监控过滤

当具有有效管理员权限且设置 `watch_ips` 参数时：

- 仅转发指定 IP 的绘画事件
- 连接自身 IP 自动加入允许列表
- 强制使用非压缩 `0xFA` 格式

---

### 消息格式

#### 操作码总览

| 操作码 | 方向 | 名称           | 长度      |
| ------ | ---- | -------------- | --------- |
| `0xFE` | C→S  | 绘画请求       | 31 字节   |
| `0xFF` | S→C  | 绘画结果 (ACK) | 6 字节    |
| `0xFA` | S→C  | 绘画事件       | 8 字节    |
| `0xF7` | S→C  | 压缩 Tick 帧   | 18 + 载荷 |
| `0xF6` | S→C  | 字典帧         | 可变      |
| `0xFC` | S→C  | Ping           | 1 字节    |
| `0xFB` | C→S  | Pong           | 1 字节    |

---

#### 0xFE 绘画请求（客户端 → 服务端）

**长度**：31 字节（固定）

| 偏移 | 大小 | 类型   | 字段                     |
| ---- | ---- | ------ | ------------------------ |
| 0    | 1    | u8     | 操作码 (`0xFE`)          |
| 1    | 2    | u16_le | x 坐标                   |
| 3    | 2    | u16_le | y 坐标                   |
| 5    | 1    | u8     | 红色值 (R)               |
| 6    | 1    | u8     | 绿色值 (G)               |
| 7    | 1    | u8     | 蓝色值 (B)               |
| 8    | 3    | u24_le | UID（低 24 位）          |
| 11   | 16   | bytes  | Token（UUID 二进制格式） |
| 27   | 4    | u32_le | 客户端请求 ID            |

**UID 编码说明**：

- UID 为 32 位无符号整数，但仅传输低 24 位
- 字节 8-10 以小端序存储：`[uid & 0xFF, (uid >> 8) & 0xFF, (uid >> 16) & 0xFF]`
- 最大可表示 UID：16,777,215 (2²⁴ - 1)

**验证规则**：

1. 数据包长度必须恰好为 31 字节
2. `x` 范围：`[0, 999]`
3. `y` 范围：`[0, 599]`
4. Token 必须存在且有效
5. Token 的 UID（低 24 位）必须与提供的 UID 匹配
6. 连接模式不能是 `readonly`

---

#### 0xFF 绘画结果（服务端 → 客户端）

**长度**：6 字节（固定）

| 偏移 | 大小 | 类型   | 字段            |
| ---- | ---- | ------ | --------------- |
| 0    | 1    | u8     | 操作码 (`0xFF`) |
| 1    | 4    | u32_le | 客户端请求 ID   |
| 5    | 1    | u8     | 状态码          |

**状态码**：

| 码     | 名称          | 说明                           |
| ------ | ------------- | ------------------------------ |
| `0xEF` | OK            | 成功入队，将在下一个 tick 提交 |
| `0xEE` | COOLING       | 冷却期间，需等待后重试         |
| `0xED` | INVALID_TOKEN | Token 无效或已过期             |
| `0xEC` | BAD_REQUEST   | 请求格式错误                   |
| `0xEB` | FORBIDDEN     | 操作被禁止（如用户被封禁）     |
| `0xEA` | SERVER_ERROR  | 服务器内部错误                 |

---

#### 0xFA 绘画事件（服务端 → 客户端）

**长度**：8 字节（固定）

| 偏移 | 大小 | 类型   | 字段            |
| ---- | ---- | ------ | --------------- |
| 0    | 1    | u8     | 操作码 (`0xFA`) |
| 1    | 2    | u16_le | x 坐标          |
| 3    | 2    | u16_le | y 坐标          |
| 5    | 1    | u8     | 红色值 (R)      |
| 6    | 1    | u8     | 绿色值 (G)      |
| 7    | 1    | u8     | 蓝色值 (B)      |

**说明**：

- 表示已提交的像素变更
- 广播给所有非 `writeonly` 连接
- 同一 tick 内的多个事件可合并在一个 WebSocket 帧中发送

---

#### 0xF7 压缩 Tick 帧（服务端 → 客户端）

**头长度**：18 字节
**载荷**：Zstd 压缩的 `0xFA` 事件序列

| 偏移 | 大小 | 类型   | 字段                     |
| ---- | ---- | ------ | ------------------------ |
| 0    | 1    | u8     | 操作码 (`0xF7`)          |
| 1    | 8    | u64_le | Tick 索引                |
| 9    | 2    | u16_le | 分片索引（始终为 0）     |
| 11   | 2    | u16_le | 分片总数（始终为 1）     |
| 13   | 4    | u32_le | 解压后载荷长度           |
| 17   | 1    | u8     | 标志位（保留，当前为 0） |
| 18   | N    | bytes  | Zstd 压缩载荷            |

**说明**：

- 仅当 `enc=zstd` 时发送
- 载荷解压后为连续的 `0xFA` 绘画事件

---

#### 0xF6 字典帧（服务端 → 客户端）

| 偏移 | 大小 | 类型   | 字段                               |
| ---- | ---- | ------ | ---------------------------------- |
| 0    | 1    | u8     | 操作码 (`0xF6`)                    |
| 1    | 4    | u32_le | 字典 ID（与 `dict=<id>` 参数对应） |
| 5    | 4    | u32_le | 字典字节长度 N                     |
| 9    | N    | bytes  | 原始字典内容                       |

**说明**：

- WebSocket 升级后立即发送（若协商了字典）
- 仅发送一次

---

#### 0xFC Ping（服务端 → 客户端）

**长度**：1 字节

| 偏移 | 大小 | 类型 | 字段            |
| ---- | ---- | ---- | --------------- |
| 0    | 1    | u8   | 操作码 (`0xFC`) |

---

#### 0xFB Pong（客户端 → 服务端）

**长度**：1 字节

| 偏移 | 大小 | 类型 | 字段            |
| ---- | ---- | ---- | --------------- |
| 0    | 1    | u8   | 操作码 (`0xFB`) |

**要求**：客户端收到 `0xFC` 后必须立即响应 `0xFB`

---

### 连接生命周期

#### 出站队列限制

- **上限**：32 帧/连接
- **行为**：超过限制时，连接以 close code 1011 终止

#### ACK 缓冲区限制

- **上限**：`max_ack_buffer_bytes`（可配置）
- **行为**：超过限制时，连接终止

#### WebSocket 关闭码

| 码   | 名称             | 原因                                      |
| ---- | ---------------- | ----------------------------------------- |
| 1001 | Going Away       | Ping 超时（未收到 Pong 响应）             |
| 1002 | Protocol Error   | 未知操作码、意外的 Pong、不完整的数据包   |
| 1008 | Policy Violation | 保留（IP 连接限制通过 HTTP 429 拒绝升级） |
| 1009 | Message Too Big  | 保留                                      |
| 1011 | Internal Error   | 慢消费者/出站队列溢出/服务器错误          |

#### 未知操作码处理

服务端收到未知的客户端操作码时：

1. 立即关闭连接
2. 发送 close code `1002`（Protocol Error）

---

## 管理 API

### 认证说明

- 所有 `/admin/*` 端点需要请求头 `X-Admin-Key` 与配置项 `admin_manage_key` 匹配
- 若 `admin_manage_key` 为空或缺失，所有管理请求返回 **HTTP 401**
- 服务端不额外进行速率限制或审计日志，建议在反向代理层实现

### 配置参数

| 参数                   | 类型   | 默认值    | 说明                      |
| ---------------------- | ------ | --------- | ------------------------- |
| `admin_manage_key`     | string | `""`      | 非空时启用管理端点        |
| `admin_max_blob_bytes` | u64    | 5,242,880 | 上传/下载图片的最大字节数 |

---

### 填充接口 /admin/fills

#### POST /admin/fills

填充指定区域为纯色或图片。

**请求**：

```json
{
  "area": {
    "x": <u32>,
    "y": <u32>,
    "width": <u32>,
    "height": <u32>
  },
  "source": {
    "kind": "color" | "blob" | "url",
    "value": "#RRGGBB" | "<base64>" | "https://...",
    "format": "png" | "jpg" | "jpeg" | "webp"
  },
  "filters": {
    "scale": {
      "mode": "contain" | "cover" | "fill",
      "width": <u32>,
      "height": <u32>
    },
    "grayscale": <bool>,
    "crop": {
      "x": <u32>,
      "y": <u32>,
      "width": <u32>,
      "height": <u32>
    }
  }
}
```

**字段说明**：

| 字段                | 必填 | 说明                                                                    |
| ------------------- | ---- | ----------------------------------------------------------------------- |
| `area`              | ✓    | 目标区域：`x < 1000`, `y < 600`, `x + width ≤ 1000`, `y + height ≤ 600` |
| `source.kind`       | ✓    | 源类型：`color`（纯色）、`blob`（Base64 图片）、`url`（远程图片）       |
| `source.value`      | ✓    | 颜色值（`#RRGGBB`）、Base64 编码图片、或 HTTPS URL                      |
| `source.format`     | 条件 | 图片格式，`kind` 非 `color` 时必须，支持 `png`/`jpg`/`jpeg`/`webp`      |
| `filters.scale`     | 否   | 缩放配置；不设置时默认使用 `fill` 模式缩放到 `area` 尺寸                |
| `filters.grayscale` | 否   | 是否转为灰度，默认 `false`                                              |
| `filters.crop`      | 否   | 裁切区域，在缩放后应用                                                  |

**缩放模式**：

- `fill`：强制拉伸到目标尺寸，忽略宽高比
- `contain`：保持宽高比缩放以适应目标尺寸，空白区域填充黑色
- `cover`：保持宽高比缩放以覆盖目标尺寸，居中裁切

**处理顺序**：`scale` → `crop` → `grayscale`

**约束**：处理后图片尺寸必须与 `area` 尺寸完全一致，否则返回 400。

**成功响应 (200)**：

```json
{
  "code": 200,
  "data": {
    "written": <u32>,
    "format": "rgb24"
  }
}
```

**错误码**：

- `400`：验证/解码失败
- `401`：管理密钥缺失或无效
- `413`：图片超过 `admin_max_blob_bytes` 限制
- `415`：不支持的图片格式

---

### Token 管理 /admin/tokens

#### 数据模型

```json
{
  "uid": <i64>,
  "token": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
  "cooldown_ms": <i64>,
  "issued_at_ms": <i64>,
  "ip": "<string>" | null,
  "last_paint_at_ms": <i64> | null
}
```

**字段约束**：

- `uid`：主键，唯一标识
- `token`：UUIDv4 格式
- `cooldown_ms`：`≥ -1`，`-1` 表示使用全局冷却时间
- `last_paint_at_ms`：设为 `null` 会清除冷却缓存

---

#### GET /admin/tokens

列出所有 Token。

**请求参数**：

| 参数        | 类型 | 说明                |
| ----------- | ---- | ------------------- |
| `uid`       | i64  | 按 UID 精确匹配过滤 |
| `page`      | u32  | 页码（从 0 开始）   |
| `page_size` | u32  | 每页条数            |

**响应**：

```json
{
  "code": 200,
  "data": {
    "items": [Token],
    "total": <u64>
  }
}
```

---

#### POST /admin/tokens

创建新 Token。

**请求**：

```json
{
  "uid": <i64>,
  "cooldown_ms": <i64>,
  "issued_at_ms": <i64>,
  "ip": "<string>" | null,
  "last_paint_at_ms": <i64> | null
}
```

**说明**：

- `issued_at_ms` 可选，默认为当前时间
- 自动生成 UUIDv4 作为 Token
- 若 UID 已被封禁，返回 403
- 若 UID 已有 Token，替换旧 Token 并清除其冷却缓存

**响应**：

```json
{
  "code": 200,
  "data": {
    "uid": <i64>,
    "token": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
    "cooldown_ms": <i64>,
    "issued_at_ms": <i64>,
    "ip": null,
    "last_paint_at_ms": null
  }
}
```

---

#### GET /admin/tokens/{uid}

获取指定 UID 的 Token 详情。

**响应**：成功返回 Token 对象，不存在返回 **404**。

---

#### PUT /admin/tokens/{uid}

完整替换 Token 记录。

**请求**：与 POST 相同（不含 `uid`），路径中的 `uid` 为准。

**说明**：省略的可选字段将变为 `null`。

---

#### PATCH /admin/tokens/{uid}

部分更新 Token 记录。

**请求**：仅包含需要更新的字段。

**说明**：

- 省略的字段保持不变
- `uid` 不可修改
- 若提供 `last_paint_at_ms`，更新冷却缓存（`null` 表示清除）

---

#### DELETE /admin/tokens/{uid}

删除指定 UID 的 Token。

**响应**：返回被删除的 Token 对象，或 `null`（若不存在）。

---

### 封禁管理 /admin/bans

#### 数据模型

```json
{
  "uid": <i64>,
  "reason": "<string>" | null,
  "created_at_ms": <i64>,
  "expire_at_ms": <i64> | null
}
```

**封禁状态判定**：当 `expire_at_ms` 为 `null` 或 `expire_at_ms > 当前时间戳` 时，封禁处于**活跃状态**。

---

#### GET /admin/bans

列出封禁记录。

**请求参数**：

| 参数          | 类型 | 说明           |
| ------------- | ---- | -------------- |
| `uid`         | i64  | 按 UID 过滤    |
| `active_only` | bool | 仅显示活跃封禁 |

**响应**：

```json
{
  "code": 200,
  "data": {
    "items": [BanRecord]
  }
}
```

---

#### POST /admin/bans

创建或更新封禁记录。

**请求**：

```json
{
  "uid": <i64>,
  "reason": "<string>",
  "expire_at_ms": <i64> | null
}
```

| 字段           | 必填 | 说明                          |
| -------------- | ---- | ----------------------------- |
| `uid`          | ✓    | 被封禁的用户 ID               |
| `reason`       | 否   | 封禁原因                      |
| `expire_at_ms` | 否   | 过期时间，`null` 表示永久封禁 |

**说明**：

- `created_at_ms` 自动设为当前时间
- 若 UID 已有封禁记录，将被替换

**响应**：

```json
{
  "code": 200,
  "data": {
    "uid": <i64>,
    "reason": "<string>",
    "created_at_ms": <i64>,
    "expire_at_ms": <i64> | null
  }
}
```

---

#### DELETE /admin/bans/{uid}

删除封禁记录。

**响应**：返回被删除的封禁记录，或 `null`（若不存在）。操作幂等。

---

#### 封禁执行机制

- WebSocket 绘画验证：被封禁 UID 的绘画请求返回 `ACK_FORBIDDEN` (0xEB)
- Token 接口：`/api/auth/gettoken` 和 `/admin/tokens` POST 操作拒绝为被封禁 UID 发放 Token（返回 403）

---

### 连接统计 /admin/ws/connections

#### GET /admin/ws/connections

获取当前 WebSocket 连接统计（按 IP 分组）。

**响应**：

```json
{
  "code": 200,
  "data": {
    "items": [
      {
        "ip": "<string>",
        "connections": <u64>
      }
    ]
  }
}
```

| 字段          | 说明                              |
| ------------- | --------------------------------- |
| `ip`          | 客户端 IP 地址（规范化格式）      |
| `connections` | 该 IP 当前活跃的 WebSocket 连接数 |

**说明**：返回请求时刻的内存快照，结果可能在响应后发生变化；即使未开启限制（`max_connections_per_ip` 未设置或为 `0`），也会统计连接数。

---

### 错误响应格式

所有管理 API 使用统一的 JSON 响应格式：

```json
{
  "code": <u16>,
  "data": <object>
}
```

**常见错误码**：

| HTTP 状态码 | code | 说明                                    |
| ----------- | ---- | --------------------------------------- |
| 200         | 200  | 成功                                    |
| 400         | 400  | 请求验证失败，`data.error` 包含错误说明 |
| 401         | 401  | 管理密钥缺失或无效                      |
| 403         | 403  | 操作被禁止（如为被封禁用户创建 Token）  |
| 404         | 404  | 资源不存在                              |
| 409         | 409  | 冲突（如 UID 唯一性）                   |
| 413         | 413  | 请求体过大                              |
| 415         | 415  | 不支持的媒体类型                        |
