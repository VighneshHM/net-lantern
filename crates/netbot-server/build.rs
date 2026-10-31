//! Re-embeds the UI when `ui/dist` appears or changes.
//!
//! `UiAssets` embeds `../../ui/dist` (see `src/assets.rs`), but cargo only
//! knows about files rust-embed already saw. Without this, an embed compiled
//! before the first `npm run build` is never refreshed: its folder was missing
//! at compile time, so even debug builds, which read the files from disk,
//! reject every lookup and serve the placeholder forever.
//!
//! cargo scans a directory named here recursively for newer files. While the
//! directory does not exist it counts as changed, so the crate is rebuilt
//! each time until the UI is built once.

fn main() {
    println!("cargo:rerun-if-changed=../../ui/dist");
}
