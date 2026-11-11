// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The log file: what was sent to the card and what came back, kept as the record of a test.
//! Every write is flushed at once, and a failed write is an error the window shows.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub struct Journal {
    file: File,
    path: PathBuf,
}

impl Journal {
    /// Starts a new log file, replacing one that is there.
    pub fn create(path: &Path) -> io::Result<Self> {
        Ok(Journal {
            file: File::create(path)?,
            path: path.to_path_buf(),
        })
    }

    /// Continues the log file there, or starts one.
    pub fn append(path: &Path) -> io::Result<Self> {
        Ok(Journal {
            file: OpenOptions::new().append(true).create(true).open(path)?,
            path: path.to_path_buf(),
        })
    }

    /// A log file that refuses every write (tests of what the window does then).
    #[cfg(test)]
    pub fn read_only(path: &Path) -> io::Result<Self> {
        Ok(Journal {
            file: File::open(path)?,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `text` with Windows line endings and flushes it.
    pub fn write(&mut self, text: &str) -> io::Result<()> {
        self.file.write_all(text.replace('\n', "\r\n").as_bytes())?;
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file of its own in the temp folder, removed when the test ends.
    struct Temp(PathBuf);
    impl Temp {
        fn new(name: &str) -> Self {
            Temp(std::env::temp_dir().join(format!("apdu-studio-{}-{name}", std::process::id())))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn lines_end_with_crlf_and_append_continues() {
        let p = Temp::new("crlf.log");
        Journal::create(&p.0)
            .and_then(|mut j| j.write("a\nb\n"))
            .expect("write");
        Journal::append(&p.0)
            .and_then(|mut j| j.write("c\n"))
            .expect("append");
        assert_eq!(std::fs::read(&p.0).expect("read"), b"a\r\nb\r\nc\r\n");
    }

    #[test]
    fn a_write_that_fails_is_an_error() {
        let p = Temp::new("ro.log");
        std::fs::write(&p.0, "").expect("create");
        let mut j = Journal::read_only(&p.0).expect("open");
        assert!(j.write("x\n").is_err());
    }
}
