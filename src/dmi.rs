use crate::argus_json::{self, JsonValue, escape_json_string};
use crate::error::{Error, Result};
use dmi::{
    error::DmiError,
    icon::{Icon, Looping},
};
use image::Rgba;
use png::{Decoder, Encoder, OutputInfo, Reader, text_metadata::ZTXtChunk};
use qrcode::{QrCode, render::svg};
use std::{
    fmt::Write,
    fs::{File, create_dir_all},
    io::BufReader,
    num::NonZeroU32,
    path::Path,
};

byond_fn!(fn dmi_strip_metadata(path) {
    strip_metadata(path).err()
});

byond_fn!(fn dmi_create_png(path, width, height, data) {
    create_png(path, width, height, data).err()
});

byond_fn!(fn dmi_resize_png(path, width, height, resizetype) {
    let resizetype = match resizetype {
        "catmull" => image::imageops::CatmullRom,
        "gaussian" => image::imageops::Gaussian,
        "lanczos3" => image::imageops::Lanczos3,
        "nearest" => image::imageops::Nearest,
        "triangle" => image::imageops::Triangle,
        _ => image::imageops::Nearest,
    };
    resize_png(path, width, height, resizetype).err()
});

byond_fn!(fn dmi_icon_states(path) {
    read_states(path).ok()
});

byond_fn!(fn dmi_read_metadata(path) {
    match read_metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) => {
            // Serialize the error string as a JSON string (quoted + escaped)
            let err_str = error.to_string();
            let mut out = String::with_capacity(err_str.len() + 2);
            out.push('"');
            escape_json_string(&err_str, &mut out);
            out.push('"');
            Some(out)
        },
    }
});

byond_fn!(fn dmi_inject_metadata(path, metadata) {
    inject_metadata(path, metadata).err()
});

fn strip_metadata(path: &str) -> Result<()> {
    let (reader, frame_info, image) = read_png(path)?;
    write_png(path, &reader, &frame_info, &image, true)
}

fn read_png(path: &str) -> Result<(Reader<BufReader<File>>, OutputInfo, Vec<u8>)> {
    let file = BufReader::new(File::open(path)?);
    let mut reader = Decoder::new(file).read_info()?;
    let buffer_size = reader.output_buffer_size().ok_or(Error::InvalidPngData)?;
    let mut buf = vec![0; buffer_size];
    let frame_info = reader.next_frame(&mut buf)?;

    Ok((reader, frame_info, buf))
}

fn write_png(
    path: &str,
    reader: &Reader<BufReader<File>>,
    info: &OutputInfo,
    image: &[u8],
    strip: bool,
) -> Result<()> {
    let mut encoder = Encoder::new(File::create(path)?, info.width, info.height);
    encoder.set_color(info.color_type);
    encoder.set_depth(info.bit_depth);

    let reader_info = reader.info();
    if let Some(palette) = reader_info.palette.clone() {
        encoder.set_palette(palette);
    }

    if let Some(trns_chunk) = reader_info.trns.clone() {
        encoder.set_trns(trns_chunk);
    }

    let mut writer = encoder.write_header()?;
    // Handles zTxt chunk copying from the original image if we /don't/ want to strip it
    if !strip {
        for chunk in &reader_info.compressed_latin1_text {
            writer.write_text_chunk(chunk)?;
        }
    }
    Ok(writer.write_image_data(image)?)
}

