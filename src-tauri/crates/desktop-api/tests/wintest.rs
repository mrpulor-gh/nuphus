#[test]
fn test_windows_foundation() {
    #[cfg(windows)]
    {
        // This test verifies that the windows crate's Win32 module is accessible
        let _hwnd = windows::Win32::Foundation::HWND::default();
    }
}
