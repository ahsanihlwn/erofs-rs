// SPDX-License-Identifier: MIT

//! Android `fs_config`, `file_contexts`, and `mkfs_options` generator and parser.
//! Produces output fully compatible with Android ROM kitchens, `extract.erofs`, and `mkfs.erofs`.

use crate::error::{ErofsError, Result};
use crate::reader::{ErofsImage, FileNode};
use regex::Regex;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Escape regex metacharacters for Android `file_contexts` entries.
pub fn escape_file_context_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    for ch in path.chars() {
        if matches!(ch, '.' | '+' | '[' | ']' | '*' | '(' | ')' | '?' | '^' | '$') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Format one `fs_config` line: `<path> <uid> <gid> <mode_octal>[ capabilities=0x<HEX>]`.
pub fn format_fs_config_line(node: &FileNode) -> String {
    let mut line = format!(
        "{} {} {} {:04o}",
        node.path,
        node.uid,
        node.gid,
        node.mode & 0o7777
    );
    if let Some(cap) = node.capabilities {
        if cap != 0 {
            line.push_str(&format!(" capabilities=0x{cap:X}"));
        }
    }
    line
}

/// Format one `file_contexts` line: `<escaped_path> <selinux_context>`.
pub fn format_file_context_line(path: &str, selinux: &str) -> String {
    format!("{} {}", escape_file_context_path(path), selinux)
}

#[derive(Debug, Clone)]
pub struct GeneratedConfigs {
    pub fs_config_path: PathBuf,
    pub file_contexts_path: PathBuf,
    pub mkfs_options_path: Option<PathBuf>,
}

/// Write `<base>_fs_config`, `<base>_file_contexts`, and optionally `<base>_mkfs_options`.
pub fn write_android_configs(
    img: &ErofsImage,
    nodes: &[FileNode],
    config_dir: &Path,
    extract_dir: &Path,
    image_base_name: &str,
    write_mkfs_options: bool,
) -> Result<GeneratedConfigs> {
    fs::create_dir_all(config_dir)?;
    let fs_config_path = config_dir.join(format!("{image_base_name}_fs_config"));
    let file_contexts_path = config_dir.join(format!("{image_base_name}_file_contexts"));

    let mut fc_file = File::create(&fs_config_path)?;
    let mut sx_file = File::create(&file_contexts_path)?;

    let mut wrote_root_extras = false;
    for node in nodes {
        writeln!(fc_file, "{}", format_fs_config_line(node))?;
        if let Some(ctx) = &node.selinux {
            if !ctx.is_empty() {
                writeln!(sx_file, "{}", format_file_context_line(&node.path, ctx))?;
            }
        }
        if !wrote_root_extras && node.path == "/" {
            wrote_root_extras = true;
            writeln!(
                fc_file,
                "/lost+found {} {} {:04o}",
                node.uid,
                node.gid,
                node.mode & 0o7777
            )?;
            if let Some(ctx) = &node.selinux {
                if !ctx.is_empty() {
                    writeln!(sx_file, "{}", format_file_context_line("/lost+found", ctx))?;
                }
            }
        }
    }

    let mkfs_options_path = if write_mkfs_options {
        let p = config_dir.join(format!("{image_base_name}_mkfs_options"));
        let mut opt_file = File::create(&p)?;
        let ts = img.sb.build_timestamp();
        let uuid = img.sb.uuid_string();
        let big_pc = img.sb.has_big_pcluster();
        let digest_flag = if img.ishare_xattr_name().is_some() {
            "--xattr-inode-digest "
        } else {
            ""
        };
        let algo = img
            .sb
            .algorithm_names()
            .first()
            .copied()
            .unwrap_or("lz4hc");
        writeln!(opt_file, "Filesystem created:    {ts} (unix epoch)")?;
        writeln!(opt_file, "Filesystem UUID:       {uuid}")?;
        writeln!(
            opt_file,
            "mkfs.erofs options:    -z{algo} {}-T {ts} -U {uuid} {digest_flag}--fs-config-file={} --file-contexts={} {} {}",
            if big_pc { "-C 16384 " } else { "" },
            fs_config_path.display(),
            file_contexts_path.display(),
            format!("{image_base_name}_repack.img"),
            extract_dir.display()
        )?;
        writeln!(
            opt_file,
            "erofs-rs pack options: erofs-rs pack {} {} --algo {algo} {}--timestamp {ts} --uuid {uuid} --fs-config {} --file-contexts {}",
            extract_dir.display(),
            format!("{image_base_name}_repack.img"),
            if big_pc { "--pcluster 16384 " } else { "" },
            fs_config_path.display(),
            file_contexts_path.display(),
        )?;
        Some(p)
    } else {
        None
    };

    Ok(GeneratedConfigs {
        fs_config_path,
        file_contexts_path,
        mkfs_options_path,
    })
}

/// Parsed Android `fs_config` entry.
#[derive(Debug, Clone)]
pub struct FsConfigEntry {
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
    pub capabilities: u64,
}

/// Lookup table loaded from an Android `fs_config` file.
#[derive(Debug, Default)]
pub struct FsConfigTable {
    entries: HashMap<String, FsConfigEntry>,
}

impl FsConfigTable {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut entries = HashMap::new();
        for (lineno, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 4 {
                continue;
            }
            let key = normalize_cfg_path(parts[0]);
            let uid = parts[1].parse::<u32>().map_err(|_| {
                ErofsError::InvalidArg(format!("fs_config line {}: invalid uid", lineno + 1))
            })?;
            let gid = parts[2].parse::<u32>().map_err(|_| {
                ErofsError::InvalidArg(format!("fs_config line {}: invalid gid", lineno + 1))
            })?;
            let mode = u16::from_str_radix(parts[3], 8).map_err(|_| {
                ErofsError::InvalidArg(format!("fs_config line {}: invalid octal mode", lineno + 1))
            })?;
            let mut capabilities = 0u64;
            for extra in &parts[4..] {
                if let Some(hex) = extra.strip_prefix("capabilities=") {
                    let clean_hex = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
                    capabilities = u64::from_str_radix(clean_hex, 16).unwrap_or(0);
                }
            }
            entries.insert(
                key,
                FsConfigEntry {
                    uid,
                    gid,
                    mode,
                    capabilities,
                },
            );
        }
        Ok(Self { entries })
    }

    pub fn lookup(&self, path: &str, mount_point: &str) -> Option<&FsConfigEntry> {
        let norm = normalize_cfg_path(path);
        if let Some(e) = self.entries.get(&norm) {
            return Some(e);
        }
        let mp = mount_point.trim_matches('/');
        if !mp.is_empty() {
            let with_mp = if norm == "/" {
                format!("/{mp}")
            } else {
                format!("/{mp}{norm}")
            };
            if let Some(e) = self.entries.get(&with_mp) {
                return Some(e);
            }
            let without_lead = with_mp.trim_start_matches('/');
            if let Some(e) = self.entries.get(without_lead) {
                return Some(e);
            }
        }
        self.entries.get(norm.trim_start_matches('/'))
    }
}

