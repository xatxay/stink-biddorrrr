use anyhow::{anyhow, Result};
use chrono::Utc;
use dotenv::dotenv;
use futures::StreamExt;
use hex;
use hmac::{Hmac, Mac};
use reqwest::{Client, Error};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{collections::HashMap, env};

type HmacSha256 = Hmac<Sha256>;

#[derive(Serialize, Deserialize, Debug)]
struct ApiResponse<T> {
    retCode: i32,
    retMsg: String,
    result: T,
    retExtInfo: HashMap<String, serde_json::Value>,
    time: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct KlineData {
    symbol: String,
    category: String,
    list: Vec<Kline>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Kline {
    start_time: String,
    open_price: String,
    high_price: String,
    low_price: String,
    close_price: String,
    volume: String,
    turnover: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct BatchOrderRequest {
    category: String,
    request: Vec<OrderRequest>,
}

#[derive(Serialize, Deserialize, Debug)]
struct OrderRequest {
    symbol: String,
    side: String,
    orderType: String,
    qty: String,
    price: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct BatchOrderResult {
    list: Vec<BatchOrderResponse>,
}

#[derive(Serialize, Deserialize, Debug)]
struct BatchOrderResponse {
    category: String,
    symbol: String,
    orderId: String,
    OrderLinkId: String,
    createAt: String,
}

pub async fn get_kline(symbol: &str) -> Result<(String, String), anyhow::Error> {
    let base_url = env::var("KLINE_URL").expect("KLINE_URL env var is missing");
    let url = format!("{}&symbol={}", base_url, symbol);

    let response = reqwest::get(&url).await?;

    if response.status().is_success() {
        let api_response: ApiResponse<KlineData> = response.json().await?;

        if let Some(first_kline) = api_response.result.list.first() {
            Ok((symbol.to_string(), first_kline.open_price.clone()))
        } else {
            Err(anyhow!("No Kline data found"))
        }
    } else {
        Err(anyhow!("Failed to fetch Kline data"))
    }
}

fn generate_post_signature(
    timestamp: &str,
    api_key: &str,
    recv_window: &str,
    params: &serde_json::Map<String, Value>,
    api_secret: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut mac =
        HmacSha256::new_from_slice(api_secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(timestamp.as_bytes());
    mac.update(api_key.as_bytes());
    mac.update(recv_window.as_bytes());
    mac.update(serde_json::to_string(&params)?.as_bytes());

    let result = mac.finalize();
    let code_bytes = result.into_bytes();
    Ok(hex::encode(code_bytes))
}

async fn place_batch_order(
    api_key: &str,
    api_secret: &str,
    timestamp: &String,
    recv_window: &str,
    batch_order_url: &str,
    symbol: &str,
    price: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let price_num: f64 = price.parse().expect("failed converting price to number");
    let twenty_percent = price_num - (price_num * 0.2);
    let twenty_five_percent = price_num - (price_num * 0.25);
    let thirty_percent = price_num - (price_num * 0.3);
    let twenty_percent_size = format!("{:.2}", 1000.0 / twenty_percent);
    let twenty_five_percent_size = format!("{:.2}", 1500.0 / twenty_five_percent);
    let thirty_percent_size = format!("{:.2}", 2000.0 / thirty_percent);
    println!(
        "ticker: {},open price: {}, price: {}, {}, {}, size: {}, {}, {}",
        symbol,
        price,
        twenty_percent,
        twenty_five_percent,
        thirty_percent,
        twenty_percent_size,
        twenty_five_percent_size,
        thirty_percent_size
    );
    let client = Client::new();
    let parameters: [OrderRequest; 3] = [
        OrderRequest {
            symbol: symbol.to_string(),
            side: "Buy".to_string(),
            orderType: "Limit".to_string(),
            qty: twenty_percent_size,
            price: format!("{:.2}", twenty_percent),
        },
        OrderRequest {
            symbol: symbol.to_string(),
            side: "Buy".to_string(),
            orderType: "Limit".to_string(),
            qty: twenty_five_percent_size,
            price: format!("{:.2}", twenty_five_percent),
        },
        OrderRequest {
            symbol: symbol.to_string(),
            side: "Buy".to_string(),
            orderType: "Limit".to_string(),
            qty: thirty_percent_size,
            price: format!("{:.2}", thirty_percent),
        },
    ];
    let mut params = serde_json::Map::new();
    params.insert("category".to_string(), json!("linear"));
    params.insert("request".to_string(), json!(&parameters));

    let signature = generate_post_signature(&timestamp, api_key, recv_window, &params, api_secret)?;

    let response = client
        .post(batch_order_url)
        .json(&params)
        .header("X-BAPI-API-KEY", api_key)
        .header("X-BAPI-SIGN", &signature)
        .header("X-BAPI-SIGN-TYPE", "2")
        .header("X-BAPI-TIMESTAMP", timestamp)
        .header("X-BAPI-RECV-WINDOW", recv_window)
        .header("Content-Type", "application/json")
        .send()
        .await?;

    println!("Response: {:?}", response.json().await?);
    Ok(())
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    let api_key = env::var("API_KEY").expect("api key is missing");
    let api_secret = env::var("API_SECRET").expect("api secret is missing");
    let timestamp = Utc::now().timestamp_millis().to_string();
    let recv_window = "10000";
    let batch_order_url = env::var("BATCH_ORDER_URL").expect("batch order url is missing");

    println!("Api key {}, api secret: {}", &api_key, &api_secret);
    // 100, 0.01, 1
    let symbols = vec!["BEAMUSDT", "TAOUSDT", "SEIUSDT"];

    let futures = symbols.into_iter().map(|symbol| get_kline(&symbol));
    let results = futures::future::join_all(futures).await;

    for result in results {
        if let Ok((symbol, open_price)) = result {
            println!(
                "Placing batch order for {}, open price: {}",
                symbol, open_price
            );
            place_batch_order(
                &api_key,
                &api_secret,
                &timestamp,
                &recv_window,
                &batch_order_url,
                &symbol,
                &open_price,
            )
            .await
            .expect("Error placing order");
        }
    }
}
