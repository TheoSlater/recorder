use std::{mem::size_of, path::PathBuf};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

use anyhow::Result;

#[cfg(windows)]
pub(crate) fn choose(suggested_name: &str) -> Result<Option<PathBuf>> {
    use windows::{
        Win32::UI::Controls::Dialogs::{
            GetSaveFileNameW, OFN_EXPLORER, OFN_HIDEREADONLY, OFN_OVERWRITEPROMPT,
            OFN_PATHMUSTEXIST, OPENFILENAMEW,
        },
        core::{PCWSTR, PWSTR},
    };

    let mut file = utf16_buffer(suggested_name, 512);
    let filter: Vec<u16> = "MP4 video (*.mp4)\0*.mp4\0All files (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let extension: Vec<u16> = "mp4\0".encode_utf16().collect();
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        nFilterIndex: 1,
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT | OFN_HIDEREADONLY,
        ..Default::default()
    };

    if !unsafe { GetSaveFileNameW(&mut dialog) }.as_bool() {
        return Ok(None);
    }
    let end = file.iter().position(|value| *value == 0).unwrap_or(file.len());
    let path = PathBuf::from(String::from_utf16_lossy(&file[..end]));
    Ok((!path.as_os_str().is_empty()).then_some(path))
}

#[cfg(not(windows))]
pub(crate) fn choose(_suggested_name: &str) -> Result<Option<PathBuf>> {
    Ok(None)
}

#[cfg(windows)]
fn utf16_buffer(value: &str, capacity: usize) -> Vec<u16> {
    let mut buffer = vec![0; capacity];
    let value: Vec<u16> = std::ffi::OsStr::new(value).encode_wide().collect();
    let length = value.len().min(capacity.saturating_sub(1));
    buffer[..length].copy_from_slice(&value[..length]);
    buffer
}
