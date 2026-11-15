# ikapb

ikapb 是一个高性能的像素画板后端，使用 Rust + Tokio 构建。它面向大规模并发绘制场景，采用 tick-based 的一致性模型，并支持 WebSocket 与 HTTP 的多种协议能力。

Rust 后端位于 [`paintboard/`](paintboard/)，Vite 画板和管理界面位于 [`frontend/`](frontend/)。

## 特性

- **Tick 驱动的状态机**：所有像素写入按照全局 tick 推进，同一 tick 内遵循“后画覆盖前画”。
- **高吞吐写入链路**：读写连接分离，入队与提交解耦，可承载 200k+ paint/s 的单节点写入。
- **协议兼容性**：兼容现有的 0xFA/0xFF/0xFC/0xFB 报文，并扩展了 0xF7（压缩 tick 帧）、0xF6（字典分发）。
- **高效广播与压缩**：对每个 tick 的像素集合使用 zstd 压缩，并支持字典分发以提升压缩比。
- **持久化保障**：定期对画板进行快照，可选启用 WAL；Token 与冷却状态存储于 SQLite 并批量刷新。
- **可热更新参数**：TPS、帧大小、压缩设置等核心参数可通过热更新机制在运行时调整。

完整的协议与实现细节可参考 [`spec/`](spec/) 目录下的规范文档：

- [`board.spec.md`](spec/board.spec.md): 画布状态、像素格式、持久化
- [`tick.spec.md`](spec/tick.spec.md): Tick 协调、写集合、一致性模型
- [`websocket-protocol.spec.md`](spec/websocket-protocol.spec.md): WebSocket 二进制协议
- [`token.spec.md`](spec/token.spec.md): Token 认证与冷却系统
- [`compression.spec.md`](spec/compression.spec.md): Zstd 压缩与字典管理
- [`config.spec.md`](spec/config.spec.md): 配置参数与热更新

## API 文档

完整的 API 接口文档请参阅 **[API.md](API.md)**，包含：

- 公共 HTTP API（获取画板、获取 Token）
- WebSocket 实时通信协议
- 管理 API（填充、Token 管理、封禁管理）

## 环境要求

- Rust 1.78+（建议使用 `rustup` 管理工具链）
- Zstandard 原生库（随 `zstd` crate 的 `bundled` 特性自动构建）
- SQLite（通过 `rusqlite` 的 `bundled` 依赖一同构建）

## 快速开始

### 启动服务器

```bash
# 获取代码
git clone https://github.com/Ikaleio/ikapb
cd ikapb

# 构建与运行
cargo run
```

服务器启动后会监听在 `http://localhost:8775`，你可以：

1. **访问内置 Web 界面**: 打开浏览器访问 `http://localhost:8775` 即可看到像素画板界面
2. **获取 Token**: 点击界面上的「获取 Token」按钮自动获取并连接
3. **开始绘画**: 选择颜色，点击画布上的任意位置开始绘制！

### 启动 Vite 前端

启动 Rust 后端后，在另一个终端从仓库根目录运行以下命令。

```bash
cd frontend
bun install --frozen-lockfile
VITE_PAINTBOARD_BASE_URL=http://localhost:8775 bun dev
```

画板页面在 `http://localhost:3000/`，管理界面在 `http://localhost:3000/admin`。构建和部署说明见[前端 README](frontend/README.md)。

### 使用命令行客户端

默认会加载 `paintboard/config/default.toml`。可通过设置环境变量 `PAINTBOARD_CONFIG` 指向自定义配置文件：

```bash
PAINTBOARD_CONFIG=/path/to/custom.toml cargo run
```

服务器启动后将：

1. 初始化日志与 Prometheus 指标。
2. 安装热更新信号处理器（用于在运行时重新加载配置）。
3. 构建 HTTP/WS 路由并初始化内存画板。
4. 阻塞等待 `Ctrl+C` 以优雅关闭。

## 配置说明

`paintboard/config/default.toml` 提供了所有运行参数，服务启动时默认加载。配置文件按功能分组，详见 [`spec/config.spec.md`](spec/config.spec.md)。

### 核心参数

