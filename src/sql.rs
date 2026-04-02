use crate::argus_json::{self, JsonPart, JsonValue};
use crate::jobs;
use dashmap::DashMap;
use mysql::{
    OptsBuilder, Params, Pool, PoolConstraints, PoolOpts,
    consts::{ColumnFlags, ColumnType::*},
    prelude::Queryable,
};
use std::sync::LazyLock;
use std::{collections::HashMap, sync::atomic::AtomicUsize};
use std::{error::Error, time::Duration};

// ----------------------------------------------------------------------------
// Interface

const DEFAULT_PORT: u16 = 3306;
// The `mysql` crate defaults to 10 and 100 for these, but that is too large.
const DEFAULT_MIN_THREADS: usize = 1;
const DEFAULT_MAX_THREADS: usize = 10;

struct ConnectOptions {
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    pass: Option<String>,
    db_name: Option<String>,
    read_timeout: Option<f32>,
    write_timeout: Option<f32>,
    min_threads: Option<usize>,
    max_threads: Option<usize>,
}

impl ConnectOptions {
    fn from_json(src: &str) -> Result<Self, ()> {
        let val = argus_json::parse_value(src.as_bytes())?;
        Ok(ConnectOptions {
            host: val.get("host").and_then(|v| v.as_str()).map(|s| s.to_owned()),
            port: val.get("port").and_then(|v| v.as_i64()).map(|n| n as u16),
            user: val.get("user").and_then(|v| v.as_str()).map(|s| s.to_owned()),
            pass: val.get("pass").and_then(|v| v.as_str()).map(|s| s.to_owned()),
            db_name: val.get("db_name").and_then(|v| v.as_str()).map(|s| s.to_owned()),
            read_timeout: val.get("read_timeout").and_then(|v| v.as_f64()).map(|n| n as f32),
            write_timeout: val.get("write_timeout").and_then(|v| v.as_f64()).map(|n| n as f32),
            min_threads: val.get("min_threads").and_then(|v| v.as_i64()).map(|n| n as usize),
            max_threads: val.get("max_threads").and_then(|v| v.as_i64()).map(|n| n as usize),
        })
    }
}

byond_fn!(fn sql_connect_pool(options) {
    let options = match ConnectOptions::from_json(options) {
        Ok(options) => options,
        Err(_) => return Some(err_to_json("Invalid JSON options")),
    };
    Some(match sql_connect(options) {
        Ok(o) => argus_json::serialize_value(&o),
        Err(e) => err_to_json(e)
    })
});

byond_fn!(fn sql_query_blocking(handle, query, params) {
    Some(match do_query(handle, query, params) {
        Ok(o) => argus_json::serialize_value(&o),
        Err(e) => err_to_json(e)
    })
});

byond_fn!(fn sql_query_async(handle, query, params) {
    let handle = handle.to_owned();
    let query = query.to_owned();
    let params = params.to_owned();
    Some(jobs::start(move || {
        match do_query(&handle, &query, &params) {
            Ok(o) => argus_json::serialize_value(&o),
            Err(e) => err_to_json(e)
        }
    }))
});

// hopefully won't panic if queries are running
byond_fn!(fn sql_disconnect_pool(handle) {
    let handle = match handle.parse::<usize>() {
        Ok(o) => o,
        Err(e) => return Some(err_to_json(e)),
    };
    Some(
         match POOL.remove(&handle) {
            Some(_) => {
                argus_json::json_obj(&[("status", "success")])
            },
            None => argus_json::json_obj(&[("status", "offline")])
        }
    )
});

byond_fn!(fn sql_connected(handle) {
    let handle = match handle.parse::<usize>() {
        Ok(o) => o,
        Err(e) => return Some(err_to_json(e)),
    };
    Some(
        match POOL.get(&handle) {
            Some(_) => argus_json::json_obj(&[("status", "online")]),
            None => argus_json::json_obj(&[("status", "offline")])
        }
    )
});

byond_fn!(fn sql_check_query(id) {
    Some(jobs::check(id))
});

// ----------------------------------------------------------------------------
// Main connect and query implementation

