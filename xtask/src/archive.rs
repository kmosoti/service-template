//! Pure-Rust deterministic `.tar.gz` builder, `rustc -vV` host mapping, and binary path scan.

use std::io::Write;

use flate2::{Compression, GzBuilder};
use tar::{Builder, EntryType, Header};

pub fn scan_binary_for_host_paths(bin_bytes: &[u8], needles: &[&[u8]]) -> Result<(), String> {
    for needle in needles {
        if !needle.is_empty()
            && bin_bytes
                .windows(needle.len())
                .any(|window| window == *needle)
        {
            return Err(
                "release binary failed privacy gate: embedded host or user path bytes detected"
                    .to_owned(),
            );
        }
    }
    Ok(())
}

pub fn parse_rustc_host(rustc_vv: &str) -> Result<&str, String> {
    for line in rustc_vv.lines() {
        if let Some(host) = line.strip_prefix("host:") {
            let trimmed = host.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed);
            }
        }
    }
    Err("could not parse `host:` triple from `rustc -vV`".to_owned())
}

pub fn map_host_to_target_label(host: &str) -> Result<&'static str, String> {
    match host {
        "x86_64-unknown-linux-gnu" | "x86_64-unknown-linux-musl" => Ok("linux-x86_64"),
        "aarch64-unknown-linux-gnu" | "aarch64-unknown-linux-musl" => Ok("linux-aarch64"),
        "aarch64-apple-darwin" => Ok("macos-aarch64"),
        "x86_64-apple-darwin" => Ok("macos-x86_64"),
        "x86_64-pc-windows-msvc" | "x86_64-pc-windows-gnu" => Ok("windows-x86_64"),
        other => Err(format!("unsupported host target triple `{other}`")),
    }
}

pub fn build_deterministic_tar_gz(
    bin_name: &str,
    bin_bytes: &[u8],
    compat_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    let mut entries = [
        (bin_name, bin_bytes, 0o755u32),
        ("compatibility.json", compat_bytes, 0o644u32),
    ];
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let mut gz_encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    {
        let mut tar_builder = Builder::new(&mut gz_encoder);
        for (name, data, mode) in entries {
            let mut header = Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(mode);
            header.set_mtime(0);
            header.set_uid(0);
            header.set_gid(0);
            header.set_username("").map_err(|e| e.to_string())?;
            header.set_groupname("").map_err(|e| e.to_string())?;
            header.set_entry_type(EntryType::Regular);
            header.set_cksum();
            tar_builder
                .append_data(&mut header, name, data)
                .map_err(|e| format!("append tar entry {name}: {e}"))?;
        }
        tar_builder.finish().map_err(|e| e.to_string())?;
    }
    gz_encoder.flush().map_err(|e| e.to_string())?;
    gz_encoder.finish().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        build_deterministic_tar_gz, map_host_to_target_label, parse_rustc_host,
        scan_binary_for_host_paths,
    };

    #[test]
    fn maps_host_triples_scans_binaries_and_builds_reproducible_tar_gz() {
        let vv = "rustc 1.100.0\nhost: aarch64-apple-darwin\nrelease: 1.100.0\n";
        assert_eq!(parse_rustc_host(vv), Ok("aarch64-apple-darwin"));
        assert_eq!(
            map_host_to_target_label("aarch64-apple-darwin"),
            Ok("macos-aarch64")
        );
        assert!(map_host_to_target_label("mips-unknown-linux-gnu").is_err());

        let leaked = ["ELF", "home", "x"].join("/");
        assert!(scan_binary_for_host_paths(leaked.as_bytes(), &[b"/home/"]).is_err());
        assert!(scan_binary_for_host_paths(b"ELF/workspace/src", &[b"/home/"]).is_ok());

        let a1 = build_deterministic_tar_gz("admission-engine", b"bin", b"{}").unwrap();
        let a2 = build_deterministic_tar_gz("admission-engine", b"bin", b"{}").unwrap();
        assert_eq!(a1, a2);
    }
}
