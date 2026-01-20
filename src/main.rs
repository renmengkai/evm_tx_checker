use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use csv::Reader;
use dotenv::dotenv;
use ethers::signers::Signer;
use futures::future::join_all;
use reqwest::Client;
use rust_xlsxwriter::Workbook;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, BufRead};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::time::{timeout, Duration};

const ANKR_RPC_BASE: &str = "https://rpc.ankr.com/multichain";
const WALLET_FILE: &str = "data/wallets.csv";
const DEFAULT_CONCURRENCY: usize = 10;
const DEFAULT_CHAINS: &str = "eth,bsc,polygon,arbitrum,optimism,avalanche";
const REQUEST_TIMEOUT_SECS: u64 = 15;

fn load_target_chains() -> Vec<String> {
    let chains_str = std::env::var("TARGET_CHAINS").unwrap_or_else(|_| DEFAULT_CHAINS.to_string());
    chains_str.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'a str,
    method: &'a str,
    params: RpcParams<'a>,
    id: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RpcParams<'a> {
    blockchain: Vec<&'a str>,
    address: &'a str,
    desc_order: bool,
    page_size: u32,
}

#[derive(Deserialize, Debug)]
struct RpcResponse {
    result: Option<RpcResult>,
}

#[derive(Deserialize, Debug)]
struct RpcResult {
    transactions: Vec<Transaction>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Transaction {
    hash: String,
    timestamp: String,
    blockchain: String,
}


fn identify_input(input: &str) -> (&str, bool) {
    let trimmed = input.trim();

    if trimmed.starts_with("0x") && trimmed.len() == 42 {
        if trimmed[2..].chars().all(|c| c.is_ascii_hexdigit()) {
            return (trimmed, false);
        }
    }

    if !trimmed.starts_with("0x") && trimmed.len() == 40 {
        if trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return (trimmed, false);
        }
    }

    if trimmed.starts_with("0x") && trimmed.len() == 66 {
        if trimmed[2..].chars().all(|c| c.is_ascii_hexdigit()) {
            return (trimmed, true);
        }
    }

    if !trimmed.starts_with("0x") && trimmed.len() == 64 {
        if trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return (trimmed, true);
        }
    }

    (trimmed, false)
}

fn mask_private_key(pk: &str) -> String {
    let total_len = pk.len();

    if total_len <= 10 {
        pk.to_string()
    } else if pk.starts_with("0x") {
        format!("{}...{}", &pk[..6], &pk[total_len-4..])
    } else {
        format!("{}...{}", &pk[..6], &pk[total_len-4..])
    }
}

fn private_key_to_address(private_key: &str) -> Option<String> {
    let pk = if private_key.starts_with("0x") {
        private_key
    } else {
        &format!("0x{}", private_key)
    };

    match ethers::signers::LocalWallet::from_str(pk) {
        Ok(wallet) => {
            let addr = wallet.address();
            let addr_str = format!("{:?}", addr);

            Some(addr_str)
        }
        Err(_) => None,
    }
}

fn load_wallet_addresses() -> Result<Vec<String>> {
    let mut addresses = Vec::new();

    if let Ok(file) = File::open(WALLET_FILE) {
        let mut rdr = Reader::from_reader(file);
        for result in rdr.records() {
            let record = result?;
            if let Some(field) = record.get(0) {
                let (normalized, is_private_key) = identify_input(field);

                if is_private_key {
                    if let Some(address) = private_key_to_address(normalized) {
                        println!("🔑 私钥 → 地址: {} -> {}", mask_private_key(normalized), address);
                        addresses.push(address);
                    } else {
                        println!("⚠️  私钥解析失败: {}", mask_private_key(field));
                    }
                } else {
                    let addr = if !normalized.starts_with("0x") {
                        format!("0x{}", normalized)
                    } else {
                        normalized.to_string()
                    };
                    addresses.push(addr);
                }
            }
        }
        println!("✓ 从 {} 读取到 {} 个地址\n", WALLET_FILE, addresses.len());
        return Ok(addresses);
    }

    if let Ok(file) = File::open("data/wallets.txt") {
        for line in io::BufReader::new(file).lines() {
            if let Ok(line) = line {
                let (normalized, is_private_key) = identify_input(&line);

                if is_private_key {
                    if let Some(address) = private_key_to_address(normalized) {
                        println!("🔑 私钥 → 地址: {} -> {}", mask_private_key(normalized), address);
                        addresses.push(address);
                    } else {
                        println!("⚠️  私钥解析失败: {}", mask_private_key(&line));
                    }
                } else {
                    let addr = if !normalized.starts_with("0x") {
                        format!("0x{}", normalized)
                    } else {
                        normalized.to_string()
                    };
                    addresses.push(addr);
                }
            }
        }
        println!("✓ 从 data/wallets.txt 读取到 {} 个地址\n", addresses.len());
        return Ok(addresses);
    }

    Err(anyhow::anyhow!("未找到钱包文件 (data/wallets.csv 或 data/wallets.txt)"))
}

