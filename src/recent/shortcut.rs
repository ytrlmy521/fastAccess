use crate::model::{ItemKind, RecentItem};
use anyhow::{Context, Result};
use lnk::{
    encoding::{
        BIG5, EUC_KR, GBK, SHIFT_JIS, UTF_8, WINDOWS_1250, WINDOWS_1251, WINDOWS_1252,
        WINDOWS_1253, WINDOWS_1254, WINDOWS_1255, WINDOWS_1256, WINDOWS_1257, WINDOWS_1258,
        WINDOWS_874,
    },
    Encoding, FileAttributeFlags, ShellLink,
};
use std::path::{Path, PathBuf};

pub fn parse_shortcut(path: &Path, observed_at_ms: u64) -> Result<RecentItem> {
    let shortcut = ShellLink::open(path, system_ansi_encoding())
        .with_context(|| format!("cannot parse shortcut {}", path.display()))?;
    let target = shortcut
        .link_target()
        .filter(|target| !target.trim().is_empty())
        .map(PathBuf::from)
        .with_context(|| format!("shortcut has no filesystem target: {}", path.display()))?;

    let is_directory = shortcut
        .header()
        .file_attributes()
        .contains(FileAttributeFlags::FILE_ATTRIBUTE_DIRECTORY);
    let kind = if is_directory {
        ItemKind::Folder
    } else {
        ItemKind::File
    };

    Ok(RecentItem::new(target, observed_at_ms, kind))
}

fn system_ansi_encoding() -> Encoding {
    #[cfg(windows)]
    {
        encoding_for_code_page(unsafe { GetACP() })
    }

    #[cfg(not(windows))]
    {
        WINDOWS_1252
    }
}

fn encoding_for_code_page(code_page: u32) -> Encoding {
    match code_page {
        874 => WINDOWS_874,
        932 => SHIFT_JIS,
        936 => GBK,
        949 => EUC_KR,
        950 => BIG5,
        1250 => WINDOWS_1250,
        1251 => WINDOWS_1251,
        1252 => WINDOWS_1252,
        1253 => WINDOWS_1253,
        1254 => WINDOWS_1254,
        1255 => WINDOWS_1255,
        1256 => WINDOWS_1256,
        1257 => WINDOWS_1257,
        1258 => WINDOWS_1258,
        65001 => UTF_8,
        _ => WINDOWS_1252,
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetACP() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_simplified_chinese_system_code_page_to_gbk() {
        let bytes = [0xD6, 0xC7, 0xD3, 0xB0, 0xB5, 0xBC, 0xB3, 0xF6];
        let (decoded, had_errors) = encoding_for_code_page(936).decode_without_bom_handling(&bytes);

        assert!(!had_errors);
        assert_eq!(decoded, "智影导出");
    }
}
