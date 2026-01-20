# EVM 区块链钱包交易查询工具

这个工具使用 Ankr 多链 RPC 接口查询以太坊虚拟机(EVM)兼容链上的钱包交易历史。

## 功能

- 批量查询多个钱包地址的交易记录
- 支持多条区块链（ETH、BSC、Polygon、Arbitrum、Optimism、Avalanche、zkSync）
- 支持从私钥自动导出钱包地址
- 输出到 Excel 文件格式
- 支持两种配置文件格式（CSV 和 TXT）
- 时间戳自动转换为本地时间

## 使用方法

### 1. 设置 Ankr API 密钥

首先获取 Ankr API 密钥并设置环境变量：

```bash
# Windows PowerShell
$env:ANKR_API_KEY = "your_api_key_here"

# Windows CMD
set ANKR_API_KEY=your_api_key_here

# Linux/Mac
export ANKR_API_KEY=your_api_key_here
```

或者在项目根目录创建 `.env` 文件：

```
ANKR_API_KEY=your_api_key_here
```

### 2. 准备钱包地址列表

创建配置文件，支持以下两种格式：

**方式一：CSV 格式** (`config/wallets.csv`)
```csv
0x742d35Cc6634C0532925a3b844Bc9e7595f8fEb5
0x1234567890abcdef1234567890abcdef12345678
```

**方式二：TXT 格式** (`config/wallets.txt`)
```
0x742d35Cc6634C0532925a3b844Bc9e7595f8fEb5
0x1234567890abcdef1234567890abcdef12345678
```

**方式三：直接使用私钥**（程序会自动转换为地址）
```txt
0xabcd1234...
```

### 3. 编译

```bash
cargo build --release
```

### 4. 运行

```bash
# Windows
.\target\release\evm_tx_checker.exe

# Linux/Mac
./target/release/evm_tx_checker
```

## 输出

程序会生成 `wallet_last_tx.xlsx` Excel 文件，包含以下列：

| 列名 | 说明 |
|------|------|
| 钱包地址 | 查询的钱包地址 |
| 最后交易时间 (Local) | 最新交易的本地时间戳 |
| 交易 Hash | 最新交易的哈希值 |

每条链对应一个工作表（Sheet），支持 7 条链：eth、bsc、polygon、arbitrum、optimism、avalanche、zksync。

## 技术栈

- **语言**：Rust 2021 Edition
- **异步运行时**：Tokio v1
- **HTTP 客户端**：reqwest v0.11
- **JSON 处理**：serde + serde_json
- **Excel 输出**：rust_xlsxwriter v0.60
- **DateTime**：chrono v0.4
- **密码学**：k256（ECDSA 签名）、sha3（Keccak256 哈希）
- **配置处理**：dotenv、csv

## API 信息

- **RPC 基础 URL**：`https://rpc.ankr.com/multichain/{api_key}`
- **主要方法**：`ankr_getTransactionsByAddress` - 批量查询交易
- **支持的链标识符**：
  - `eth` - Ethereum
  - `bsc` - Binance Smart Chain
  - `polygon` - Polygon
  - `arbitrum` - Arbitrum One
  - `optimism` - Optimism
  - `avalanche` - Avalanche C-Chain
  - `zksync` - zkSync Era

## 已知限制

- 每条链最多返回前 100 条交易（由 API 限制）
- 需要有效的 Ankr API 密钥才能正常查询

## 构建详情

编译后的可执行文件位于 `target/release/` 目录。

## 许可证

内部使用工具