| 字段                   | 说明                                                       |
| ---------------------- | ---------------------------------------------------------- |
| `bind_address`         | HTTP/WS 监听地址，例如 `0.0.0.0:8775`。                    |
| `tps`                  | 每秒推进的 tick 数。数值越大，提交延迟越低，CPU 占用越高。 |
| `no_access_key_verify` | 是否跳过 Access-Key 校验，仅建议在本地开发使用。           |
| `background_color`     | 空白画板的默认背景色（十六进制 RGB）。                     |
| `data_dir`             | 运行期生成文件（快照、字典、数据库等）的根目录。           |
| `sqlite_path`          | Token 与冷却状态 SQLite 数据库文件路径。                   |

### 网络配置 `[network]`

| 字段                     | 说明                                               |
| ------------------------ | -------------------------------------------------- |
| `trusted_proxies`        | 受信任的反向代理 IP 列表，用于解析真实客户端地址。 |
| `max_connections_per_ip` | 每个 IP 的最大 WebSocket 连接数（0 = 不限制）。    |

### WebSocket 配置 `[websocket]`

| 字段                    | 说明                                                 |
| ----------------------- | ---------------------------------------------------- |
| `ws_send_period_ms`     | WebSocket 出站粘包周期（毫秒），降低会缩短广播延迟。 |
| `max_ack_buffer_bytes`  | ACK 缓冲区绝对上限（字节），超过则强制关闭连接。     |
| `max_client_latency_ms` | 客户端最大允许延迟（毫秒），用于慢消费者保护。       |

### 持久化配置 `[persistence]`

| 字段                    | 说明                                                   |
| ----------------------- | ------------------------------------------------------ |
| `persist_board`         | 是否启用快照/WAL 落盘，调试环境可关闭以减少 I/O。      |
| `snapshot_interval_sec` | 周期性生成画板快照的时间间隔（秒）。                   |
| `snapshot_retain`       | 最多保留的快照数量，超出后会删除最旧快照。             |
| `wal_flush_every_ticks` | 将冻结写集合刷入 WAL 的 tick 周期；设为 0 可禁用 WAL。 |

### Token 配置 `[token]`

| 字段                         | 说明                                                           |
| ---------------------------- | -------------------------------------------------------------- |
| `global_cooldown_ms`         | 全局默认冷却时长（毫秒），token 的 cooldown_ms=-1 时使用此值。 |
| `cooldown_flush_interval_ms` | Token 冷却状态批量写入 SQLite 的周期（毫秒）；0 表示禁用。     |

### Zstd 压缩配置 `[zstd]`

| 字段                          | 说明                                                       |
| ----------------------------- | ---------------------------------------------------------- |
| `zstd_level`                  | 广播压缩使用的 zstd 级别，0 表示自适应，1–22 为固定级别。  |
| `zstd_dictionary_path`        | zstd 字典文件路径；启用采样时退出服务会在此生成/替换字典。 |
| `zstd_dictionary_max_bytes`   | 训练生成的 zstd 字典目标大小（字节）。                     |
| `zstd_sampling_enabled`       | 是否开启广播样本采集；开启后退出服务时自动训练字典。       |
| `zstd_sampling_dir`           | 样本输出目录，留空时默认为 `data_dir/zstd-samples`。       |
| `zstd_sampling_tick_interval` | 每隔多少个 tick 采集一次样本，1 表示每个 tick。            |
| `zstd_sampling_max_samples`   | 最多保留的样本数量，达到上限后丢弃新样本。                 |

你可以复制该文件，调整需要的参数，并通过设置环境变量 `PAINTBOARD_CONFIG=/path/to/custom.toml` 指向自定义配置。热更新流程会在收到信号时重新加载该文件，详情见 [`spec/config.spec.md`](spec/config.spec.md)。

## 发布与部署建议

- 使用 systemd 或容器化方式托管服务，确保为二进制提供 `data_dir`、`sqlite_path` 等可写目录。
- 配置 Prometheus 抓取指标，辅助监控 TPS、队列长度等运行状态。
- 若部署于公网，请在上层（例如 Nginx、Cloudflare）终止 TLS，或为服务添加 TLS 终端。

## 许可证

本项目采用 [MIT License](LICENSE)。
