// Library re-exports for use by test binaries and integration tests.

pub mod acp;
pub mod archive;
pub mod config;
pub mod focus;
pub mod grouping;
pub mod groups;
pub mod log;
pub mod log_search;
pub mod mock;
pub mod models;
pub mod process_info;
pub mod provider;
pub mod search;
pub mod search_eval;
pub mod supervisor;
pub mod testing;
pub mod ui;
pub mod util;
#[cfg(target_os = "windows")]
pub mod wt_tabs;
#[cfg(not(target_os = "windows"))]
pub mod wt_tabs {
    pub fn list_tab_titles() -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
}
