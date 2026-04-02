use crate::argus_json::{self, JsonPart};
use crate::{error::Result, jobs};
use std::sync::LazyLock;
use std::io::Write;
use std::time::Duration;

// ----------------------------------------------------------------------------
// Interface

struct RequestOptions {
    output_filename: Option<String>,
    body_filename: Option<String>,
    timeout_seconds: Option<u64>,
}

impl RequestOptions {
    fn from_json(src: &str) -> std::result::Result<Self, ()> {
        let val = argus_json::parse_value(src.as_bytes())?;
        Ok(Self {
            output_filename: val.get("output_filename").and_then(|v| v.as_str()).map(|s| s.to_owned()),
            body_filename: val.get("body_filename").and_then(|v| v.as_str()).map(|s| s.to_owned()),
            timeout_seconds: val.get("timeout_seconds").and_then(|v| v.as_i64()).and_then(|n| u64::try_from(n).ok()),
        })
    }
}

// If the response can be deserialized -> success.
// If the response can't be deserialized -> failure.
byond_fn!(fn http_request_blocking(method, url, body, headers, options) {
    let req = match construct_request(method, url, body, headers, options) {
        Ok(r) => r,
        Err(e) => return Some(e.to_string())
    };

    match submit_request(req) {
        Ok(r) => Some(r),
        Err(e) => Some(e.to_string())
    }
});

// Returns new job-id.
byond_fn!(fn http_request_async(method, url, body, headers, options) {
    let req = match construct_request(method, url, body, headers, options) {
        Ok(r) => r,
        Err(e) => return Some(e.to_string())
    };

    Some(jobs::start(move || {
        match submit_request(req) {
            Ok(r) => r,
            Err(e) => e.to_string()
        }
    }))
});

byond_fn!(fn http_request_fire_and_forget(method, url, body, headers, options) {
    let req = match construct_request(method, url, body, headers, options) {
        Ok(r) => r,
        Err(e) => return Some(e.to_string())
    };

    std::thread::spawn(move || {
        let _ = req.req.send_bytes(&req.body); // discard result
    });
    Some("ok".to_owned())
});

// If the response can be deserialized -> success.
// If the response can't be deserialized -> failure or WIP.
byond_fn!(fn http_check_request(id) {
    Some(jobs::check(id))
});

// ----------------------------------------------------------------------------
// Shared HTTP client state

const VERSION: &str = env!("CARGO_PKG_VERSION");
const PKG_NAME: &str = env!("CARGO_PKG_NAME");

pub static HTTP_CLIENT: LazyLock<ureq::Agent> = LazyLock::new(ureq::agent);

// ----------------------------------------------------------------------------
// Request construction and execution

struct RequestPrep {
    req: ureq::Request,
    output_filename: Option<String>,
    body: Vec<u8>,
}

