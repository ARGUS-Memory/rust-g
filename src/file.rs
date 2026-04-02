use crate::error::Result;
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Write},
};

byond_fn!(fn file_read(path) {
    read(path).ok()
});

byond_fn!(fn file_exists(path) {
    Some(exists(path))
});

byond_fn!(fn file_write(data, path) {
    write(data, path).err()
});

byond_fn!(fn file_append(data, path) {
    append(data, path).err()
});

byond_fn!(fn file_get_line_count(path) {
    Some(get_line_count(path).ok()?.to_string())
});

byond_fn!(fn file_seek_line(path, line) {
    seek_line(path, match line.parse::<usize>() {
        Ok(line) => line,
        Err(_) => return None,
    })
});

fn read(path: &str) -> Result<String> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    let mut file = BufReader::new(file);

    let mut content = String::with_capacity(metadata.len() as usize);
    file.read_to_string(&mut content)?;
    let content = content.replace('\r', "");

    Ok(content)
}

fn exists(path: &str) -> String {
    let path = std::path::Path::new(path);
    path.exists().to_string()
}

fn write(data: &str, path: &str) -> Result<usize> {
    let path: &std::path::Path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = BufWriter::new(File::create(path)?);
    let written = file.write(data.as_bytes())?;

    file.flush()?;
    file.into_inner()
        .map_err(|e| std::io::Error::new(e.error().kind(), e.error().to_string()))? // This is god-awful, but the compiler REFUSES to let me get an owned copy of `e`
        .sync_all()?;

    Ok(written)
}

fn append(data: &str, path: &str) -> Result<usize> {
    let path: &std::path::Path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = BufWriter::new(OpenOptions::new().append(true).create(true).open(path)?);
    let written = file.write(data.as_bytes())?;

    file.flush()?;
    file.into_inner()
        .map_err(|e| std::io::Error::new(e.error().kind(), e.error().to_string()))?
        .sync_all()?;

    Ok(written)
}

fn get_line_count(path: &str) -> Result<u32> {
    let file = BufReader::new(File::open(path)?);
    Ok(file.lines().count() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_file_path(name: &str) -> String {
        let dir = std::env::temp_dir().join("rust_g_test_file");
        fs::create_dir_all(&dir).unwrap();
        dir.join(name).to_str().unwrap().to_owned()
    }

    #[test]
    fn test_write_and_read() {
        let path = test_file_path("write_read.txt");
        write("hello world", &path).unwrap();
        let content = read(&path).unwrap();
        assert_eq!(content, "hello world");
        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_append() {
        let path = test_file_path("append_test.txt");
        write("line1", &path).unwrap();
        append("\nline2", &path).unwrap();
        let content = read(&path).unwrap();
        assert_eq!(content, "line1\nline2");
        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_exists() {
        let path = test_file_path("exists_test.txt");
        write("data", &path).unwrap();
        assert_eq!(exists(&path), "true");
        fs::remove_file(&path).ok();
        assert_eq!(exists(&path), "false");
    }

    #[test]
    fn test_exists_nonexistent() {
        assert_eq!(exists("/nonexistent/path/file.txt"), "false");
    }

    #[test]
    fn test_get_line_count() {
        let path = test_file_path("line_count.txt");
        write("line1\nline2\nline3", &path).unwrap();
        let count = get_line_count(&path).unwrap();
        assert_eq!(count, 3);
        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_get_line_count_empty() {
        let path = test_file_path("empty_count.txt");
        write("", &path).unwrap();
        // An empty file has 0 or 1 lines depending on implementation
        let count = get_line_count(&path).unwrap();
        // BufReader::lines on empty string yields 0 lines
        assert!(count <= 1);
        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_seek_line_valid() {
        let path = test_file_path("seek_line.txt");
        write("line0\nline1\nline2", &path).unwrap();
        assert_eq!(seek_line(&path, 0), Some("line0".to_owned()));
        assert_eq!(seek_line(&path, 1), Some("line1".to_owned()));
        assert_eq!(seek_line(&path, 2), Some("line2".to_owned()));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_seek_line_out_of_range() {
        let path = test_file_path("seek_oob.txt");
        write("only one line", &path).unwrap();
        assert_eq!(seek_line(&path, 5), None);
        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_seek_line_nonexistent_file() {
        assert_eq!(seek_line("/no/such/file.txt", 0), None);
    }

    #[test]
    fn test_read_nonexistent() {
        assert!(read("/no/such/file.txt").is_err());
    }

    #[test]
    fn test_read_strips_carriage_returns() {
        let path = test_file_path("crlf.txt");
        // Write raw bytes with \r\n
        fs::write(&path, b"line1\r\nline2\r\n").unwrap();
        let content = read(&path).unwrap();
        assert!(!content.contains('\r'));
        assert!(content.contains("line1\nline2\n"));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_write_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("rust_g_test_file/nested/deep");
        let path = dir.join("test.txt").to_str().unwrap().to_owned();
        write("nested", &path).unwrap();
        assert_eq!(read(&path).unwrap(), "nested");
        fs::remove_dir_all(dir.parent().unwrap()).ok();
    }
}

fn seek_line(path: &str, line: usize) -> Option<String> {
    let file = BufReader::new(File::open(path).ok()?);
    file.lines().nth(line).and_then(std::result::Result::ok)
}