fn create_png(path: &str, width: &str, height: &str, data: &str) -> Result<()> {
    let width = width.parse::<u32>()?;
    let height = height.parse::<u32>()?;

    let bytes = data.as_bytes();

    let mut result: Vec<u8> = Vec::new();
    for pixel in bytes.split(|&b| b == b'#').skip(1) {
        if pixel.len() != 6 && pixel.len() != 8 {
            return Err(Error::InvalidPngData);
        }
        for channel in pixel.chunks_exact(2) {
            result.push(u8::from_str_radix(std::str::from_utf8(channel)?, 16)?);
        }
        // If only RGB is provided for any pixel we also add alpha
        if pixel.len() == 6 {
            result.push(255);
        }
    }

    if let Some(fdir) = Path::new(path).parent()
        && !fdir.is_dir()
    {
        create_dir_all(fdir)?;
    }

    let mut encoder = Encoder::new(File::create(path)?, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    Ok(writer.write_image_data(&result)?)
}

fn resize_png<P: AsRef<Path>>(
    path: P,
    width: &str,
    height: &str,
    resizetype: image::imageops::FilterType,
) -> std::result::Result<(), Error> {
    let width = width.parse::<u32>()?;
    let height = height.parse::<u32>()?;

    let img = image::open(path.as_ref())?;

    let newimg = img.resize(width, height, resizetype);

    Ok(newimg.save_with_format(path.as_ref(), image::ImageFormat::Png)?)
}

/// Output is a JSON string for reading within BYOND
///
/// Erroring at any point will produce an empty string
fn read_states(path: &str) -> Result<String> {
    let file = File::open(path).map(BufReader::new)?;
    let decoder = png::Decoder::new(file);
    let reader = decoder.read_info().map_err(|_| Error::InvalidPngData)?;
    let info = reader.info();
    let mut states = Vec::<String>::new();
    for ztxt in &info.compressed_latin1_text {
        let text = ztxt.get_text()?;
        text.lines()
            .take_while(|line| !line.contains("# END DMI"))
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("state = \"")
                    .and_then(|line| line.strip_suffix('"'))
            })
            .for_each(|state| {
                states.push(state.to_owned());
            });
    }
    // Serialize Vec<String> as JSON array of strings
    let arr: Vec<JsonValue> = states.into_iter().map(JsonValue::Str).collect();
    Ok(argus_json::serialize_value(&JsonValue::Array(arr)))
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum DmiStateDirCount {
    One = 1,
    Four = 4,
    Eight = 8,
}

impl TryFrom<u8> for DmiStateDirCount {
    type Error = u8;
    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::One),
            4 => Ok(Self::Four),
            8 => Ok(Self::Eight),
            n => Err(n),
        }
    }
}

struct DmiState {
    name: String,
    dirs: DmiStateDirCount,
    delay: Option<Vec<f32>>,
    rewind: Option<u8>,
    movement: Option<u8>,
    loop_count: Option<NonZeroU32>,
    hotspot: Option<(u32, u32, u32)>,
}

impl DmiState {
    fn to_json_value(&self) -> JsonValue {
        let mut pairs = Vec::new();
        pairs.push(("name".to_owned(), JsonValue::Str(self.name.clone())));
        pairs.push(("dirs".to_owned(), JsonValue::Number(self.dirs as u8 as f64)));
        if let Some(ref delay) = self.delay {
            let arr: Vec<JsonValue> = delay.iter().map(|&d| JsonValue::Number(d as f64)).collect();
            pairs.push(("delay".to_owned(), JsonValue::Array(arr)));
        }
        if let Some(rewind) = self.rewind {
            pairs.push(("rewind".to_owned(), JsonValue::Number(rewind as f64)));
        }
        if let Some(movement) = self.movement {
            pairs.push(("movement".to_owned(), JsonValue::Number(movement as f64)));
        }
        if let Some(loop_count) = self.loop_count {
            pairs.push(("loop_count".to_owned(), JsonValue::Number(loop_count.get() as f64)));
        }
        if let Some((x, y, z)) = self.hotspot {
            pairs.push(("hotspot".to_owned(), JsonValue::Array(vec![
                JsonValue::Number(x as f64),
                JsonValue::Number(y as f64),
                JsonValue::Number(z as f64),
            ])));
        }
        JsonValue::Object(pairs)
    }

    fn from_json(val: &JsonValue) -> std::result::Result<Self, Error> {
        let name = val.get("name")
            .and_then(|v| v.as_str())
            .ok_or(Error::InvalidPngData)?
            .to_owned();
        let dirs_num = val.get("dirs")
            .and_then(|v| v.as_i64())
            .ok_or(Error::InvalidPngData)? as u8;
        let dirs = DmiStateDirCount::try_from(dirs_num)
            .map_err(|_| Error::InvalidPngData)?;
        let delay = val.get("delay").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter().map(|item| item.as_f64().unwrap_or(0.0) as f32).collect()
            })
        });
        let rewind = val.get("rewind").and_then(|v| v.as_i64()).map(|n| n as u8);
        let movement = val.get("movement").and_then(|v| v.as_i64()).map(|n| n as u8);
        let loop_count = val.get("loop_count")
            .and_then(|v| v.as_i64())
            .and_then(|n| NonZeroU32::new(n as u32));
        let hotspot = val.get("hotspot").and_then(|v| {
            v.as_array().and_then(|arr| {
                if arr.len() == 3 {
                    Some((
                        arr[0].as_i64().unwrap_or(0) as u32,
                        arr[1].as_i64().unwrap_or(0) as u32,
                        arr[2].as_i64().unwrap_or(0) as u32,
                    ))
                } else {
                    None
                }
            })
        });
        Ok(DmiState {
            name,
            dirs,
            delay,
            rewind,
            movement,
            loop_count,
            hotspot,
        })
    }
}

