#[path = "../config.rs"]
mod config;
#[path = "../db/mod.rs"]
mod db;

#[tokio::main]
async fn main() {
    let cfg = config::AppConfig::from_env().expect("config");
    let rb = db::init_rbatis(&cfg.database.url).await.expect("db");

    let raw1 = rb.query("select * from customer_review order by id desc limit 1", vec![]).await;
    println!("customer_review raw = {:?}", raw1);

    let raw2 = rb.query("select * from dealer_review order by id desc limit 1", vec![]).await;
    println!("dealer_review raw = {:?}", raw2);
}