fn normalize_cfg_path(p: &str) -> String {
    let s = p.replace('\\', "/");
    let trimmed = s.trim_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{trimmed}")
    }
}

struct FileContextRule {
    regex: Regex,
    file_type: Option<char>,
    context: String,
}

/// Matcher loaded from an Android `file_contexts` file.
#[derive(Default)]
pub struct FileContextsMatcher {
    exact: HashMap<String, String>,
    rules: Vec<FileContextRule>,
}

impl FileContextsMatcher {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut exact = HashMap::new();
        let mut rules = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            let (pattern, file_type, context) = match parts.len() {
                2 => (parts[0], None, parts[1]),
                3 if parts[1].starts_with('-') && parts[1].len() == 2 => {
                    (parts[0], parts[1].chars().nth(1), parts[2])
                }
                _ => continue,
            };
            if context == "<<none>>" {
                continue;
            }
            // Fast path: if pattern is an escaped literal path, index in exact map too.
            if file_type.is_none() && is_simple_escaped_literal(pattern) {
                let unescaped = unescape_literal(pattern);
                exact.insert(unescaped, context.to_string());
            }
            let anchored = format!("^(?:{pattern})$");
            if let Ok(regex) = Regex::new(&anchored) {
                rules.push(FileContextRule {
                    regex,
                    file_type,
                    context: context.to_string(),
                });
            }
        }
        Ok(Self { exact, rules })
    }

    /// Match a path against `file_contexts` (last matching rule wins, matching libselinux).
    pub fn lookup(&self, path: &str, mode_type_char: char, mount_point: &str) -> Option<&str> {
        let norm = normalize_cfg_path(path);
        let mp = mount_point.trim_matches('/');
        let full = if mp.is_empty() {
            norm.clone()
        } else if norm == "/" {
            format!("/{mp}")
        } else {
            format!("/{mp}{norm}")
        };

        // Check exact fast-path first if no regex overrides it later, or scan rules in reverse.
        for candidate in [&full, &norm] {
            if let Some(ctx) = self.exact.get(candidate) {
                return Some(ctx.as_str());
            }
        }
        for candidate in [&full, &norm] {
            for rule in self.rules.iter().rev() {
                if let Some(ft) = rule.file_type {
                    if ft != mode_type_char {
                        continue;
                    }
                }
                if rule.regex.is_match(candidate) {
                    return Some(rule.context.as_str());
                }
            }
        }
        None
    }
}

fn is_simple_escaped_literal(pat: &str) -> bool {
    let mut chars = pat.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if chars.next().is_none() {
                return false;
            }
        } else if matches!(c, '.' | '+' | '*' | '?' | '[' | ']' | '(' | ')' | '|' | '^' | '$' | '{' | '}') {
            return false;
        }
    }
    true
}

fn unescape_literal(pat: &str) -> String {
    let mut out = String::with_capacity(pat.len());
    let mut chars = pat.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}
