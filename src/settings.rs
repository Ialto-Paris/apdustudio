// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The person's settings, kept between runs in a small text file (`ApduStudio.toml`) in the
//! system's settings folder: the theme, and whether 61xx and 6Cxx are followed.
//!
//! The file has a `version`. A file from a newer ApduStudio is read for what this version
//! knows, and never written over, so a newer version's settings are not lost.

use std::io;
use std::path::{Path, PathBuf};

use egui::ThemePreference;

/// The version this ApduStudio reads and writes.
pub const VERSION: u32 = 1;

const FILE_NAME: &str = "ApduStudio.toml";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub theme: ThemePreference,
    /// Send GET RESPONSE after 61xx and the command again after 6Cxx (off by default).
    pub follow_up: bool,
    /// The version the file was written with (`VERSION` for new settings).
    pub version: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            follow_up: false,
            version: VERSION,
        }
    }
}

impl Settings {
    /// Written by a newer ApduStudio: read, but never written over.
    pub fn is_newer(&self) -> bool {
        self.version > VERSION
    }
}

/// `ApduStudio.toml` in the system's settings folder: `%APPDATA%\ApduStudio` on Windows,
/// `~/Library/Application Support/ApduStudio` on macOS, `$XDG_CONFIG_HOME/apdu-studio` (or
/// `~/.config/apdu-studio`) elsewhere. None when the system names no such folder.
pub fn default_path() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let folder = if cfg!(windows) {
        var("APPDATA")?.join("ApduStudio")
    } else if cfg!(target_os = "macos") {
        var("HOME")?
            .join("Library")
            .join("Application Support")
            .join("ApduStudio")
    } else {
        var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".config")))?
            .join("apdu-studio")
    };
    Some(folder.join(FILE_NAME))
}

fn theme_name(t: ThemePreference) -> &'static str {
    match t {
        ThemePreference::System => "system",
        ThemePreference::Light => "light",
        ThemePreference::Dark => "dark",
    }
}

fn theme_of(name: &str) -> Option<ThemePreference> {
    match name {
        "system" => Some(ThemePreference::System),
        "light" => Some(ThemePreference::Light),
        "dark" => Some(ThemePreference::Dark),
        _ => None,
    }
}

/// Reads the settings from the file's text. Lines are `key = value`; `#` starts a comment.
/// Unknown keys and values that can't be read are left out: their setting keeps its default.
pub fn parse(text: &str) -> Settings {
    let mut s = Settings::default();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "version" => {
                if let Ok(v) = value.parse() {
                    s.version = v;
                }
            }
            "theme" => {
                if let Some(t) = theme_of(value) {
                    s.theme = t;
                }
            }
            "follow_up" => match value {
                "true" => s.follow_up = true,
                "false" => s.follow_up = false,
                _ => {}
            },
            _ => {}
        }
    }
    s
}

pub fn to_text(s: &Settings) -> String {
    format!(
        "# ApduStudio settings\nversion = {}\n# system, light or dark\ntheme = \"{}\"\n\
         # send GET RESPONSE after 61xx and the command again after 6Cxx: true or false\n\
         follow_up = {}\n",
        VERSION,
        theme_name(s.theme),
        s.follow_up
    )
}

/// The settings in `path`; the defaults when there is no file. A file that can't be read gives
/// the defaults too, with the reason.
pub fn load(path: &Path) -> (Settings, Option<io::Error>) {
    match std::fs::read(path) {
        Ok(bytes) => (parse(&String::from_utf8_lossy(&bytes)), None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Settings::default(), None),
        Err(e) => (Settings::default(), Some(e)),
    }
}

/// Writes the settings to `path`, creating its folder: to a temporary file first, then renamed,
/// so a failed write never leaves half a file.
pub fn save(path: &Path, s: &Settings) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, to_text(s))?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A file path of its own in the system's temporary folder, for one test.
    pub fn temp_file(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("apdu-studio-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("settings").join(FILE_NAME)
    }

    #[test]
    fn every_theme_is_read_back_as_written() {
        for theme in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            for follow_up in [false, true] {
                let s = Settings {
                    theme,
                    follow_up,
                    version: VERSION,
                };
                assert_eq!(parse(&to_text(&s)), s);
            }
        }
    }

    #[test]
    fn what_cannot_be_read_keeps_its_default() {
        assert_eq!(parse(""), Settings::default());
        assert_eq!(
            parse("theme = \"purple\"\nnonsense\n= 3\n"),
            Settings::default()
        );
        assert_eq!(parse("theme = dark # comment").theme, ThemePreference::Dark);
        assert_eq!(parse("  theme=light  ").theme, ThemePreference::Light);
        assert!(parse("follow_up = true").follow_up);
        assert!(!parse("follow_up = yes").follow_up, "only true or false");
    }

    #[test]
    fn a_newer_file_is_read_but_flagged() {
        let s = parse("version = 7\ntheme = \"dark\"\nzoom = 2\n");
        assert_eq!(s.theme, ThemePreference::Dark);
        assert!(s.is_newer());
        assert!(!Settings::default().is_newer());
    }

    #[test]
    fn saving_creates_the_folder_and_loading_reads_it_back() {
        let path = temp_file("save-load");
        assert_eq!(load(&path).0, Settings::default(), "no file: the defaults");
        assert!(load(&path).1.is_none(), "no file is not an error");
        let s = Settings {
            theme: ThemePreference::Light,
            follow_up: true,
            version: VERSION,
        };
        save(&path, &s).expect("save");
        assert_eq!(load(&path).0, s);
        let dark = Settings {
            theme: ThemePreference::Dark,
            ..s
        };
        save(&path, &dark).expect("save over");
        assert_eq!(load(&path).0, dark);
        assert!(!path.with_extension("toml.tmp").exists());
        if let Some(dir) = path.parent().and_then(Path::parent) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_folder_that_cannot_be_made_is_an_error_not_a_panic() {
        let path = temp_file("blocked");
        let parent = path.parent().and_then(Path::parent).map(PathBuf::from);
        let parent = parent.expect("a parent");
        std::fs::create_dir_all(&parent).expect("dir");
        // A file where the settings folder should be.
        std::fs::write(parent.join("settings"), b"x").expect("file");
        assert!(save(&path, &Settings::default()).is_err());
        let _ = std::fs::remove_dir_all(&parent);
    }
}
