use super::{hash, Application};
use std::path::{Path, PathBuf};
use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HWND};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, RRF_RT_REG_EXPAND_SZ,
    RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellExecuteW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowTextLengthW, GetWindowTextW, SW_SHOWNORMAL,
};

pub(super) fn window_title(hwnd: i32) -> Option<String> {
    let hwnd = HWND(hwnd as isize);
    let length = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut buffer = vec![0u16; length + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
    Some(String::from_utf16_lossy(&buffer[..copied]))
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(Some(0)).collect()
}
fn string(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|c| *c == 0).unwrap_or(value.len())])
}
fn app(path: PathBuf, name: String, launch_path: Option<PathBuf>) -> Option<Application> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    let id = format!(
        "windows-app:{}",
        hash(&format!(
            "process|{}",
            path.to_string_lossy().to_lowercase()
        ))
    );
    Some(Application {
        app_id: id,
        name,
        launch_path,
    })
}

pub(super) fn running(pid: u32) -> Option<Application> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = vec![0_u16; 32768];
        let mut length = buffer.len() as u32;
        let read = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(process);
        read.ok()?;
        let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
        let name = path.file_stem()?.to_string_lossy().into_owned();
        app(path, name, None)
    }
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn shortcut(path: &Path) -> Option<Application> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        let path_wide = wide(path.as_os_str());
        persist.Load(PCWSTR(path_wide.as_ptr()), STGM_READ).ok()?;
        let mut target = vec![0_u16; 32768];
        link.GetPath(&mut target, std::ptr::null_mut(), 0).ok()?;
        let target = PathBuf::from(string(&target));
        if !target.is_file() {
            return None;
        }
        app(
            target,
            path.file_stem()?.to_string_lossy().into_owned(),
            Some(path.to_owned()),
        )
    }
}

pub(super) fn installed() -> Vec<Application> {
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let _apartment = initialized.then_some(Apartment);
    let mut result = Vec::new();
    if initialized {
        let roots = [std::env::var_os("APPDATA"), std::env::var_os("PROGRAMDATA")];
        let mut pending: Vec<_> = roots
            .into_iter()
            .flatten()
            .map(|p| {
                (
                    PathBuf::from(p).join("Microsoft/Windows/Start Menu/Programs"),
                    0,
                )
            })
            .collect();
        let mut visited = 0;
        while let Some((directory, depth)) = pending.pop() {
            if visited >= 4096 {
                break;
            }
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                visited += 1;
                if visited > 4096 {
                    break;
                }
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() && depth < 6 {
                    pending.push((path, depth + 1));
                } else if kind.is_file()
                    && path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
                {
                    if let Some(app) = shortcut(&path) {
                        result.push(app);
                    }
                }
            }
        }
    }
    let key_path = wide(std::ffi::OsStr::new(
        "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths",
    ));
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let mut raw = HKEY::default();
            if unsafe {
                RegOpenKeyExW(
                    root,
                    PCWSTR(key_path.as_ptr()),
                    0,
                    KEY_READ | view,
                    &mut raw,
                )
            } != ERROR_SUCCESS
            {
                continue;
            }
            let key = Key(raw);
            for index in 0..2048 {
                let mut name = vec![0u16; 512];
                let mut length = name.len() as u32;
                if unsafe {
                    RegEnumKeyExW(
                        key.0,
                        index,
                        PWSTR(name.as_mut_ptr()),
                        &mut length,
                        None,
                        PWSTR::null(),
                        None,
                        None,
                    )
                } != ERROR_SUCCESS
                {
                    break;
                }
                let name = string(&name);
                let child = wide(std::ffi::OsStr::new(&name));
                let mut path = vec![0u16; 32768];
                let mut bytes = (path.len() * 2) as u32;
                if unsafe {
                    RegGetValueW(
                        key.0,
                        PCWSTR(child.as_ptr()),
                        PCWSTR::null(),
                        RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                        None,
                        Some(path.as_mut_ptr().cast()),
                        Some(&mut bytes),
                    )
                } != ERROR_SUCCESS
                {
                    continue;
                }
                let path = PathBuf::from(string(&path).trim_matches('"'));
                if path.is_file() {
                    if let Some(app) = app(
                        path.clone(),
                        name.trim_end_matches(".exe").into(),
                        Some(path),
                    ) {
                        result.push(app);
                    }
                }
            }
        }
    }
    result
}

pub(super) fn launch(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err("Catalog launch entry no longer exists".into());
    }
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|error| format!("Could not initialize native application launcher: {error}"))?;
    let _apartment = Apartment;
    let path = wide(path.as_os_str());
    let verb = wide(std::ffi::OsStr::new("open"));
    let result = unsafe {
        ShellExecuteW(
            HWND(0),
            PCWSTR(verb.as_ptr()),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 <= 32 {
        Err(format!("Native application launch failed ({})", result.0))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_id_uses_same_process_seed_as_uia() {
        let a = app(PathBuf::from("C:\\Apps\\Test.EXE"), "Test".into(), None).unwrap();
        assert_eq!(
            a.app_id,
            format!("windows-app:{}", hash("process|c:\\apps\\test.exe"))
        );
        assert!(app(PathBuf::from("C:\\Apps\\task.ps1"), "script".into(), None).is_none());
    }
}