fn format_timestamp(hex_timestamp: &str) -> String {
    let timestamp_str = if hex_timestamp.starts_with("0x") {
        &hex_timestamp[2..]
    } else {
        hex_timestamp
    };

    match u64::from_str_radix(timestamp_str, 16) {
        Ok(ts) => {
            match DateTime::<Utc>::from_timestamp(ts as i64, 0) {
                Some(dt) => {
                    let local_dt: DateTime<Local> = DateTime::from(dt);
                    local_dt.format("%Y-%m-%d %H:%M").to_string()
                }
                None => "时间格式错误".to_string(),
            }
        }
        Err(_) => "时间解析失败".to_string(),
    }
}

struct QueryResult {
    address: String,
    tx_hash: String,
    tx_time: String,
    tx_chain: String,
}

async fn get_last_txs_batch(client: &Client, addresses: &[String], chains: Vec<String>, api_key: &str, semaphore: Arc<Semaphore>) -> Vec<QueryResult> {
    let base_url = if api_key.is_empty() {
        ANKR_RPC_BASE.to_string()
    } else {
        format!("{}/{}", ANKR_RPC_BASE, api_key)
    };

    let chains_arc = Arc::new(chains);
    let blockchain_vec_arc: Arc<Vec<String>> = Arc::new((*chains_arc).iter().cloned().collect());
    let mut tasks = Vec::new();

    for address in addresses {
        let client_clone = client.clone();
        let url = base_url.clone();
        let addr = address.clone();
        let semaphore = semaphore.clone();
        let chains_arc = chains_arc.clone();
        let blockchain_vec_arc = blockchain_vec_arc.clone();

        tasks.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            let blockchain_vec: Vec<&str> = blockchain_vec_arc.iter().map(|s| s.as_str()).collect();

            let payload = RpcRequest {
                jsonrpc: "2.0",
                method: "ankr_getTransactionsByAddress",
                params: RpcParams {
                    blockchain: blockchain_vec,
                    address: &addr,
                    desc_order: true,
                    page_size: 100,
                },
                id: 1,
            };

            let mut results = Vec::new();
            let chains_clone = (*chains_arc).clone();

            match timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS), client_clone.post(&url).json(&payload).send()).await {
                Ok(Ok(r)) => {
                    let text = r.text().await.unwrap_or_default();

                    match serde_json::from_str::<RpcResponse>(&text) {
                        Ok(json_body) => {
                            if let Some(res) = json_body.result {
                                let txs = res.transactions;
                                if !txs.is_empty() {
                                    let mut by_chain: std::collections::HashMap<String, &Transaction> = std::collections::HashMap::new();
                                    for tx in &txs {
                                        if !tx.hash.is_empty() && !by_chain.contains_key(&tx.blockchain) {
                                            by_chain.insert(tx.blockchain.clone(), tx);
                                        }
                                    }
                                    for chain in &chains_clone {
                                        if let Some(tx) = by_chain.get(chain) {
                                            let tx_hash = tx.hash.clone();
                                            let tx_time = format_timestamp(&tx.timestamp);
                                            println!("✓ {} on {}: {} @ {}", addr, chain, &tx_hash[..12], tx_time);
                                            results.push(QueryResult {
                                                address: addr.clone(),
                                                tx_hash,
                                                tx_time,
                                                tx_chain: chain.to_string(),
                                            });
                                        } else {
                                            println!("○ {} on {}: 无交易", addr, chain);
                                            results.push(QueryResult {
                                                address: addr.clone(),
                                                tx_hash: "无交易".to_string(),
                                                tx_time: "N/A".to_string(),
                                                tx_chain: chain.to_string(),
                                            });
                                        }
                                    }
                                } else {
                                    for chain in &chains_clone {
                                        println!("○ {} on {}: 无交易记录", addr, chain);
                                        results.push(QueryResult {
                                            address: addr.clone(),
                                            tx_hash: "无交易".to_string(),
                                            tx_time: "N/A".to_string(),
                                            tx_chain: chain.to_string(),
                                        });
                                    }
                                }
                            } else {
                                for chain in &chains_clone {
                                    println!("○ {} on {}: result 为空", addr, chain);
                                    results.push(QueryResult {
                                        address: addr.clone(),
                                        tx_hash: "无数据".to_string(),
                                        tx_time: "N/A".to_string(),
                                        tx_chain: chain.to_string(),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            println!("✗ JSON 解析失败 (地址: {}): {}", addr, e);
                            for chain in &chains_clone {
                                results.push(QueryResult {
                                    address: addr.clone(),
                                    tx_hash: "解析失败".to_string(),
                                    tx_time: "N/A".to_string(),
                                    tx_chain: chain.to_string(),
                                });
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    println!("✗ 网络错误 (地址: {}): {}", addr, e);
                    for chain in &chains_clone {
                        results.push(QueryResult {
                            address: addr.clone(),
                            tx_hash: "网络错误".to_string(),
                            tx_time: "N/A".to_string(),
                            tx_chain: chain.to_string(),
                        });
                    }
                }
                Err(_) => {
                    println!("✗ 请求超时 (地址: {}): 超过 {} 秒", addr, REQUEST_TIMEOUT_SECS);
                    for chain in &chains_clone {
                        results.push(QueryResult {
                            address: addr.clone(),
                            tx_hash: "超时".to_string(),
                            tx_time: "N/A".to_string(),
                            tx_chain: chain.to_string(),
                        });
                    }
                }
            }
            results
        }));
    }

    let all_results = join_all(tasks).await;

    let mut query_results = Vec::new();
    for res in all_results {
        if let Ok(data_vec) = res {
            query_results.extend(data_vec);
        }
    }

    query_results
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new();

    dotenv().ok();
    let api_key = std::env::var("ANKR_API_KEY").unwrap_or_else(|_| String::new());
    let concurrency: usize = std::env::var("CONCURRENCY")
        .unwrap_or_else(|_| DEFAULT_CONCURRENCY.to_string())
        .parse()
        .unwrap_or(DEFAULT_CONCURRENCY);

    if api_key.is_empty() {
        println!("⚠️  警告: 未设置 ANKR_API_KEY");
        println!("请在 .env 文件中设置: ANKR_API_KEY=your_api_key");
        println!("或设置环境变量: set ANKR_API_KEY=your_api_key");
        println!("API 密钥格式: https://rpc.ankr.com/multichain/{{your_api_key}}\n");
    } else {
        println!("✓ 已加载 ANKR_API_KEY（{}...）\n", &api_key[..api_key.len().min(8)]);
    }

    println!("✓ 并发数: {}\n", concurrency);

    let target_chains = load_target_chains();
    println!("✓ 目标链: {}\n", target_chains.join(", "));

    let wallet_addresses = load_wallet_addresses()?;
    let addresses_str: Vec<String> = wallet_addresses;
    let semaphore = Arc::new(Semaphore::new(concurrency));

    println!("开始批量查询... (链数量: {}, 地址数量: {})\n", target_chains.len(), addresses_str.len());

    let results = get_last_txs_batch(&client, &addresses_str, target_chains.clone(), &api_key, semaphore).await;

    println!();

    let mut grouped: std::collections::HashMap<String, Vec<&QueryResult>> = std::collections::HashMap::new();
    for row in &results {
        grouped.entry(row.tx_chain.clone()).or_insert_with(Vec::new).push(row);
    }

    let mut workbook = Workbook::new();

    for chain in &target_chains {
        if let Some(rows) = grouped.get(chain) {
            let worksheet = workbook.add_worksheet().set_name(chain)?;

            worksheet.write_string(0, 0, "钱包地址")?;
            worksheet.write_string(0, 1, "最后交易时间 (Local)")?;
            worksheet.write_string(0, 2, "交易 Hash")?;

            worksheet.set_column_width(0, 45)?;
            worksheet.set_column_width(1, 25)?;
            worksheet.set_column_width(2, 70)?;

            for (i, row) in rows.iter().enumerate() {
                let row_idx = (i + 1) as u32;

                worksheet.write_string(row_idx, 0, &row.address)?;
                worksheet.write_string(row_idx, 1, &row.tx_time)?;
                worksheet.write_string(row_idx, 2, &row.tx_hash)?;
            }
        }
    }

    let filename = "wallet_last_tx.xlsx";
    workbook.save(filename)?;

    println!("查询完成！结果已保存至 {}", filename);
    Ok(())
}
