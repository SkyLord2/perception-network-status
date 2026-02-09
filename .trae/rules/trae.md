# TRAE 项目开发规范：Napi-rs 混合架构指南

## 1. 项目概览与核心原则

### 1.1 项目简介

本项目采用 Rust 与 Node.js 混合架构，核心业务逻辑通过 napi-rs 构建为高性能的原生插件，并通过 TypeScript 暴露给上层应用。

### 1.2 核心原则 (Core Principles)

- **Rust 优先 (Rust First)**：计算密集型任务、内存敏感操作及核心算法必须在 Rust 层实现。
- **零拷贝 (Zero-Copy)**：在 JS 和 Rust 之间传递大数据（如 Buffer、大字符串）时，优先通过引用或 Buffer 切片操作，避免不必要的内存克隆。
- **类型安全**：严禁使用 any。Rust 的类型必须通过 napi-rs 的宏精确映射到 TypeScript 定义 (.d.ts)。
- **非阻塞**：在 Rust 中执行耗时操作时，必须使用异步模式，绝对禁止阻塞 Node.js 的主事件循环 (Event Loop)。
- **无 Panic (Panic Free)**: Rust 端严禁直接 panic!，所有错误必须转换为 napi::Result 并抛出给 JS 层捕获。

## 2. 技术栈与环境规范

### 2.1 开发环境

- **Rust 版本**：最新稳定版
- **Node.js Runtime**：LTS 版本 (v18+).
- **Core Library**：napi v3+, napi-derive v3+
- **Build System**：Cargo (Rust) + CLI (napi-cli).
- **Linting**：Rust: clippy (必须通过 cargo clippy -- -D warnings).

## 3. 代码生成规范 (Code Generation Standards)

### 3.1 代码风格

- 遵循标准 Rust 格式化 (cargo fmt)。
- 变量命名使用 snake_case，结构体和枚举使用 PascalCase。
- 注释: 公开导出的函数 (#[napi]) 必须包含文档注释 (///)，这些注释会被自动生成到 .d.ts 文件中。

### 3.2 Rust (`src/lib.rs` 及其他 `.rs` 文件)

- **宏的使用**：
  - 所有暴露给 JS 的函数、结构体和枚举必须使用 `#[napi]` 宏标记。
  - 构造函数应标记为 `#[napi(constructor)]`。
- **错误处理**：
  - 绝对禁止在 FFI 边界让 Rust 代码 Panic。这会导致 Node.js 进程直接崩溃。
  - Rust 函数应返回 `napi::Result<T>`。
  - 使用 `napi::Error::from_reason("error message")` 抛出 JS 异常。
  - 利用 `?` 操作符自动传递错误。

### 3.3 内存管理 (Memory Management)

- **引用计数**：当需要在 Rust 中长期持有 JS 对象（如回调函数）时，必须使用 `napi::threadsafe_function::ThreadsafeFunction` 或 `napi::Ref`，防止对象被 GC 回收。
- **Buffer 操作**：避免不必要的内存拷贝。如果可能，直接在 Rust 中操作传入的 `Buffer` 引用（`&[u8]`）。

### 3.4 异步编程与并发 (Async & Concurrency)

- 在 napi-rs 中处理异步任务有两种主要模式，需根据场景严格选择：
  - **简单异步 (I/O Bound)**：直接使用 `async fn` 并标记 `#[napi]`，返回值会被自动转换为 Promise。
  - **CPU 密集型任务**：对于会阻塞 Node.js 事件循环的繁重计算（如图像处理、加密、大文件解析），必须使用 AsyncTask 将工作卸载到 libuv 线程池。

## 5. 数据交互与类型映射 (Interop)

### 5.1 复杂对象传递

避免在 Rust 和 JS 之间频繁传递极其复杂的嵌套对象。如果必须传递，优先使用 serde 序列化，或者设计扁平化的 API。

- Config 对象: 使用 #[napi(object)] 宏定义纯数据结构（POD），这比 Class 性能更高，因为它直接映射到 JS Object。

### 5.2 Buffer 操作 (关键)

处理二进制数据时，必须小心内存拷贝。

- **接收 Buffer (JS -> Rust)**: 使用 Buffer 或 &[u8]。

- **发送 Buffer (Rust -> JS)**: 使用 Buffer::from(vec).

- **外部引用**: 如果要直接操作 JS 内存而不复制，使用 napi::bindgen_prelude::Buffer 并注意生命周期管理。

### 5.3 外部引用 (External)

如果需要在 JS 对象中持有 Rust 的复杂数据结构（非 Copy 类型），使用 External<T>。这允许 JS "拥有" 一个指向 Rust 堆内存的指针，但不会直接序列化它。

## 6. 测试策略 (Testing Strategy)

本项目采用双层测试策略：

- **Rust 单元测试 (cargo test)**:
  - 针对纯算法逻辑，不涉及 JS 环境的代码。

  - 放置在 src/ 下，使用 standard #[test]。

- **JavaScript 集成测试 (ava / jest)**:
  - 这是主要的测试方式。

  - 必须编译并加载编译后的 .node 文件进行测试。

  - 测试用例应覆盖所有导出的 API，包括边界条件和错误捕获（验证 Promise.reject）。
