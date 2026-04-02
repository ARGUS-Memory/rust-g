use crate::argus_json::{self, JsonPart, JsonValue};
use redis::{Client, Commands, RedisError};
use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::time::Duration;

thread_local! {
    static REDIS_CLIENT: RefCell<Option<Client>> = const { RefCell::new(None) };
}

fn connect(addr: &str) -> Result<(), RedisError> {
    let client = redis::Client::open(addr)?;
    let _ = client.get_connection_with_timeout(Duration::from_secs(1))?;
    REDIS_CLIENT.with(|cli| cli.replace(Some(client)));
    Ok(())
}

fn disconnect() {
    // Drop the client
    REDIS_CLIENT.with(|client| {
        client.replace(None);
    });
}

fn success_response(content: JsonPart<'_>) -> String {
    argus_json::json_obj_mixed(&[
        ("success", JsonPart::Bool(true)),
        ("content", content),
    ])
}

fn error_response(msg: &str) -> String {
    argus_json::json_obj_mixed(&[
        ("success", JsonPart::Bool(false)),
        ("content", JsonPart::Str(msg)),
    ])
}

/// <https://redis.io/commands/lpush/>
fn lpush(key: &str, data: JsonValue) -> String {
    REDIS_CLIENT.with(|client| {
        let client_ref = client.borrow();
        if let Some(client) = client_ref.as_ref() {
            return match client.get_connection() {
                Ok(mut conn) => {
                    // Need to handle the case of `[{}, {}]` and `{}`
                    let result = match &data {
                        JsonValue::Null => return error_response(
                            "Failed to perform LPUSH operation: Data sent was null"
                        ),
                        JsonValue::Array(arr) => {
                            let strings: Vec<String> = arr.iter().map(|v| argus_json::serialize_value(v)).collect();
                            conn.lpush::<&str, Vec<String>, isize>(key, strings)
                        }
                        _ => conn.lpush::<&str, String, isize>(key, argus_json::serialize_value(&data)),
                    };
                    return match result {
                        Ok(res) => {
                            let res_str = res.to_string();
                            argus_json::json_obj_mixed(&[
                                ("success", JsonPart::Bool(true)),
                                ("content", JsonPart::Raw(&res_str)),
                            ])
                        }
                        Err(e) => error_response(
                            &format!("Failed to perform LPUSH operation: {e}")
                        ),
                    };
                },
                Err(e) => {
                    error_response(&format!("Failed to get connection: {e}"))
                }
            }
        }
        error_response("Not Connected")
    })
}

/// <https://redis.io/commands/lrange/>
fn lrange(key: &str, start: isize, stop: isize) -> String {
    REDIS_CLIENT.with(|client| {
        let client_ref = client.borrow();
        if let Some(client) = client_ref.as_ref() {
            return match client.get_connection() {
                Ok(mut conn) => match conn.lrange::<&str, Vec<String>>(key, start, stop) {
                    Ok(res) => {
                        let arr = argus_json::serialize_value(
                            &JsonValue::Array(res.into_iter().map(JsonValue::Str).collect())
                        );
                        argus_json::json_obj_mixed(&[
                            ("success", JsonPart::Bool(true)),
                            ("content", JsonPart::Raw(&arr)),
                        ])
                    }
                    Err(e) => error_response(
                        &format!("Failed to perform LRANGE operation: {e}")
                    ),
                },
                Err(e) => error_response(&format!("Failed to get connection: {e}")),
            }
        }
        error_response("Not Connected")
    })
}

/// <https://redis.io/commands/lpop/>
fn lpop(key: &str, count: Option<NonZeroUsize>) -> String {
    REDIS_CLIENT.with(|client| {
        let client_ref = client.borrow();
        if let Some(client) = client_ref.as_ref() {
            let mut conn = match client.get_connection() {
                Ok(conn) => conn,
                Err(e) => {
                    return error_response(&format!("Failed to get connection: {e}"))
                }
            };
            // It will return either an Array or a BulkStr per ref
            // Yes, this code could be written more tersely but it's more intensive
            match count {
                None => {
                    let result = conn.lpop::<&str, String>(key, count);
                    return match result {
                        Ok(res) => success_response(JsonPart::Str(&res)),
                        Err(e) => error_response(
                            &format!("Failed to perform LPOP operation: {e}")
                        ),
                    };
                }
                Some(_) => {
                    let result = conn.lpop::<&str, Vec<String>>(key, count);
                    return match result {
                        Ok(res) => {
                            let arr = argus_json::serialize_value(
                                &JsonValue::Array(res.into_iter().map(JsonValue::Str).collect())
                            );
                            argus_json::json_obj_mixed(&[
                                ("success", JsonPart::Bool(true)),
                                ("content", JsonPart::Raw(&arr)),
                            ])
                        }
                        Err(e) => error_response(
                            &format!("Failed to perform LPOP operation: {e}")
                        ),
                    };
                }
            };
        }
        error_response("Not Connected")
    })
}

byond_fn!(fn redis_connect_rq(addr) {
    match connect(addr) {
        Ok(_) => Some(success_response(JsonPart::Str(""))),
        Err(e) => Some(error_response(
            &format!("Failed to connect to {addr}: {e}")
        )),
    }
});

byond_fn!(
    fn redis_disconnect_rq() {
        disconnect();
        Some(success_response(JsonPart::Str("")))
    }
);

byond_fn!(fn redis_lpush(key, elements) {
    match argus_json::parse_value(elements.as_bytes()) {
        Ok(elem) => Some(lpush(key, elem)),
        Err(_) => Some(error_response("Failed to deserialize JSON")),
    }
});

byond_fn!(fn redis_lrange(key, start, stop) {
    Some(lrange(key, start.parse().unwrap_or(0), stop.parse().unwrap_or(-1)))
});

byond_fn!(fn redis_lpop(key, count) {
    let count_parsed = if count.is_empty() {
        0
    } else {
        count.parse().unwrap_or(0)
    };
    Some(lpop(key, std::num::NonZeroUsize::new(count_parsed)))
});