static POOL: LazyLock<DashMap<usize, Pool>> = LazyLock::new(DashMap::new);
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn sql_connect(options: ConnectOptions) -> Result<JsonValue, Box<dyn Error>> {
    let pool_constraints = PoolConstraints::new(
        options.min_threads.unwrap_or(DEFAULT_MIN_THREADS),
        options.max_threads.unwrap_or(DEFAULT_MAX_THREADS),
    )
    .unwrap_or(PoolConstraints::new_const::<
        DEFAULT_MIN_THREADS,
        DEFAULT_MAX_THREADS,
    >());

    let pool_opts = PoolOpts::with_constraints(PoolOpts::new(), pool_constraints);

    let builder = OptsBuilder::new()
        .ip_or_hostname(options.host)
        .tcp_port(options.port.unwrap_or(DEFAULT_PORT))
        // Work around addresses like `localhost:3307` defaulting to socket as
        // if the port were the default too.
        .prefer_socket(options.port.is_none_or(|p| p == DEFAULT_PORT))
        .user(options.user)
        .pass(options.pass)
        .db_name(options.db_name)
        .read_timeout(options.read_timeout.map(Duration::from_secs_f32))
        .write_timeout(options.write_timeout.map(Duration::from_secs_f32))
        .pool_opts(pool_opts);

    let pool = Pool::new(builder)?;

    let handle = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    POOL.insert(handle, pool);
    Ok(JsonValue::Object(vec![
        ("status".to_owned(), JsonValue::Str("ok".to_owned())),
        ("handle".to_owned(), JsonValue::Str(handle.to_string())),
    ]))
}

fn do_query(handle: &str, query: &str, params: &str) -> Result<JsonValue, Box<dyn Error>> {
    let mut conn = {
        let pool = match POOL.get(&handle.parse()?) {
            Some(s) => s,
            None => {
                return Ok(JsonValue::Object(vec![
                    ("status".to_owned(), JsonValue::Str("offline".to_owned())),
                ]));
            }
        };
        pool.get_conn()?
    };

    let query_result = conn.exec_iter(query, params_from_json(params))?;
    let affected = query_result.affected_rows();
    let last_insert_id = query_result.last_insert_id();
    let mut columns = Vec::new();
    for col in query_result.columns().as_ref().iter() {
        columns.push(JsonValue::Object(vec![
            ("name".to_owned(), JsonValue::Str(col.name_str().to_string())),
            // Expansion room left for other column metadata.
        ]));
    }

    let mut rows: Vec<JsonValue> = Vec::new();
    for row in query_result {
        let row = row?;
        let mut json_row: Vec<JsonValue> = Vec::new();
        for (i, col) in row.columns_ref().iter().enumerate() {
            let ctype = col.column_type();
            let value = row
                .as_ref(i)
                .ok_or("length of row was smaller than column count")?;
            let converted = match value {
                mysql::Value::Bytes(b) => match ctype {
                    MYSQL_TYPE_VARCHAR | MYSQL_TYPE_STRING | MYSQL_TYPE_VAR_STRING => {
                        JsonValue::Str(String::from_utf8_lossy(b).into_owned())
                    }
                    MYSQL_TYPE_BLOB
                    | MYSQL_TYPE_LONG_BLOB
                    | MYSQL_TYPE_MEDIUM_BLOB
                    | MYSQL_TYPE_TINY_BLOB => {
                        if col.flags().contains(ColumnFlags::BINARY_FLAG) {
                            JsonValue::Array(
                                b.iter()
                                    .map(|x| JsonValue::Number(*x as f64))
                                    .collect(),
                            )
                        } else {
                            JsonValue::Str(String::from_utf8_lossy(b).into_owned())
                        }
                    }
                    _ => JsonValue::Null,
                },
                mysql::Value::Float(f) => {
                    let val = f64::from(*f);
                    if val.is_finite() {
                        JsonValue::Number(val)
                    } else {
                        JsonValue::Number(0.0)
                    }
                }
                mysql::Value::Double(f) => {
                    if f.is_finite() {
                        JsonValue::Number(*f)
                    } else {
                        JsonValue::Number(0.0)
                    }
                }
                mysql::Value::Int(i) => JsonValue::Number(*i as f64),
                mysql::Value::UInt(u) => JsonValue::Number(*u as f64),
                mysql::Value::Date(year, month, day, hour, minute, second, _ms) => {
                    JsonValue::Str(format!(
                        "{year}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
                    ))
                }
                _ => JsonValue::Null,
            };
            json_row.push(converted);
        }
        rows.push(JsonValue::Array(json_row));
    }

    drop(conn);

    Ok(JsonValue::Object(vec![
        ("status".to_owned(), JsonValue::Str("ok".to_owned())),
        ("affected".to_owned(), JsonValue::Number(affected as f64)),
        ("last_insert_id".to_owned(), match last_insert_id {
            Some(id) => JsonValue::Number(id as f64),
            None => JsonValue::Null,
        }),
        ("columns".to_owned(), JsonValue::Array(columns)),
        ("rows".to_owned(), JsonValue::Array(rows)),
    ]))
}

