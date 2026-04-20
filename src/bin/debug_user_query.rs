#[path = "../config.rs"]
mod config;
#[path = "../db/mod.rs"]
mod db;
#[path = "../models/mod.rs"]
mod models;

use rbs::value;

#[tokio::main]
async fn main() {
    let cfg = config::AppConfig::from_env().expect("config");
    let rb = db::init_rbatis(&cfg.database.url).await.expect("db");
    let result = models::user::UserEntity::select_by_map(&rb, value! { "mobile": "13800000000" }).await;
    match result {
        Ok(rows) => println!("ok rows={}", rows.len()),
        Err(err) => println!("err: {:?}", err),
    }

    let raw = rb.query("select * from user where mobile = ?", vec![rbs::to_value!("13800000000")]).await;
    match raw {
        Ok(v) => { println!("raw: {:?}", v); let conv: Result<Vec<models::user::UserEntity>, _> = rbs::from_value(v.clone()); println!("from_value: {:?}", conv.as_ref().map(|x| x.len())); },
        Err(e) => println!("raw err: {:?}", e),
    }

    let decoded: Result<Vec<models::user::UserEntity>, _> = rb.query_decode("select * from user where mobile = ?", vec![value!("13800000000")]).await;
    match decoded {
        Ok(rows) => println!("decoded rows={}", rows.len()),
        Err(err) => println!("decoded err: {:?}", err),
    }

    let compat = db::select_by_map_compat::<models::user::UserEntity>(&rb, "user", value! { "mobile": "13800000000" }).await;
    match compat {
        Ok(rows) => println!("compat rows={}", rows.len()),
        Err(err) => println!("compat err: {:?}", err),
    }
}
