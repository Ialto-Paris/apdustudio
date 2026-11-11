// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The version of the system's smart card stack, for About. PC/SC has no call that gives it, so
//! each system is asked its own way:
//!
//! - Windows: the file version of `WinSCard.dll`, read from its version resource;
//! - Linux and other pcsc-lite systems: `pcscd --version`, with a short time limit;
//! - macOS: the macOS version, which PCSC.framework follows.
//!
//! The lookup runs once, on a thread of its own, the first time it is asked for; until then,
//! and when it fails, there is no version and About shows the service's name alone.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The version once looked up: Some(None) when the lookup failed.
static VERSION: OnceLock<Option<String>> = OnceLock::new();
/// Whether the lookup was started.
static STARTED: OnceLock<()> = OnceLock::new();

/// The stack's version, if it is known: starts the lookup on first call, and returns None until
/// it's done or when it failed.
pub fn version() -> Option<&'static str> {
    if STARTED.set(()).is_ok() {
        let _ = std::thread::Builder::new()
            .name("stack-version".into())
            .spawn(|| {
                let _ = VERSION.set(look_up());
            });
    }
    VERSION.get().and_then(|v| v.as_deref())
}

/// Whether the lookup has finished (with a version or not).
pub fn known() -> bool {
    VERSION.get().is_some()
}

#[cfg(windows)]
fn look_up() -> Option<String> {
    let root = std::env::var_os("SystemRoot")?;
    let dll = std::path::Path::new(&root)
        .join("System32")
        .join("WinSCard.dll");
    file_version(&std::fs::read(dll).ok()?)
}

#[cfg(target_os = "macos")]
fn look_up() -> Option<String> {
    let plist = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").ok()?;
    plist_string(&plist, "ProductVersion").map(|v| format!("macOS {v}"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn look_up() -> Option<String> {
    // pcscd is usually in an sbin folder, which a person's PATH may not hold.
    [
        "/usr/sbin/pcscd",
        "/usr/local/sbin/pcscd",
        "/sbin/pcscd",
        "pcscd",
    ]
    .iter()
    .find_map(|p| run_for_output(p, "--version", Duration::from_secs(2)))
    .and_then(|out| pcscd_version(&out))
}

/// Runs `program arg` without input and returns what it printed, or None if it couldn't start,
/// failed, or took longer than `limit` (then it is stopped).
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn run_for_output(program: &str, arg: &str, limit: Duration) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .arg(arg)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    // A version line is short: what's printed fits in the pipe, so it's read after the exit.
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    out.push_str(&err);
    Some(out)
}

/// The version in `pcscd --version`'s output: "pcsc-lite version 2.0.3." gives "2.0.3".
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn pcscd_version(out: &str) -> Option<String> {
    let rest = out.split("version ").nth(1)?;
    let v: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let v = v.trim_end_matches('.');
    v.chars()
        .next()
        .filter(char::is_ascii_digit)
        .map(|_| v.to_string())
}

/// The string after `<key>name</key>` in a property list.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn plist_string(plist: &str, name: &str) -> Option<String> {
    let after = plist.split(&format!("<key>{name}</key>")).nth(1)?;
    let value = after.split("<string>").nth(1)?.split("</string>").next()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// A Windows file's version, from the VS_FIXEDFILEINFO of its version resource: the signature
/// 0xFEEF04BD, the structure's version, then the file version's two DWORDs (most significant
/// first): "10.0.26100.9949".
#[cfg_attr(not(windows), allow(dead_code))]
fn file_version(bytes: &[u8]) -> Option<String> {
    const SIGNATURE: [u8; 4] = [0xBD, 0x04, 0xEF, 0xFE];
    let at = bytes.windows(4).position(|w| w == SIGNATURE)?;
    let dword = |i: usize| -> Option<u32> {
        let b = bytes.get(at + i..at + i + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (ms, ls) = (dword(8)?, dword(12)?);
    Some(format!(
        "{}.{}.{}.{}",
        ms >> 16,
        ms & 0xFFFF,
        ls >> 16,
        ls & 0xFFFF
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_version_is_read_from_its_fixed_file_info() {
        let mut b = vec![0u8; 40];
        b.extend_from_slice(&[0xBD, 0x04, 0xEF, 0xFE]);
        b.extend_from_slice(&0x0001_0000u32.to_le_bytes());
        b.extend_from_slice(&(10u32 << 16).to_le_bytes());
        b.extend_from_slice(&((26100u32 << 16) | 9949).to_le_bytes());
        assert_eq!(file_version(&b).as_deref(), Some("10.0.26100.9949"));
        assert_eq!(file_version(&b[..50]), None, "cut short");
        assert_eq!(file_version(b"no resource here"), None);
    }

    #[test]
    fn pcscd_prints_its_version() {
        assert_eq!(
            pcscd_version("pcsc-lite version 2.0.3.\nCopyright (C) 1999-2002 ...").as_deref(),
            Some("2.0.3")
        );
        assert_eq!(pcscd_version("pcscd: unknown option"), None);
        assert_eq!(pcscd_version("version x"), None);
    }

    #[test]
    fn a_plist_string_is_read_by_its_key() {
        let p = "<dict>\n\t<key>ProductName</key>\n\t<string>macOS</string>\n\
                 \t<key>ProductVersion</key>\n\t<string>15.1</string>\n</dict>";
        assert_eq!(plist_string(p, "ProductVersion").as_deref(), Some("15.1"));
        assert_eq!(plist_string(p, "BuildVersion"), None);
    }
}
