# ikapb

ikapb 是 2027 年保存站绘版活动的像素画板服务端。

> 后端源码会在本次绘版活动结束后开源。在那之前，这个分支只包含协议文档和示例客户端。2026 年活动的后端源码在 [`2026`](https://github.com/Ikaleio/ikapb/tree/2026) 分支。

活动地址：https://pb.ika.zip

## 玩法

- **距离冷却**：每个 uid 有一支画笔，停在上次成功落笔的位置。在位置 `i` 再次落笔的最早时间是 $t + c_0 + k \cdot d$，`d` 是到上次落笔位置的曼哈顿距离（默认 $c_0 = 10$ 秒、$k = 1$ 秒/格）。就近作画便宜，长途调动昂贵。
- **战争迷雾**：没有持续的实时视野。每次成功落笔会回传一次以落笔点为中心、曼哈顿距离 ≤ 2 的 13 个像素的实时画面；其余区域只能看到延迟 10 分钟的公共画面。

以上是默认值，管理员可以随时调整，实际值以 `GET /api/meta` 和连接后收到的 `RULES` 消息为准。

## 文档

[`docs/protocol.md`](docs/protocol.md) 完整描述规则、HTTP 接口和 WebSocket 二进制协议的每个字节，照着它就能写出客户端。

## 获取 Paint Token

1. 在活动网页的「Paint Token」页做剪贴板验证：发一个内容为固定文本的洛谷云剪贴板，提交链接并通过人机验证。验证成功后网页会显示 Paint Token 和 Access Key。
2. 脚本用 Access Key 换新的 Paint Token，不需要人机验证：

   ```bash
   curl -X POST https://pb.ika.zip/api/paint-token \
     -H 'content-type: application/json' \
     -d '{"access_key": "<43 个字符>"}'
   # {"uid": 123, "paint_token": "0f8c2a5e-…"}
   ```

   换到新的 Paint Token 后，这个 uid 原来的 Paint Token 立即失效。

剪贴板链接就是凭据：拿到链接的人都能领这个 uid 的 Paint Token，只交给替你作画的人。

## 示例客户端

用一个 Paint Token 把矩形涂成一种颜色：

```bash
# Python ≥ 3.14
uv run examples/python/painter.py --server https://pb.ika.zip \
  --paint-token <uuid> --rect 10 10 8 8 --color '#ff6600'

# TypeScript，Node.js ≥ 24.6 或 Bun
node examples/ts/painter.ts --server https://pb.ika.zip \
  --paint-token <uuid> --rect 10,10,8,8 --color '#ff6600'
```

## 许可证

本分支的文档和示例代码以 [MIT](LICENSE) 许可发布。