// ----------------------------------------------------------------------------
// Helpers

fn err_to_json<E: std::fmt::Display>(e: E) -> String {
    argus_json::json_obj_mixed(&[
        ("status", JsonPart::Str("err")),
        ("data", JsonPart::Str(&e.to_string())),
    ])
}

fn json_to_mysql(val: JsonValue) -> mysql::Value {
    match val {
        JsonValue::Bool(b) => mysql::Value::UInt(b as u64),
        JsonValue::Number(n) => {
            // Try unsigned integer first
            let i = n as i64;
            if (i as f64) == n && i >= 0 {
                let u = n as u64;
                if (u as f64) == n {
                    return mysql::Value::UInt(u);
                }
            }
            // Try signed integer
            if (i as f64) == n {
                return mysql::Value::Int(i);
            }
            // Fall back to float
            mysql::Value::Float(n as f32) // Loses precision.
        }
        JsonValue::Str(s) => mysql::Value::Bytes(s.into()),
        JsonValue::Array(a) => mysql::Value::Bytes(
            a.into_iter()
                .map(|x| {
                    if let JsonValue::Number(n) = x {
                        let u = n as u64;
                        if (u as f64) == n {
                            u as u8
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                })
                .collect(),
        ),
        _ => mysql::Value::NULL,
    }
}

fn array_to_params(params: Vec<JsonValue>) -> Params {
    if params.is_empty() {
        Params::Empty
    } else {
        Params::Positional(params.into_iter().map(json_to_mysql).collect())
    }
}

fn object_to_params(params: Vec<(String, JsonValue)>) -> Params {
    if params.is_empty() {
        Params::Empty
    } else {
        Params::Named(
            params
                .into_iter()
                .map(|(key, val)| {
                    let key_bytes: Vec<u8> = key.into_bytes();
                    (key_bytes, json_to_mysql(val))
                })
                .collect::<HashMap<_, _>>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connect_options_from_json_all_fields() {
        let opts = ConnectOptions::from_json(
            "{\"host\":\"localhost\",\"port\":3307,\"user\":\"root\",\"pass\":\"secret\",\"db_name\":\"test_db\",\"read_timeout\":5.0,\"write_timeout\":10.0,\"min_threads\":2,\"max_threads\":20}"
        ).unwrap();
        assert_eq!(opts.host.as_deref(), Some("localhost"));
        assert_eq!(opts.port, Some(3307));
        assert_eq!(opts.user.as_deref(), Some("root"));
        assert_eq!(opts.pass.as_deref(), Some("secret"));
        assert_eq!(opts.db_name.as_deref(), Some("test_db"));
        assert_eq!(opts.read_timeout, Some(5.0));
        assert_eq!(opts.write_timeout, Some(10.0));
        assert_eq!(opts.min_threads, Some(2));
        assert_eq!(opts.max_threads, Some(20));
    }

    #[test]
    fn test_connect_options_from_json_empty() {
        let opts = ConnectOptions::from_json("{}").unwrap();
        assert!(opts.host.is_none());
        assert!(opts.port.is_none());
        assert!(opts.user.is_none());
        assert!(opts.pass.is_none());
        assert!(opts.db_name.is_none());
        assert!(opts.read_timeout.is_none());
        assert!(opts.write_timeout.is_none());
        assert!(opts.min_threads.is_none());
        assert!(opts.max_threads.is_none());
    }

    #[test]
    fn test_connect_options_from_json_partial() {
        let opts = ConnectOptions::from_json("{\"host\":\"db.example.com\",\"port\":5432}").unwrap();
        assert_eq!(opts.host.as_deref(), Some("db.example.com"));
        assert_eq!(opts.port, Some(5432));
        assert!(opts.user.is_none());
    }

    #[test]
    fn test_connect_options_from_json_invalid() {
        assert!(ConnectOptions::from_json("not json").is_err());
    }

    #[test]
    fn test_connect_options_from_json_wrong_types() {
        // port as string should yield None
        let opts = ConnectOptions::from_json("{\"port\":\"not_a_number\"}").unwrap();
        assert!(opts.port.is_none());
    }

    #[test]
    fn test_params_from_json_array() {
        let params = params_from_json("[1, \"hello\", true, null]");
        match params {
            Params::Positional(p) => assert_eq!(p.len(), 4),
            _ => panic!("Expected positional params"),
        }
    }

    #[test]
    fn test_params_from_json_empty_array() {
        let params = params_from_json("[]");
        assert!(matches!(params, Params::Empty));
    }

    #[test]
    fn test_params_from_json_object() {
        let params = params_from_json("{\"name\":\"Alice\",\"age\":30}");
        match params {
            Params::Named(p) => assert_eq!(p.len(), 2),
            _ => panic!("Expected named params"),
        }
    }

    #[test]
    fn test_params_from_json_empty_object() {
        let params = params_from_json("{}");
        assert!(matches!(params, Params::Empty));
    }

    #[test]
    fn test_params_from_json_invalid() {
        let params = params_from_json("not json");
        assert!(matches!(params, Params::Empty));
    }

    #[test]
    fn test_params_from_json_scalar() {
        // A scalar (non-array, non-object) should return Empty
        let params = params_from_json("42");
        assert!(matches!(params, Params::Empty));
    }

    #[test]
    fn test_json_to_mysql_bool() {
        assert!(matches!(json_to_mysql(JsonValue::Bool(true)), mysql::Value::UInt(1)));
        assert!(matches!(json_to_mysql(JsonValue::Bool(false)), mysql::Value::UInt(0)));
    }

    #[test]
    fn test_json_to_mysql_number_unsigned_int() {
        match json_to_mysql(JsonValue::Number(42.0)) {
            mysql::Value::UInt(v) => assert_eq!(v, 42),
            other => panic!("Expected UInt, got {:?}", other),
        }
    }

    #[test]
    fn test_json_to_mysql_number_signed_int() {
        match json_to_mysql(JsonValue::Number(-5.0)) {
            mysql::Value::Int(v) => assert_eq!(v, -5),
            other => panic!("Expected Int, got {:?}", other),
        }
    }

    #[test]
    fn test_json_to_mysql_number_float() {
        match json_to_mysql(JsonValue::Number(3.14)) {
            mysql::Value::Float(_) => {}
            other => panic!("Expected Float, got {:?}", other),
        }
    }

    #[test]
    fn test_json_to_mysql_string() {
        match json_to_mysql(JsonValue::Str("hello".to_owned())) {
            mysql::Value::Bytes(b) => assert_eq!(b, b"hello"),
            other => panic!("Expected Bytes, got {:?}", other),
        }
    }

    #[test]
    fn test_json_to_mysql_null() {
        assert!(matches!(json_to_mysql(JsonValue::Null), mysql::Value::NULL));
    }

    #[test]
    fn test_json_to_mysql_array_of_numbers() {
        let val = JsonValue::Array(vec![
            JsonValue::Number(65.0),
            JsonValue::Number(66.0),
            JsonValue::Number(67.0),
        ]);
        match json_to_mysql(val) {
            mysql::Value::Bytes(b) => assert_eq!(b, vec![65u8, 66, 67]),
            other => panic!("Expected Bytes, got {:?}", other),
        }
    }

    #[test]
    fn test_err_to_json() {
        let result = err_to_json("something failed");
        assert!(result.contains("\"status\":\"err\""));
        assert!(result.contains("something failed"));
    }
}

fn params_from_json(params: &str) -> Params {
    match argus_json::parse_value(params.as_bytes()) {
        Ok(JsonValue::Object(o)) => object_to_params(o),
        Ok(JsonValue::Array(a)) => array_to_params(a),
        _ => Params::Empty,
    }
}
