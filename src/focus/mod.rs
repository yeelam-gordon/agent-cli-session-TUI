//! Native Windows Terminal tab focus via UI Automation.
//!
//! Uses the `windows` crate for COM-based UI Automation — no PowerShell overhead.
//! Finds WT tabs by substring match on tab name, selects the tab, and brings
//! the window to foreground (preserving maximized state).

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::focus_wt_tab;

/// Attempt to focus a Windows Terminal tab whose name contains `search`.
///
/// Returns `true` if a matching tab was found and focused.
/// On non-Windows platforms, always returns `false`.
#[cfg(not(windows))]
pub fn focus_wt_tab(_search: &str) -> bool {
    false
}