fn construct_request(
    method: &str,
    url: &str,
    body: &str,
    headers: &str,
    options: &str,
) -> Result<RequestPrep> {
    let mut req = match method {
        "post" => HTTP_CLIENT.post(url),
        "put" => HTTP_CLIENT.put(url),
        "patch" => HTTP_CLIENT.patch(url),
        "delete" => HTTP_CLIENT.delete(url),
        "head" => HTTP_CLIENT.head(url),
        _ => HTTP_CLIENT.get(url),
    }
    .set("User-Agent", &format!("{PKG_NAME}/{VERSION}"));

    let mut final_body = body.as_bytes().to_vec();

    if !headers.is_empty() {
        let header_pairs = argus_json::parse_string_map(headers.as_bytes())
            .map_err(|_| crate::error::Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid headers JSON")))?;
        for (key, value) in header_pairs {
            req = req.set(&key, &value);
        }
    }

    let mut output_filename = None;
    if !options.is_empty() {
        let options = RequestOptions::from_json(options)
            .map_err(|_| crate::error::Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid options JSON")))?;
        output_filename = options.output_filename;
        if let Some(fname) = options.body_filename {
            final_body = std::fs::read(fname)?;
        }

        if let Some(timeout_seconds) = options.timeout_seconds {
            req = req.timeout(Duration::from_secs(timeout_seconds));
        }
    }

    Ok(RequestPrep {
        req,
        output_filename,
        body: final_body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_options_from_json_empty() {
        let opts = RequestOptions::from_json("{}").unwrap();
        assert!(opts.output_filename.is_none());
        assert!(opts.body_filename.is_none());
        assert!(opts.timeout_seconds.is_none());
    }

    #[test]
    fn test_request_options_from_json_all_fields() {
        let opts = RequestOptions::from_json(
            "{\"output_filename\":\"out.txt\",\"body_filename\":\"body.json\",\"timeout_seconds\":30}"
        ).unwrap();
        assert_eq!(opts.output_filename.as_deref(), Some("out.txt"));
        assert_eq!(opts.body_filename.as_deref(), Some("body.json"));
        assert_eq!(opts.timeout_seconds, Some(30));
    }

    #[test]
    fn test_request_options_from_json_partial() {
        let opts = RequestOptions::from_json("{\"timeout_seconds\":5}").unwrap();
        assert!(opts.output_filename.is_none());
        assert!(opts.body_filename.is_none());
        assert_eq!(opts.timeout_seconds, Some(5));
    }

    #[test]
    fn test_request_options_from_json_invalid() {
        assert!(RequestOptions::from_json("not json").is_err());
    }

    #[test]
    fn test_request_options_from_json_wrong_types_ignored() {
        // Non-string for output_filename should yield None
        let opts = RequestOptions::from_json("{\"output_filename\":42}").unwrap();
        assert!(opts.output_filename.is_none());
    }

    #[test]
    fn test_construct_request_get() {
        let prep = construct_request("get", "http://example.com", "", "", "").unwrap();
        assert!(prep.output_filename.is_none());
        assert!(prep.body.is_empty());
    }

    #[test]
    fn test_construct_request_post_with_body() {
        let prep = construct_request("post", "http://example.com", "hello", "", "").unwrap();
        assert_eq!(prep.body, b"hello");
    }

    #[test]
    fn test_construct_request_with_headers() {
        let prep = construct_request(
            "get", "http://example.com", "",
            "{\"Content-Type\":\"application/json\",\"X-Custom\":\"value\"}", ""
        ).unwrap();
        assert!(prep.output_filename.is_none());
    }

    #[test]
    fn test_construct_request_invalid_headers() {
        let result = construct_request("get", "http://example.com", "", "not json", "");
        assert!(result.is_err());
    }

    #[test]
    fn test_construct_request_with_options() {
        let prep = construct_request(
            "get", "http://example.com", "", "",
            "{\"output_filename\":\"download.bin\",\"timeout_seconds\":10}"
        ).unwrap();
        assert_eq!(prep.output_filename.as_deref(), Some("download.bin"));
    }

    #[test]
    fn test_construct_request_methods() {
        // All valid methods should construct without error
        for method in &["get", "post", "put", "patch", "delete", "head", "unknown"] {
            let result = construct_request(method, "http://example.com", "", "", "");
            assert!(result.is_ok(), "Failed for method: {}", method);
        }
    }
}

fn submit_request(prep: RequestPrep) -> Result<String> {
    let response = prep.req.send_bytes(&prep.body).map_err(Box::new)?;

    let status_code = response.status();

    // Collect headers
    let mut header_pairs: Vec<(&str, JsonPart<'_>)> = Vec::new();
    let mut header_names: Vec<String> = Vec::new();
    let mut header_values: Vec<String> = Vec::new();
    for key in response.headers_names() {
        let Some(value) = response.header(&key) else {
            continue;
        };
        header_names.push(key);
        header_values.push(value.to_owned());
    }
    for (k, v) in header_names.iter().zip(header_values.iter()) {
        header_pairs.push((k.as_str(), JsonPart::Str(v.as_str())));
    }
    let headers_json = argus_json::json_obj_mixed(&header_pairs);

    if let Some(output_filename) = prep.output_filename {
        let mut writer = std::io::BufWriter::new(std::fs::File::create(output_filename)?);
        std::io::copy(&mut response.into_reader(), &mut writer)?;
        writer.flush()?;

        Ok(argus_json::json_obj_mixed(&[
            ("status_code", JsonPart::Int(status_code as i64)),
            ("headers", JsonPart::Raw(&headers_json)),
            ("body", JsonPart::Null),
        ]))
    } else {
        let body = response.into_string()?;

        Ok(argus_json::json_obj_mixed(&[
            ("status_code", JsonPart::Int(status_code as i64)),
            ("headers", JsonPart::Raw(&headers_json)),
            ("body", JsonPart::Str(&body)),
        ]))
    }
}
