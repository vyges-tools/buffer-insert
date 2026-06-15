//! vyges-buffer-insert — STA-driven buffer insertion.
//!
//! The third of the Vyges "close-timing" engines, after `vyges-resize` (drive strength) and
//! `vyges-vt-swap` (threshold voltage). Those swap a cell for another with the same footprint;
//! this one **adds cells** — it splits a net that is too heavily loaded (its driver's output
//! transition exceeds a limit) by inserting a buffer that takes over a share of the sinks, so
//! the original driver sees less load and switches faster.
//!
//! Because it changes the netlist topology it cannot ride the timer's in-place cell-swap
//! mutation; each candidate insertion is scored by rebuilding the timer on the mutated netlist
//! (correct; the incremental topology update is future work). It is a **pre-place** structural
//! fixup — it decides *where in the logical net* to split, and hands placement of the new
//! buffer back to the flow. Inputs/outputs are files: a `.bufins` job + Liberty in, a buffered
//! netlist + a before/after transition & timing report out. Pure std, unit-tested offline.

pub mod emit;
pub mod engine;
pub mod job;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COPYRIGHT: &str = "© 2026 Vyges. All Rights Reserved.  https://vyges.com";
