// Exercise the actual vendored decoder on Linux too: the Windows camera backend
// is otherwise target-gated and vendor crates are excluded from the workspace.
#[path = "../../../vendor/nokhwa-bindings-windows/src/frame_rate.rs"]
mod frame_rate;
