use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use csv::Reader;
use dotenv::dotenv;
use futures::future::join_all;
use k256::ecdsa::SigningKey;
use reqwest::Client;
use rust_xlsxwriter::Workbook;
use serde::{Deserialize, Serialize};
use sha3::Digest;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead};
use std::sync::Arc;
use tokio::sync::Semaphore;

const ANKR_RPC_BASE: &str = "https://rpc.ankr.com/multichain";
const TARGET_CHAINS: &[&str; 7] = &["eth", "bsc", "polygon", "arbitrum", "optimism", "avalanche", "zksync"];
const WALLET_FILE: &str = "data/wallets.csv";
const DEFAULT_CONCURRENCY: usize = 10;

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
    blockchain: &'a str,
    address: &'a str,
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
}

#[derive(Deserialize, Debug)]
struct BlockResponse {
    result: Option<Block>,
}

#[derive(Deserialize, Debug)]
struct Block {
    timestamp: String,
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

fn private_key_to_address(private_key: &str) -> Option<String> {
    let pk = if private_key.starts_with("0x") {
        &private_key[2..]
    } else {
        private_key
    };
    
    let secret_bytes = hex::decode(pk).ok()?;
    let signing_key = SigningKey::from_slice(&secret_bytes).ok()?;
    let verifying_key = signing_key.verifying_key();
    let hash = sha3::Keccak256::digest(verifying_key.to_encoded_point(false).as_bytes());
    let address_bytes = &hash.as_slice()[12..];
    Some(format!("0x{}", hex::encode(address_bytes)))
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
                        println!("🔑 私钥 → 地址: {}... -> {}", &normalized[..8], &address[2..10]);
                        addresses.push(address);
                    } else {
                        println!("⚠️  私钥解析失败: {}", field);
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
                        println!("🔑 私钥 → 地址: {}... -> {}", &normalized[..8], &address[2..10]);
                        addresses.push(address);
                    } else {
                        println!("⚠️  私钥解析失败: {}", line);
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
    chain: String,
    address: String,
    tx_hash: String,
    tx_time: String,
}

async fn get_last_txs(client: &Client, chain: &str, addresses: &[String], api_key: &str, semaphore: Arc<Semaphore>) -> Vec<QueryResult> {
    let base_url = if api_key.is_empty() {
        ANKR_RPC_BASE.to_string()
    } else {
        format!("{}/{}", ANKR_RPC_BASE, api_key)
    };

    let mut tasks = Vec::new();
    
    for address in addresses {
        let client_clone = client.clone();
        let url = base_url.clone();
        let chain_name = chain.to_string();
        let addr = address.clone();
        let semaphore = semaphore.clone();
        
        tasks.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            
            let payload = RpcRequest {
                jsonrpc: "2.0",
                method: "ankr_getTransactionsByAddress",
                params: RpcParams {
                    blockchain: &chain_name,
                    address: &addr,
                },
                id: 1,
            };

            match client_clone.post(&url).json(&payload).send().await {
                Ok(r) => match r.json::<RpcResponse>().await {
                    Ok(json_body) => {
                        if let Some(res) = json_body.result {
                            if let Some(tx) = res.transactions.first() {
                                return Some((addr.clone(), tx.hash.clone(), format_timestamp(&tx.timestamp)));
                            }
                        }
                    }
                    Err(e) => {
                        println!("JSON 解析失败 ({} on {}): {}", addr, chain_name, e);
                    }
                },
                Err(e) => {
                    println!("网络错误 ({} on {}): {}", addr, chain_name, e);
                }
            }
            None
        }));
    }

    let results = join_all(tasks).await;

    let mut query_results = Vec::new();
    for (i, res) in results.iter().enumerate() {
        let addr = &addresses[i];
        match res {
            Ok(Some((_, tx_hash, tx_time))) => {
                query_results.push(QueryResult {
                    chain: chain.to_string(),
                    address: addr.clone(),
                    tx_hash: tx_hash.clone(),
                    tx_time: tx_time.clone(),
                });
            }
            _ => {
                query_results.push(QueryResult {
                    chain: chain.to_string(),
                    address: addr.clone(),
                    tx_hash: "未找到交易".to_string(),
                    tx_time: "N/A".to_string(),
                });
            }
        }
    }

    query_results
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new();
    let mut tasks = Vec::new();

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

    let wallet_addresses = load_wallet_addresses()?;
    let addresses_str: Vec<String> = wallet_addresses;
    let semaphore = Arc::new(Semaphore::new(concurrency));

    println!("开始批量查询... (链数量: {}, 地址数量: {})\n", TARGET_CHAINS.len(), addresses_str.len());

    for &chain in TARGET_CHAINS {
        let client_ref = client.clone();
        let api_key_ref = api_key.clone();
        let addresses_clone = addresses_str.clone();
        let semaphore = semaphore.clone();
        
        tasks.push(tokio::spawn(async move {
            get_last_txs(&client_ref, chain, &addresses_clone, &api_key_ref, semaphore).await
        }));
    }

    let results = join_all(tasks).await;

    let mut grouped_results: HashMap<String, Vec<QueryResult>> = HashMap::new();
    for res in results {
        if let Ok(data_vec) = res {
            for data in data_vec {
                grouped_results
                    .entry(data.chain.clone())
                    .or_insert_with(Vec::new)
                    .push(data);
            }
        }
    }

    let mut workbook = Workbook::new();

    for chain_name in TARGET_CHAINS {
        if let Some(rows) = grouped_results.get(*chain_name) {
            let worksheet = workbook.add_worksheet().set_name(*chain_name)?;

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