struct DmiMetadata {
    width: u32,
    height: u32,
    states: Vec<DmiState>,
}

impl DmiMetadata {
    fn to_json_string(&self) -> String {
        let states_arr: Vec<JsonValue> = self.states.iter().map(|s| s.to_json_value()).collect();
        let val = JsonValue::Object(vec![
            ("width".to_owned(), JsonValue::Number(self.width as f64)),
            ("height".to_owned(), JsonValue::Number(self.height as f64)),
            ("states".to_owned(), JsonValue::Array(states_arr)),
        ]);
        argus_json::serialize_value(&val)
    }

    fn from_json(src: &str) -> std::result::Result<Self, Error> {
        let val = argus_json::parse_value(src.as_bytes()).map_err(|_| Error::InvalidPngData)?;
        let width = val.get("width")
            .and_then(|v| v.as_i64())
            .ok_or(Error::InvalidPngData)? as u32;
        let height = val.get("height")
            .and_then(|v| v.as_i64())
            .ok_or(Error::InvalidPngData)? as u32;
        let states_arr = val.get("states")
            .and_then(|v| v.as_array())
            .ok_or(Error::InvalidPngData)?;
        let states: Vec<DmiState> = states_arr
            .iter()
            .map(DmiState::from_json)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(DmiMetadata { width, height, states })
    }
}

fn read_metadata(path: &str) -> Result<String> {
    let dmi = Icon::load_meta(File::open(path).map(BufReader::new)?)?;
    let metadata = DmiMetadata {
        width: dmi.width,
        height: dmi.height,
        states: dmi
            .states
            .iter()
            .map(|state| {
                Ok(DmiState {
                    name: state.name.clone(),
                    dirs: DmiStateDirCount::try_from(state.dirs).map_err(|n| {
                        DmiError::IconState(format!(
                            "State \"{}\" has invalid dir count (expected 1, 4, or 8, got {})",
                            state.name, n
                        ))
                    })?,
                    delay: state.delay.clone(),
                    movement: state.movement.then_some(1),
                    rewind: state.rewind.then_some(1),
                    loop_count: match state.loop_flag {
                        Looping::Indefinitely => None,
                        Looping::NTimes(n) => Some(n),
                    },
                    hotspot: state.hotspot.map(|hotspot| (hotspot.x, hotspot.y, 1)),
                })
            })
            .collect::<Result<Vec<DmiState>>>()?,
    };
    Ok(metadata.to_json_string())
}

