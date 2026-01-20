# EVM 区块链钱包交易查询工具

这个工具使用 Ankr 多链 RPC 接口查询以太坊虚拟机(EVM)兼容链上的钱包交易历史。

## 功能

- 批量查询多个钱包地址的交易记录
- 支持多条区块链（Arbitrum、Optimism）
- 输出到 Excel 文件格式
- 采用批量查询优化（每条链只需2次API调用）

## 使用方法

### 设置环境变量

首先设置 Ankr API 密钥：

```bash
# Windows PowerShell
$env:ANKR_API_KEY = "your_api_key_here"

# Windows CMD
set ANKR_API_KEY=your_api_key_here

# Linux/Mac
export ANKR_API_KEY=your_api_key_here
```

### 编译

```bash
cargo build --release
```

### 运行

```bash
# Windows
.\target\release\evm_tx_checker.exe

# Linux/Mac
./target/release/evm_tx_checker
```

### 配置查询参数

编辑 `src/main.rs` 中的常量来修改：

- `TARGET_CHAINS` - 要查询的区块链列表
- `WALLET_ADDRESSES` - 要查询的钱包地址列表

## 输出

程序会生成 `wallet_last_tx.xlsx` Excel 文件，包含以下列：

| 列名 | 说明 |
|------|------|
| 钱包地址 | 查询的钱包地址 |
| 最后交易时间 | 最新交易的时间戳 |
| 交易 Hash | 最新交易的哈希值 |

每条链对应一个工作表（Sheet）。

## 技术栈

- **语言**：Rust 2021 Edition
- **异步运行时**：Tokio v1
- **HTTP 客户端**：reqwest v0.11
- **JSON 处理**：serde + serde_json
- **Excel 输出**：rust_xlsxwriter v0.60
- **DateTime**：chrono v0.4

## API 信息

- **RPC 基础 URL**：`https://rpc.ankr.com/multichain/{api_key}`
- **主要方法**：`ankr_getTransactionsByAddress` - 批量查询交易
- **支持的链标识符**：
  - `arbitrum` - Arbitrum One
  - `optimism` - Optimism

## 已知限制

- 交易时间戳字段当前显示为 "N/A"（Ankr RPC 不支持 `eth_getBlockByNumber`）
- 每条链最多返回前 100 条交易（由 `page_size` 参数限制）

## 构建详情

编译后的可执行文件位于 `target/release/` 目录。

## 许可证

内部使用工具
