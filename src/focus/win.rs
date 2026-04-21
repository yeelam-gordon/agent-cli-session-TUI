use std::ptr;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::*;
use windows::Win32::System::Variant::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BSTR;

/// Focus a Windows Terminal tab whose name contains `search` (case-insensitive).
///
/// Steps:
/// 1. Initialize COM (idempotent if already initialized)
/// 2. Find all CASCADIA_HOSTING_WINDOW_CLASS windows (WT + Agentic Terminal)
/// 3. Search descendant TabItem elements for a name match
/// 4. Select the tab via SelectionItemPattern
/// 5. Bring the window to foreground (only SW_RESTORE if minimized)
///
/// Returns `true` if a matching tab was found and focused.
pub fn focus_wt_tab(search: &str) -> bool {
    let start = std::time::Instant::now();
    let result = unsafe { focus_wt_tab_inner(search).unwrap_or(false) };
    crate::log::info(&format!(
        "focus_wt_tab('{}') = {} in {:?}",
        search, result, start.elapsed()
    ));
    result
}

unsafe fn focus_wt_tab_inner(search: &str) -> windows::core::Result<bool> {
    let search_lower = search.to_lowercase();

    // COM init (safe to call multiple times — returns S_FALSE if already initialized)
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

    let uia: IUIAutomation =
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;

    let root = uia.GetRootElement()?;

    // Find all WT-class windows (covers wt, wtai, wta — all use CASCADIA_HOSTING_WINDOW_CLASS)
    let class_cond = uia.CreatePropertyCondition(
        UIA_ClassNamePropertyId,
        &make_bstr_variant("CASCADIA_HOSTING_WINDOW_CLASS"),
    )?;
    let wt_windows = root.FindAll(TreeScope_Children, &class_cond)?;

    // TabItem ControlType condition (reused across windows)
    let tab_cond = uia.CreatePropertyCondition(
        UIA_ControlTypePropertyId,
        &make_i4_variant(UIA_TabItemControlTypeId.0),
    )?;

    let win_count = wt_windows.Length()?;
    for i in 0..win_count {
        let w = wt_windows.GetElement(i)?;
        let tabs = w.FindAll(TreeScope_Descendants, &tab_cond)?;
        let tab_count = tabs.Length()?;

        for j in 0..tab_count {
            let tab = tabs.GetElement(j)?;
            let name: BSTR = tab.CurrentName()?;
            let name_str = name.to_string().to_lowercase();

            if name_str.contains(&search_lower) {
                // Select the tab
                let pattern: IUIAutomationSelectionItemPattern =
                    tab.GetCurrentPatternAs(UIA_SelectionItemPatternId)?;
                pattern.Select()?;

                // Bring window to foreground (preserve maximized state)
                let hwnd_raw = w.CurrentNativeWindowHandle()?;
                let hwnd = HWND(hwnd_raw.0);
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);

                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Build a VARIANT containing a BSTR value.
fn make_bstr_variant(s: &str) -> VARIANT {
    unsafe {
        let mut v = VARIANT::default();
        let inner = &mut *v.Anonymous.Anonymous;
        ptr::write(&raw mut inner.vt, VT_BSTR);
        ptr::write(
            &raw mut inner.Anonymous.bstrVal,
            std::mem::ManuallyDrop::new(BSTR::from(s)),
        );
        v
    }
}

/// Build a VARIANT containing an i32 value.
fn make_i4_variant(val: i32) -> VARIANT {
    unsafe {
        let mut v = VARIANT::default();
        let inner = &mut *v.Anonymous.Anonymous;
        ptr::write(&raw mut inner.vt, VT_I4);
        ptr::write(&raw mut inner.Anonymous.lVal, val);
        v
    }
}
