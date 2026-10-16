//! Thin wrapper over the Win32 registry calls, limited to `HKEY_CURRENT_USER`.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegDeleteKeyW,
    RegGetValueW, RegSetKeyValueW,
};

/// Sets one text value, creating `key` if needed; `""` is the key's default value.
pub fn set_value(key: &str, name: &str, value: &str) -> io::Result<()> {
    let data = wide(value);
    let byte_len = u32::try_from(data.len() * size_of::<u16>())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "registry value too long"))?;
    // SAFETY: every pointer is a NUL-terminated UTF-16 buffer that outlives the call,
    // and `data` is `byte_len` bytes long.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            wide(name).as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            byte_len,
        )
    };
    check(status)
}

/// Reads one text value; `None` when the key or the value does not exist or is not text.
pub fn get_value(key: &str, name: &str) -> Option<String> {
    let key = wide(key);
    let name = wide(name);
    let mut byte_len = 0u32;
    // SAFETY: the paths are NUL-terminated UTF-16 buffers; a null data pointer asks for the size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut byte_len,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let mut data = vec![0u16; (byte_len as usize).div_ceil(size_of::<u16>())];
    // SAFETY: `data` holds `byte_len` bytes, the size the first call reported.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            data.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let text = data.split(|&unit| unit == 0).next().unwrap_or_default();
    Some(String::from_utf16_lossy(text))
}

/// Reads one 32-bit number; `None` when the key or the value does not exist or is not one.
pub fn get_dword(key: &str, name: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut byte_len = size_of::<u32>() as u32;
    // SAFETY: the paths are NUL-terminated UTF-16 buffers; `value` holds `byte_len` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            wide(name).as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut value).cast(),
            &mut byte_len,
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}

/// Deletes one value; a value or key that does not exist is fine.
pub fn delete_value(key: &str, name: &str) -> io::Result<()> {
    // SAFETY: both paths are NUL-terminated UTF-16 buffers that outlive the call.
    let status =
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr()) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status)
}

/// Deletes `key` if it has no subkeys left; a key that does not exist is fine.
pub fn delete_key(key: &str) -> io::Result<()> {
    // SAFETY: the path is a NUL-terminated UTF-16 buffer that outlives the call.
    let status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide(key).as_ptr()) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status)
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain([0]).collect()
}

fn check(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        return Ok(());
    }
    Err(io::Error::from_raw_os_error(status as i32))
}
