use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Find the final N lines with fixed-size backwards reads, then stream forwards.
/// The whole file and its line index never need to be kept in memory.
pub(crate) fn tail_reader(path: &Path, lines: usize) -> io::Result<BufReader<File>> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular log file",
        ));
    }
    let len = metadata.len();
    let mut position = len;
    let mut start = if lines == 0 { len } else { 0 };
    let mut remaining = lines;
    let mut block = [0; 8192];
    'search: while position > 0 && remaining > 0 {
        let offset = position.saturating_sub(block.len() as u64);
        let count = (position - offset) as usize;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut block[..count])?;
        for index in (0..count).rev() {
            let absolute = offset + index as u64;
            // A trailing newline terminates the last line; it does not add one.
            if block[index] == b'\n' && absolute + 1 < len {
                remaining -= 1;
                if remaining == 0 {
                    start = absolute + 1;
                    break 'search;
                }
            }
        }
        position = offset;
    }
    file.seek(SeekFrom::Start(start))?;
    Ok(BufReader::new(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn tails_exact_lines_across_blocks_and_line_endings() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for content in [
            "",
            "\n",
            "one\ntwo\nthree",
            "one\r\ntwo\r\nthree\r\n",
            "one\n\n",
        ] {
            std::fs::write(file.path(), content).unwrap();
            for count in 0..5 {
                let mut result = String::new();
                tail_reader(file.path(), count)
                    .unwrap()
                    .read_to_string(&mut result)
                    .unwrap();
                let expected: Vec<_> = content.lines().rev().take(count).collect();
                assert_eq!(result.lines().rev().collect::<Vec<_>>(), expected);
            }
        }
        file.as_file_mut().set_len(0).unwrap();
        file.write_all("prefix\n".repeat(10_000).as_bytes())
            .unwrap();
        file.write_all(b"penultimate\nlast\n").unwrap();
        let mut reader = tail_reader(file.path(), 2).unwrap();
        assert!(reader.stream_position().unwrap() > 65_536);
        let mut result = String::new();
        reader.read_to_string(&mut result).unwrap();
        assert_eq!(result, "penultimate\nlast\n");
        let mut reader = tail_reader(file.path(), 2_000).unwrap();
        result.clear();
        reader.read_to_string(&mut result).unwrap();
        assert_eq!(result.lines().count(), 2_000);
        assert!(result.starts_with("prefix\n"));
        assert!(result.ends_with("penultimate\nlast\n"));
    }
}
