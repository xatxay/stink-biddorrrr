use chrono::Utc;
use dotenv::dotenv;
use hex;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{collections::HashMap, env, time::Duration};
use tokio::time::sleep;

type HmacSha256 = Hmac<Sha256>;

#[derive(Serialize, Deserialize, Debug)]
struct ApiResponse<T> {
    #[serde(rename = "retCode")]
    ret_code: i32,
    #[serde(rename = "retMsg")]
    ret_msg: String,
    result: T,
    #[serde(rename = "retExtInfo")]
    ret_ext_info: HashMap<String, serde_json::Value>,
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
    #[serde(rename = "orderType")]
    order_type: String,
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
    #[serde(rename = "orderId")]
    order_id: String,
    #[serde(rename = "orderLinkId")]
    order_link_id: String,
    #[serde(rename = "createAt")]
    create_at: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Quantity {
    twenty_percent_size: f64,
    twenty_five_percent_size: f64,
    thirty_percent_size: f64,
}

#[derive(Serialize, Deserialize, Debug)]
struct Price {
    twenty_percent_price: f64,
    twenty_five_percent_price: f64,
    thirty_percent_price: f64,
}

#[derive(Serialize, Deserialize, Debug)]
struct FormattedPosition {
    twenty_percent_price: String,
    twenty_five_percent_price: String,
    thirty_percent_price: String,
    twenty_percent_size: String,
    twenty_five_percent_size: String,
    thirty_percent_size: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct CancelOrderData {
    symbol: String,
    #[serde(rename = "orderId")]
    order_id: String,
}

pub async fn get_kline(symbol: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    let base_url = env::var("KLINE_URL").expect("KLINE_URL env var is missing");
    let url = format!("{}&symbol={}", base_url, symbol);

    let response = reqwest::get(&url).await?;

    let api_response: ApiResponse<KlineData> = response.json().await?;

    let first_kline = api_response.result.list.first().unwrap();
    Ok((symbol.to_string(), first_kline.open_price.clone()))
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

fn calculate_position(price: &f64, symbol: &str) -> Option<FormattedPosition> {
    println!("cal price: {}, symbol: {}", price, symbol);
    let price = Price {
        twenty_percent_price: price - (price * 0.2),
        twenty_five_percent_price: price - (price * 0.25),
        thirty_percent_price: price - (price * 0.3),
    };
    let size = Quantity {
        twenty_percent_size: 1000.0 / price.twenty_percent_price,
        twenty_five_percent_size: 1000.0 / price.twenty_five_percent_price,
        thirty_percent_size: 2000.0 / price.thirty_percent_price,
    };

    //ideally i'd hit the intrument info api to get the tickSize and qtyStep
    //when starting the app

    let formatted_position = match symbol {
        "BEAMUSDT" => FormattedPosition {
            twenty_percent_price: format!("{:.6}", price.twenty_percent_price),
            twenty_five_percent_price: format!("{:.6}", price.twenty_five_percent_price),
            thirty_percent_price: format!("{:.6}", price.thirty_percent_price),
            twenty_percent_size: format!("{:.0}", size.twenty_percent_size.round()),
            twenty_five_percent_size: format!("{:.0}", size.twenty_five_percent_size.round()),
            thirty_percent_size: format!("{:.0}", size.thirty_percent_size.round()),
        },
        "SEIUSDT" => FormattedPosition {
            twenty_percent_price: format!("{:.5}", price.twenty_percent_price),
            twenty_five_percent_price: format!("{:.5}", price.twenty_five_percent_price),
            thirty_percent_price: format!("{:.5}", price.thirty_percent_price),
            twenty_percent_size: format!("{}", size.twenty_percent_size.round()),
            twenty_five_percent_size: format!("{}", size.twenty_five_percent_size.round()),
            thirty_percent_size: format!("{}", size.thirty_percent_size.round()),
        },
        "AGIXUSDT" => FormattedPosition {
            twenty_percent_price: format!("{:.5}", price.twenty_percent_price),
            twenty_five_percent_price: format!("{:.5}", price.twenty_five_percent_price),
            thirty_percent_price: format!("{:.5}", price.thirty_percent_price),
            twenty_percent_size: format!("{}", size.twenty_percent_size.round()),
            twenty_five_percent_size: format!("{}", size.twenty_five_percent_size.round()),
            thirty_percent_size: format!("{}", size.thirty_percent_size.round()),
        },
        _ => return None,
    };

    Some(formatted_position)
}

async fn place_batch_order(
    api_key: &str,
    api_secret: &str,
    recv_window: &str,
    batch_order_url: &str,
    symbol: &str,
    price: &str,
) -> Result<Vec<CancelOrderData>, Box<dyn std::error::Error>> {
    let timestamp = Utc::now().timestamp_millis().to_string();
    let price_num: f64 = price.parse().expect("failed converting price to number");
    let position = calculate_position(&price_num, symbol).expect("Failed calculating position");
    println!(
        "ticker: {},open price: {}, price: {}, {}, {}, size: {}, {}, {}",
        symbol,
        price,
        position.twenty_percent_price,
        position.twenty_five_percent_price,
        position.thirty_percent_price,
        position.twenty_percent_size,
        position.twenty_five_percent_size,
        position.thirty_percent_size
    );
    let client = Client::new();
    let parameters: [OrderRequest; 3] = [
        OrderRequest {
            symbol: symbol.to_string(),
            side: "Buy".to_string(),
            order_type: "Limit".to_string(),
            qty: position.twenty_percent_size,
            price: position.twenty_percent_price,
        },
        OrderRequest {
            symbol: symbol.to_string(),
            side: "Buy".to_string(),
            order_type: "Limit".to_string(),
            qty: position.twenty_five_percent_size,
            price: position.twenty_five_percent_price,
        },
        OrderRequest {
            symbol: symbol.to_string(),
            side: "Buy".to_string(),
            order_type: "Limit".to_string(),
            qty: position.thirty_percent_size,
            price: position.thirty_percent_price,
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
        .header("X-BAPI-TIMESTAMP", &timestamp)
        .header("X-BAPI-RECV-WINDOW", recv_window)
        .header("Content-Type", "application/json")
        .send()
        .await?;

    let response_data: ApiResponse<BatchOrderResult> = response.json().await?;
    println!("Response: {:#?}", response_data);

    let cancel_order_data: Vec<CancelOrderData> = response_data
        .result
        .list
        .iter()
        .map(|order_response| CancelOrderData {
            symbol: order_response.symbol.clone(),
            order_id: order_response.order_id.clone(),
        })
        .collect();

    Ok(cancel_order_data)
}

async fn cancel_batch_order(
    api_key: &str,
    api_secret: &str,
    recv_window: &str,
    batch_cancel_order_url: &str,
    cancel_order_data: &Vec<CancelOrderData>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    let timestamp = Utc::now().timestamp_millis().to_string();
    let mut params = serde_json::Map::new();
    params.insert("category".to_string(), json!("linear"));
    params.insert("request".to_string(), json!(cancel_order_data));

    let signature = generate_post_signature(&timestamp, api_key, recv_window, &params, api_secret)?;

    let response = client
        .post(batch_cancel_order_url)
        .json(&params)
        .header("X-BAPI-API-KEY", api_key)
        .header("X-BAPI-SIGN", &signature)
        .header("X-BAPI-SIGN-TYPE", "2")
        .header("X-BAPI-TIMESTAMP", &timestamp)
        .header("X-BAPI-RECV-WINDOW", recv_window)
        .header("Content-Type", "application/json")
        .send()
        .await?;

    println!("cancel response = {}", response.text().await?);
    Ok(())
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    let api_key = env::var("API_KEY").expect("api key is missing");
    let api_secret = env::var("API_SECRET").expect("api secret is missing");
    let recv_window = "10000";
    let batch_order_url = env::var("BATCH_ORDER_URL").expect("batch order url is missing");
    let batch_cancel_order_url =
        env::var("BATCH_CANCEL_ORDER_URL").expect("batch cancel order url is missing");

    loop {
        let symbols = vec!["BEAMUSDT", "SEIUSDT", "AGIXUSDT"];
        let futures = symbols.into_iter().map(|symbol| get_kline(&symbol));
        let results = futures::future::join_all(futures).await;
        let mut cancel_order_data: Vec<CancelOrderData> = Vec::new();

        for result in results {
            if let Ok((symbol, open_price)) = result {
                println!(
                    "Placing batch order for {}, open price: {}",
                    symbol, open_price
                );
                let cancel_data = place_batch_order(
                    &api_key,
                    &api_secret,
                    &recv_window,
                    &batch_order_url,
                    &symbol,
                    &open_price,
                )
                .await
                .expect("Error placing order");

                cancel_order_data.extend(cancel_data);
            }
        }

        println!("waiting 24hrs: {:#?}", &cancel_order_data);
        sleep(Duration::from_secs(86400)).await;

        if !cancel_order_data.is_empty() {
            cancel_batch_order(
                &api_key,
                &api_secret,
                recv_window,
                &batch_cancel_order_url,
                &cancel_order_data,
            )
            .await
            .expect("Failed canceling orders")
        }
        println!("canceled order data: {:#?}", &cancel_order_data);
        sleep(Duration::from_secs(60)).await;
    }
}
