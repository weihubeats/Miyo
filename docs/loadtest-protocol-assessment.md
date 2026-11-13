# 压测协议适配评估：WebSocket / SSE / gRPC 流式

> 结论先行：三者都不能套用现有 HTTP 压测的「worker 领任务 → 单发 → 采样」
> 循环，各自需要独立的连接模型与指标口径。本文档给出差异分析与实施拆解，
> 实现留作后续专项（本轮仅落地 HTTP 增强，见 `crates/miyo-test/src/load.rs`）。

## 0. 现有 HTTP 引擎的模型边界

`run_load_with` 的核心假设（load.rs）：
- 一次压测单元 = 一个独立 HTTP 请求（连接由 reqwest 池内部复用）；
- 计时口径 = 请求发出到响应截断读完（单往返）；
- 并发 = 同时在途的请求数（固定 worker 池 + 全局槽位限速）。

以下三种协议都打破其中至少两条假设。

## 1. WebSocket 压测

**连接模型差异**：压测单元从「一个请求」变为「一条持久连接上的 N 次消息往返」。
每个虚拟用户（VU）持有一条长连接，复用 `miyo-http::ws_client` 的连接层
（tokio-tungstenite 已是 miyo-http 依赖），但压测需要的是**独立连接池**
（现有 ws_client 是调试会话语义：单连接 + 事件转发，无多 VU 编排）。

指标口径必须改：
- 连接建立耗时与消息往返耗时（RTT）分开统计——用户关心的 P99 是 RTT；
- 「成功」定义可选：连接建立成功 / 消息回包匹配 / 收到任意帧；
- 断线重连计数是核心错误类别（HTTP 里没有对应物）。

实施拆解（估 3~5 天）：
1. `miyo-http` 增加无状态 WS 客户端构造（不走调试会话注册表）；
2. `miyo-test` 新增 `load_ws.rs`：VU 任务模型（1 连接/VU，消息循环内计时），
   复用 LoadResult 聚合骨架 + 直方图；QPS 语义变为「消息发送速率」；
3. tauri command `load_test_ws` + 前端面板模式切换；
4. 取消语义：断开所有 VU 连接（现有 CancellationToken 可复用）。

## 2. SSE 压测

**连接模型差异**：请求是普通 HTTP GET，但响应是**无界流**——「完成」没有
自然终点。现有引擎按响应读完计时会得到无意义的天文数字。

指标口径必须改：
- 首字节时间（TTFB）与事件到达速率（events/s）是主指标；
- 计时截点 = 建流成功（收到 200 + text/event-stream 头）而非流结束；
- 采样单元 = 一次流建立 + 持续 N 秒的事件计数。

实施拆解（估 2~3 天，最轻）：
1. `miyo-test` 新增 `load_sse.rs`：VU = 长驻 GET + 逐事件计数（复用 reqwest
   `bytes_stream`，HTTP 引擎已有流式基建）；
2. LoadResult 扩展 `events_total / events_rps / stream_uptime` 字段；
3. 服务端事件速率曲线作为前端图表数据源。

## 3. gRPC 流式压测

**连接模型差异**：`miyo-grpc` 目前是**调试语义**的单次动态调用
（prost-reflect 免生成代码 + protox 运行时编译），没有流式客户端——
server/client/duplex streaming 三种方法形态都没有压测语义的封装。

指标口径必须改：
- unary ≈ 现有 HTTP 模型（可直接复用）；
- server-streaming：计时截点 = 首消息到达；指标含消息吞吐（msg/s）；
- client/duplex streaming：双向编排，消息序号配对复杂度最高。

实施拆解（估 5~8 天，最重）：
1. `miyo-grpc` 增加流式调用封装（tonic `Streaming` 句柄管理 + 反射描述符）；
2. `miyo-test` 新增 `load_grpc.rs`：unary 直接复用 HTTP 骨架；
   streaming 用「VU = 一条流 + N 消息」模型；
3. proto 文件选择复用现有 proto_files 表（前端已有管理 UI）。

## 4. 共性结论

- 三者的聚合/报告层（percentile、直方图、错误率熔断、资源采样）与 HTTP
  引擎可共享——应抽成 `load::aggregate` 公共模块再实现各自 VU 模型；
- 各自的 QPS 语义不同（HTTP 请求 / WS 消息 / SSE 流建立 + gRPC 消息），
  配置 UI 需按模式切换语义标签而非简单复用字段；
- 取消与错误率熔断逻辑可原样复用（CancellationToken 子令牌 + 阈值检查）。
