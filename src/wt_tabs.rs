//! Enumerate Windows Terminal tab titles via UI Automation.
//!
//! Used by the `tab_title_match` liveness strategy — for agents (qwen) that
//! neither lock a file nor carry a session UUID on their cmdline, the only
//! durable signal of "this session is currently live" is the tab subject
//! written via OSC 2 to the WT titlebar. UIA exposes these as the `Name`
//! property of each `TabItem` child of a CASCADIA_HOSTING_WINDOW_CLASS window.
//!
//! This is read-only; see `src/focus/win.rs` for the read-write analogue that
//! selects a specific tab. COM initialization is idempotent across the two.

use anyhow::Result;
use std::ptr;
use windows::Win32::System::Com::*;
use windows::Win32::System::Variant::*;
use windows::Win32::UI::Accessibility::*;
use windows::core::BSTR;

/// Return the `Name` (tab title) of every `TabItem` inside any Windows-Terminal
/// class window currently open on the desktop. Order is undefined.
///
/// Returns `Ok(vec![])` when no WT windows are open. Returns `Err` only for
/// UI-Automation failures (e.g. COM init returned an unexpected error).
pub fn list_tab_titles() -> Result<Vec<String>> {
    unsafe { list_tab_titles_inner() }
        .map_err(|e| anyhow::anyhow!("UIA tab enumeration failed: {e}"))
}

unsafe fn list_tab_titles_inner() -> windows::core::Result<Vec<String>> {
    // COM init — idempotent. Returns S_FALSE if already initialized; that's fine.
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

    let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
    let root = uia.GetRootElement()?;

    let class_cond = uia.CreatePropertyCondition(
        UIA_ClassNamePropertyId,
        &make_bstr_variant("CASCADIA_HOSTING_WINDOW_CLASS"),
    )?;
    let wt_windows = root.FindAll(TreeScope_Children, &class_cond)?;

    let tab_cond = uia.CreatePropertyCondition(
        UIA_ControlTypePropertyId,
        &make_i4_variant(UIA_TabItemControlTypeId.0),
    )?;

    let mut out: Vec<String> = Vec::new();
    let win_count = wt_windows.Length()?;
    for i in 0..win_count {
        let w = wt_windows.GetElement(i)?;
        let tabs = w.FindAll(TreeScope_Descendants, &tab_cond)?;
        let tab_count = tabs.Length()?;
        for j in 0..tab_count {
            let tab = tabs.GetElement(j)?;
            let name: BSTR = tab.CurrentName()?;
            let s = name.to_string();
            if !s.is_empty() {
                out.push(s);
            }
        }
    }
    Ok(out)
}

// ── VARIANT helpers (mirror src/focus/win.rs) ─────────────────────────────

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

fn make_i4_variant(val: i32) -> VARIANT {
    unsafe {
        let mut v = VARIANT::default();
        let inner = &mut *v.Anonymous.Anonymous;
        ptr::write(&raw mut inner.vt, VT_I4);
        ptr::write(&raw mut inner.Anonymous.lVal, val);
        v
    }
}
