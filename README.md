# perception-network-status

基于 Rust 与 Windows API 的网络状态与质量监控组件，提供网络连通性、WLAN 信号与网络质量指标采样能力，并通过 N-API 供 Node.js 调用。

## 功能特性

- 网络连通性监控：监听系统网络连接变化
- WLAN 信号监控：信号质量变化与弱信号状态
- 网络质量探测：延迟（RTT）、丢包率、抖动、TCP 重传率等指标
- 后台线程持续采样，日志与回调双通道输出

## 运行环境

- Windows 平台
- Rust 与 cargo
- Node.js（N-API）

## 模块说明

- 入口与 N-API 绑定：[src/lib.rs](./src/lib.rs)
- 监控线程与消息循环：[src/monitor.rs](./src/monitor.rs)
- 网络连通性监控：[src/network.rs](./src/network.rs)
- WLAN 信号监控：[src/wlan.rs](./src/wlan.rs)
- 网络质量探测：[src/network_quality.rs](./src/network_quality.rs)
- 全局状态与回调注册：[src/global.rs](./src/global.rs)
- 线程消息投递：[src/messages.rs](./src/messages.rs)

## 网络质量指标说明

- 延迟（Latency/RTT）：ICMP Echo 往返时间
- 丢包率（Packet Loss）：探测包未返回比例
- 稳定性（Retransmission）：TCP 重传率
- 其他指标：抖动、发送段/重传段数量

## 配置说明

网络质量探测的默认参数在全局配置中定义：

- DEFAULT_PING_TARGET：探测目标（支持 IPv4 或域名）
- DEFAULT_PING_COUNT：每次探测的回包次数
- DEFAULT_PING_TIMEOUT_MS：单次探测超时
- DEFAULT_PROBE_INTERVAL_SECS：探测间隔

## 使用方式（示例）

项目作为 N-API 插件使用，需在 Node 侧初始化并注册回调，然后启动后台监控线程。

> 具体 Node.js 调用示例请参考项目内现有测试或业务调用代码。

## 构建与检查

```bash
cargo fmt
cargo clippy
```
