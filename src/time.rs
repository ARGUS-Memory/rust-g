use chrono::{FixedOffset, Local, Utc};
use std::{
    cell::RefCell,
    collections::hash_map::{Entry, HashMap},
    time::Instant,
};

thread_local!( static INSTANTS: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::with_capacity(16)) );

byond_fn!(fn time_microseconds(instant_id) {
    INSTANTS.with(|instants| {
        let mut map = instants.borrow_mut();
        let instant = match map.entry(instant_id.into()) {
            Entry::Occupied(elem) => elem.into_mut(),
            Entry::Vacant(elem) => elem.insert(Instant::now()),
        };
        Some(instant.elapsed().as_micros().to_string())
    })
});

byond_fn!(fn time_milliseconds(instant_id) {
    INSTANTS.with(|instants| {
        let mut map = instants.borrow_mut();
        let instant = match map.entry(instant_id.into()) {
            Entry::Occupied(elem) => elem.into_mut(),
            Entry::Vacant(elem) => elem.insert(Instant::now()),
        };
        Some(instant.elapsed().as_millis().to_string())
    })
});

byond_fn!(fn time_reset(instant_id) {
    INSTANTS.with(|instants| {
        let mut map = instants.borrow_mut();
        map.insert(instant_id.into(), Instant::now());
        Some("")
    })
});

byond_fn!(fn time_delete(instant_id) {
    INSTANTS.with(|instants| instants.borrow_mut().remove(instant_id));
    Some("")
});

byond_fn!(
    fn unix_timestamp() {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| format!("{:.6}", d.as_secs_f64()))
    }
);

byond_fn!(
    fn formatted_timestamp(format, offset) {
        format_timestamp(format, offset)
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timestamp_local() {
        let result = format_timestamp("%Y-%m-%d", "");
        assert!(result.is_some());
        let s = result.unwrap();
        // Should be in YYYY-MM-DD format
        assert_eq!(s.len(), 10);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
    }

    #[test]
    fn test_format_timestamp_utc_offset_zero() {
        let result = format_timestamp("%H:%M", "0");
        assert!(result.is_some());
        let s = result.unwrap();
        assert!(s.contains(':'));
    }

    #[test]
    fn test_format_timestamp_positive_offset() {
        let result = format_timestamp("%Y", "5");
        assert!(result.is_some());
    }

    #[test]
    fn test_format_timestamp_negative_offset() {
        let result = format_timestamp("%Y", "-8");
        assert!(result.is_some());
    }

    #[test]
    fn test_format_timestamp_invalid_offset() {
        let result = format_timestamp("%Y", "not_a_number");
        assert!(result.is_none());
    }

    #[test]
    fn test_format_timestamp_extreme_offset() {
        // Offset of 25 hours -> 90000 seconds, which exceeds FixedOffset bounds
        let result = format_timestamp("%Y", "25");
        assert!(result.is_none());
    }

    #[test]
    fn test_format_timestamp_various_formats() {
        assert!(format_timestamp("%Y-%m-%d %H:%M:%S", "").is_some());
        assert!(format_timestamp("%s", "").is_some()); // unix timestamp
        assert!(format_timestamp("no format specifiers", "").is_some());
    }
}

fn format_timestamp(format: &str, offset: &str) -> Option<String> {
    if offset.is_empty() {
        Some(Local::now().format(format).to_string())
    } else {
        let offset_seconds = offset.parse::<i32>().ok()? * 3600;
        let timezone = FixedOffset::east_opt(offset_seconds)?;
        Some(
            Utc::now()
                .with_timezone(&timezone)
                .format(format)
                .to_string(),
        )
    }
}
