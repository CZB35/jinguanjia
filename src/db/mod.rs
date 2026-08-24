use anyhow::{Context, Result, anyhow};
use chrono::DateTime;
use rbatis::{RBatis, executor::Executor};
use rbdc_mysql::driver::MysqlDriver;
use rbs::{Value, from_value, value::map::ValueMap};
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;

pub async fn init_rbatis(database_url: &str) -> Result<RBatis> {
    let rb = RBatis::new();
    rb.link(MysqlDriver {}, database_url)
        .await
        .with_context(|| format!("failed to connect mysql: {database_url}"))?;
    Ok(rb)
}

pub async fn select_by_map_compat<T>(rb: &RBatis, table: &str, condition: Value) -> Result<Vec<T>>
where
    T: DeserializeOwned,
{
    let (sql, args) = build_select_sql(table, condition)?;
    let raw = rb
        .query(sql.as_str(), args)
        .await
        .with_context(|| format!("failed to query table: {table}"))?;
    decode_table_rows(raw)
}

pub async fn select_one_by_map_compat<T>(
    rb: &RBatis,
    table: &str,
    condition: Value,
) -> Result<Option<T>>
where
    T: DeserializeOwned,
{
    let mut rows: Vec<T> = select_by_map_compat(rb, table, condition).await?;
    Ok(rows.drain(..).next())
}

pub async fn select_by_map_for_update_compat<T, E>(
    executor: &E,
    table: &str,
    condition: Value,
) -> Result<Vec<T>>
where
    T: DeserializeOwned,
    E: Executor + ?Sized,
{
    let (mut sql, args) = build_select_sql(table, condition)?;
    sql.push_str(" for update");
    let raw = executor
        .query(sql.as_str(), args)
        .await
        .with_context(|| format!("failed to lock table rows: {table}"))?;
    decode_table_rows(raw)
}

fn build_select_sql(table: &str, condition: Value) -> Result<(String, Vec<Value>)> {
    let mut sql = format!("select * from {}", table);
    let mut clauses = Vec::new();
    let mut args = Vec::new();

    if let Value::Map(map) = condition {
        for (key, value) in map {
            let Some(column) = key.as_str() else {
                continue;
            };
            if column == "column" || value == Value::Null {
                continue;
            }
            match value {
                Value::Array(arr) if !arr.is_empty() => {
                    let placeholders = std::iter::repeat_n("?", arr.len())
                        .collect::<Vec<_>>()
                        .join(", ");
                    clauses.push(format!("`{}` in ({})", column, placeholders));
                    args.extend(arr);
                }
                Value::Array(_) => {}
                other => {
                    clauses.push(format!("`{}` = ?", column));
                    args.push(other);
                }
            }
        }
    }

    if !clauses.is_empty() {
        sql.push_str(" where ");
        sql.push_str(&clauses.join(" and "));
    }

    Ok((sql, args))
}

fn decode_table_rows<T>(raw: Value) -> Result<Vec<T>>
where
    T: DeserializeOwned,
{
    match raw {
        Value::Null => Ok(Vec::new()),
        Value::Map(_) => from_value::<T>(raw)
            .map(|item| vec![item])
            .map_err(|e| anyhow!(e.to_string())),
        Value::Array(rows) => {
            if rows.is_empty() {
                return Ok(Vec::new());
            }

            if matches!(rows.first(), Some(Value::Map(_))) {
                return from_value::<Vec<T>>(Value::Array(rows))
                    .map_err(|e| anyhow!(e.to_string()));
            }

            let headers = rows
                .first()
                .and_then(|item| item.as_array().cloned())
                .ok_or_else(|| anyhow!("invalid mysql table result header"))?;

            let mut mapped_rows = Vec::new();
            for row in rows.into_iter().skip(1) {
                let cells = row
                    .as_array()
                    .cloned()
                    .ok_or_else(|| anyhow!("invalid mysql table result row"))?;
                let mut map = ValueMap::with_capacity(headers.len());
                for (index, header) in headers.iter().enumerate() {
                    let normalized =
                        normalize_value(cells.get(index).cloned().unwrap_or(Value::Null));
                    let final_value = if is_json_column(header) {
                        Value::String(to_json_value(normalized).to_string())
                    } else {
                        normalized
                    };
                    map.insert(header.clone(), final_value);
                }
                mapped_rows.push(Value::Map(map));
            }

            from_value::<Vec<T>>(Value::Array(mapped_rows)).map_err(|e| anyhow!(e.to_string()))
        }
        other => Err(anyhow!("unsupported mysql result shape: {other:?}")),
    }
}

fn is_json_column(header: &Value) -> bool {
    match header {
        Value::String(name) => {
            name.ends_with("_json")
                || matches!(
                    name.as_str(),
                    "config_value"
                        | "before_value"
                        | "after_value"
                        | "request_payload"
                        | "response_payload"
                )
        }
        _ => false,
    }
}

fn normalize_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(normalize_value).collect()),
        Value::Map(map) => {
            let mut normalized = ValueMap::with_capacity(map.len());
            for (key, val) in map {
                normalized.insert(key, normalize_value(val));
            }
            Value::Map(normalized)
        }
        Value::Ext(tag, ext) if tag == "DateTime" => match *ext {
            Value::String(text) => {
                if let Ok(dt) = DateTime::parse_from_rfc3339(&text) {
                    Value::String(dt.naive_local().format("%Y-%m-%dT%H:%M:%S").to_string())
                } else {
                    Value::String(text)
                }
            }
            other => Value::Ext(tag, Box::new(other)),
        },
        Value::Ext(tag, ext)
            if tag.eq_ignore_ascii_case("json") || tag.eq_ignore_ascii_case("Json") =>
        {
            match *ext {
                Value::String(text) => Value::String(text),
                other => {
                    let json = to_json_value(normalize_value(other));
                    Value::String(json.to_string())
                }
            }
        }
        other => other,
    }
}

fn to_json_value(value: Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Bool(v) => JsonValue::Bool(v),
        Value::I32(v) => JsonValue::from(v),
        Value::I64(v) => JsonValue::from(v),
        Value::U32(v) => JsonValue::from(v),
        Value::U64(v) => JsonValue::from(v),
        Value::F32(v) => JsonValue::from(v as f64),
        Value::F64(v) => JsonValue::from(v),
        Value::String(v) => JsonValue::String(v),
        Value::Binary(v) => JsonValue::String(String::from_utf8_lossy(&v).to_string()),
        Value::Array(values) => JsonValue::Array(values.into_iter().map(to_json_value).collect()),
        Value::Map(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                obj.insert(
                    match k {
                        Value::String(s) => s,
                        other => format!("{other:?}"),
                    },
                    to_json_value(v),
                );
            }
            JsonValue::Object(obj)
        }
        Value::Ext(_, ext) => to_json_value(*ext),
    }
}