fn inject_metadata(path: &str, metadata: &str) -> Result<()> {
    let read_file = File::open(path).map(BufReader::new)?;
    let decoder = png::Decoder::new(read_file);
    let mut reader = decoder.read_info().map_err(|_| Error::InvalidPngData)?;
    let new_dmi_metadata = DmiMetadata::from_json(metadata)?;
    let mut new_metadata_string = String::new();
    writeln!(new_metadata_string, "# BEGIN DMI")?;
    writeln!(new_metadata_string, "version = 4.0")?;
    writeln!(new_metadata_string, "\twidth = {}", new_dmi_metadata.width)?;
    writeln!(
        new_metadata_string,
        "\theight = {}",
        new_dmi_metadata.height
    )?;
    for state in new_dmi_metadata.states {
        writeln!(new_metadata_string, "state = \"{}\"", state.name)?;
        writeln!(new_metadata_string, "\tdirs = {}", state.dirs as u8)?;
        writeln!(
            new_metadata_string,
            "\tframes = {}",
            state.delay.as_ref().map_or(1, Vec::len)
        )?;
        if let Some(delay) = state.delay {
            writeln!(
                new_metadata_string,
                "\tdelay = {}",
                delay
                    .iter()
                    .map(f32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            )?;
        }
        if state.rewind.is_some_and(|r| r != 0) {
            writeln!(new_metadata_string, "\trewind = 1")?;
        }
        if state.movement.is_some_and(|m| m != 0) {
            writeln!(new_metadata_string, "\tmovement = 1")?;
        }
        if let Some(loop_count) = state.loop_count {
            writeln!(new_metadata_string, "\tloop = {loop_count}")?;
        }
        if let Some((hotspot_x, hotspot_y, hotspot_frame)) = state.hotspot {
            writeln!(
                new_metadata_string,
                "\totspot = {hotspot_x},{hotspot_y},{hotspot_frame}"
            )?;
        }
    }
    writeln!(new_metadata_string, "# END DMI")?;
    let mut info = reader.info().clone();
    info.compressed_latin1_text
        .push(ZTXtChunk::new("Description", new_metadata_string));
    let mut raw_image_data: Vec<u8> = vec![];
    while let Some(row) = reader.next_row()? {
        raw_image_data.append(&mut row.data().to_vec());
    }
    let encoder = png::Encoder::with_info(File::create(path)?, info)?;
    encoder.write_header()?.write_image_data(&raw_image_data)?;
    Ok(())
}

byond_fn!(fn create_qr_code_png(path, data) {
    let code = match QrCode::new(data.as_bytes()) {
        Ok(code) => code,
        Err(err) => return Some(format!("Error: Could not read data into QR code: {err}"))
    };
    let image = code.render::<Rgba<u8>>().build();
    match image.save(path) {
        Ok(_) => Some(String::from(path)),
        Err(err) => Some(format!("Error: Could not write QR code image to path: {err}"))
    }
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::argus_json;
    use std::num::NonZeroU32;

    #[test]
    fn test_dmi_state_dir_count_valid() {
        assert!(matches!(DmiStateDirCount::try_from(1), Ok(DmiStateDirCount::One)));
        assert!(matches!(DmiStateDirCount::try_from(4), Ok(DmiStateDirCount::Four)));
        assert!(matches!(DmiStateDirCount::try_from(8), Ok(DmiStateDirCount::Eight)));
    }

    #[test]
    fn test_dmi_state_dir_count_invalid() {
        assert!(matches!(DmiStateDirCount::try_from(0), Err(0)));
        assert!(matches!(DmiStateDirCount::try_from(2), Err(2)));
        assert!(matches!(DmiStateDirCount::try_from(3), Err(3)));
        assert!(matches!(DmiStateDirCount::try_from(5), Err(5)));
        assert!(matches!(DmiStateDirCount::try_from(16), Err(16)));
    }

    #[test]
    fn test_dmi_state_to_json_minimal() {
        let state = DmiState {
            name: "idle".to_owned(),
            dirs: DmiStateDirCount::One,
            delay: None,
            rewind: None,
            movement: None,
            loop_count: None,
            hotspot: None,
        };
        let json_val = state.to_json_value();
        assert_eq!(json_val.get("name").unwrap().as_str(), Some("idle"));
        assert_eq!(json_val.get("dirs").unwrap().as_i64(), Some(1));
        assert!(json_val.get("delay").is_none());
        assert!(json_val.get("rewind").is_none());
        assert!(json_val.get("movement").is_none());
        assert!(json_val.get("loop_count").is_none());
        assert!(json_val.get("hotspot").is_none());
    }

    #[test]
    fn test_dmi_state_to_json_full() {
        let state = DmiState {
            name: "walk".to_owned(),
            dirs: DmiStateDirCount::Four,
            delay: Some(vec![1.0, 2.0, 3.0]),
            rewind: Some(1),
            movement: Some(1),
            loop_count: NonZeroU32::new(5),
            hotspot: Some((10, 20, 1)),
        };
        let json_val = state.to_json_value();
        assert_eq!(json_val.get("name").unwrap().as_str(), Some("walk"));
        assert_eq!(json_val.get("dirs").unwrap().as_i64(), Some(4));
        let delay_arr = json_val.get("delay").unwrap().as_array().unwrap();
        assert_eq!(delay_arr.len(), 3);
        assert_eq!(delay_arr[0].as_f64(), Some(1.0));
        assert_eq!(json_val.get("rewind").unwrap().as_i64(), Some(1));
        assert_eq!(json_val.get("movement").unwrap().as_i64(), Some(1));
        assert_eq!(json_val.get("loop_count").unwrap().as_i64(), Some(5));
        let hotspot = json_val.get("hotspot").unwrap().as_array().unwrap();
        assert_eq!(hotspot.len(), 3);
        assert_eq!(hotspot[0].as_i64(), Some(10));
    }

    #[test]
    fn test_dmi_state_roundtrip() {
        let state = DmiState {
            name: "test_state".to_owned(),
            dirs: DmiStateDirCount::Eight,
            delay: Some(vec![0.5, 1.0]),
            rewind: Some(1),
            movement: None,
            loop_count: NonZeroU32::new(3),
            hotspot: Some((5, 10, 1)),
        };
        let json_val = state.to_json_value();
        let reconstructed = DmiState::from_json(&json_val).unwrap();
        assert_eq!(reconstructed.name, "test_state");
        assert_eq!(reconstructed.dirs as u8, 8);
        assert_eq!(reconstructed.delay.as_ref().unwrap().len(), 2);
        assert_eq!(reconstructed.rewind, Some(1));
        assert!(reconstructed.movement.is_none());
        assert_eq!(reconstructed.loop_count.unwrap().get(), 3);
        assert_eq!(reconstructed.hotspot, Some((5, 10, 1)));
    }

    #[test]
    fn test_dmi_state_from_json_missing_name() {
        let val = argus_json::parse_value(b"{\"dirs\":1}").unwrap();
        assert!(DmiState::from_json(&val).is_err());
    }

    #[test]
    fn test_dmi_state_from_json_missing_dirs() {
        let val = argus_json::parse_value(b"{\"name\":\"test\"}").unwrap();
        assert!(DmiState::from_json(&val).is_err());
    }

    #[test]
    fn test_dmi_state_from_json_invalid_dirs() {
        let val = argus_json::parse_value(b"{\"name\":\"test\",\"dirs\":3}").unwrap();
        assert!(DmiState::from_json(&val).is_err());
    }

    #[test]
    fn test_dmi_metadata_roundtrip() {
        let metadata = DmiMetadata {
            width: 32,
            height: 32,
            states: vec![
                DmiState {
                    name: "idle".to_owned(),
                    dirs: DmiStateDirCount::One,
                    delay: None,
                    rewind: None,
                    movement: None,
                    loop_count: None,
                    hotspot: None,
                },
                DmiState {
                    name: "walk".to_owned(),
                    dirs: DmiStateDirCount::Four,
                    delay: Some(vec![1.0, 1.0, 1.0, 1.0]),
                    rewind: None,
                    movement: Some(1),
                    loop_count: None,
                    hotspot: None,
                },
            ],
        };
        let json_str = metadata.to_json_string();
        let reconstructed = DmiMetadata::from_json(&json_str).unwrap();
        assert_eq!(reconstructed.width, 32);
        assert_eq!(reconstructed.height, 32);
        assert_eq!(reconstructed.states.len(), 2);
        assert_eq!(reconstructed.states[0].name, "idle");
        assert_eq!(reconstructed.states[0].dirs as u8, 1);
        assert_eq!(reconstructed.states[1].name, "walk");
        assert_eq!(reconstructed.states[1].dirs as u8, 4);
        assert_eq!(reconstructed.states[1].movement, Some(1));
    }

    #[test]
    fn test_dmi_metadata_from_json_missing_width() {
        assert!(DmiMetadata::from_json("{\"height\":32,\"states\":[]}").is_err());
    }

    #[test]
    fn test_dmi_metadata_from_json_missing_height() {
        assert!(DmiMetadata::from_json("{\"width\":32,\"states\":[]}").is_err());
    }

    #[test]
    fn test_dmi_metadata_from_json_missing_states() {
        assert!(DmiMetadata::from_json("{\"width\":32,\"height\":32}").is_err());
    }

    #[test]
    fn test_dmi_metadata_from_json_invalid_json() {
        assert!(DmiMetadata::from_json("not json at all").is_err());
    }

    #[test]
    fn test_dmi_metadata_empty_states() {
        let json = "{\"width\":32,\"height\":32,\"states\":[]}";
        let metadata = DmiMetadata::from_json(json).unwrap();
        assert_eq!(metadata.width, 32);
        assert_eq!(metadata.height, 32);
        assert!(metadata.states.is_empty());
    }

    #[test]
    fn test_dmi_state_from_json_optional_fields_absent() {
        let val = argus_json::parse_value(b"{\"name\":\"s\",\"dirs\":1}").unwrap();
        let state = DmiState::from_json(&val).unwrap();
        assert!(state.delay.is_none());
        assert!(state.rewind.is_none());
        assert!(state.movement.is_none());
        assert!(state.loop_count.is_none());
        assert!(state.hotspot.is_none());
    }

    #[test]
    fn test_dmi_state_hotspot_wrong_length() {
        // Hotspot array with 2 elements instead of 3 should yield None
        let val = argus_json::parse_value(b"{\"name\":\"s\",\"dirs\":1,\"hotspot\":[1,2]}").unwrap();
        let state = DmiState::from_json(&val).unwrap();
        assert!(state.hotspot.is_none());
    }
}

byond_fn!(fn create_qr_code_svg(data) {
    let code = match QrCode::new(data.as_bytes()) {
        Ok(code) => code,
        Err(err) => return Some(format!("Error: Could not read data into QR code: {err}"))
    };
    let svg_xml = code.render::<svg::Color>().build();
    Some(svg_xml)
});
