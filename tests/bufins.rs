//! End-to-end buffer-insertion tests — fully offline (the sta-si timer is pure std).

use vyges_buffer_insert::engine::run_inputs;
use vyges_buffer_insert::job::{glob_match, parse_cfg};
use vyges_sta_si::job::StaJob;
use vyges_sta_si::netlist;

// An inverter whose transition blows up with load, plus a non-inverting BUF to relieve it.
const LIB: &str = r#"
library (d) {
  cell (INV) {
    pin (A) { direction : input; capacitance : 0.0030; }
    pin (Y) { direction : output;
      timing () { related_pin : "A";
        cell_rise (t)       { index_1 ("0.01, 0.10"); index_2 ("0.001, 0.016"); values ( "0.06, 0.30", "0.09, 0.40" ); }
        cell_fall (t)       { index_1 ("0.01, 0.10"); index_2 ("0.001, 0.016"); values ( "0.05, 0.28", "0.08, 0.38" ); }
        rise_transition (t) { index_1 ("0.01, 0.10"); index_2 ("0.001, 0.016"); values ( "0.03, 0.34", "0.04, 0.42" ); }
        fall_transition (t) { index_1 ("0.01, 0.10"); index_2 ("0.001, 0.016"); values ( "0.03, 0.30", "0.04, 0.38" ); } } }
  }
  cell (BUF) {
    pin (A) { direction : input; capacitance : 0.0030; }
    pin (Y) { direction : output;
      timing () { related_pin : "A"; timing_sense : positive_unate;
        cell_rise (t)       { index_1 ("0.01, 0.10"); index_2 ("0.001, 0.016"); values ( "0.06, 0.30", "0.09, 0.40" ); }
        cell_fall (t)       { index_1 ("0.01, 0.10"); index_2 ("0.001, 0.016"); values ( "0.05, 0.28", "0.08, 0.38" ); }
        rise_transition (t) { index_1 ("0.01, 0.10"); index_2 ("0.001, 0.016"); values ( "0.03, 0.34", "0.04, 0.42" ); }
        fall_transition (t) { index_1 ("0.01, 0.10"); index_2 ("0.001, 0.016"); values ( "0.03, 0.30", "0.04, 0.38" ); } } }
  }
}
"#;

// u0 fans out to four sinks (a heavy net `w`); the four sinks drive four outputs.
const NL: &str = "module top ( a, o1, o2, o3, o4 ); input a; output o1, o2, o3, o4; wire w;\n\
                  INV u0 ( .A(a), .Y(w) );\n\
                  INV u1 ( .A(w), .Y(o1) ); INV u2 ( .A(w), .Y(o2) );\n\
                  INV u3 ( .A(w), .Y(o3) ); INV u4 ( .A(w), .Y(o4) );\n\
                  endmodule";

fn sta(period: f64) -> StaJob {
    StaJob::parse(
        &format!("design: t\nnetlist: x\nlib: x\nclock: clk {period}\ninput_slew: 0.02\noutput_load: 0.003\n"),
        "",
    )
    .unwrap()
}

#[test]
fn relieves_an_over_transition_net() {
    let cfg = parse_cfg("buffer: BUF\nmax_slew: 0.18\nmin_fanout: 2\neffort: high\n").unwrap();
    let r = run_inputs(NL, LIB, &sta(2.0), &cfg).unwrap();

    assert!(
        r.before_slew > r.max_slew_limit,
        "the fanout net should start over-limit"
    );
    assert!(!r.inserted.is_empty(), "a buffer should be inserted");
    assert!(
        r.after_slew < r.before_slew,
        "worst transition should drop: {} -> {}",
        r.before_slew,
        r.after_slew
    );
    assert!(
        r.after_wns >= 0.0,
        "timing must stay met (slack absorbs the buffer)"
    );

    // the buffered netlist round-trips, has the BUF, and moved some sinks onto its net.
    let nl2 = netlist::parse(&r.netlist_v).unwrap();
    let buf = nl2
        .insts
        .iter()
        .find(|i| i.cell == "BUF")
        .expect("a BUF was emitted");
    let bufnet = &buf.conns.iter().find(|(p, _)| p == "Y").unwrap().1;
    let moved = nl2
        .insts
        .iter()
        .filter(|i| i.cell == "INV" && i.conns.iter().any(|(p, n)| p == "A" && n == bufnet))
        .count();
    assert!(
        moved >= 1,
        "at least one sink should now be driven by the buffer"
    );
}

#[test]
fn nothing_to_do_under_a_loose_limit() {
    // a generous slew limit -> no net is over it -> no buffers.
    let cfg = parse_cfg("buffer: BUF\nmax_slew: 1.0\nmin_fanout: 2\neffort: high\n").unwrap();
    let r = run_inputs(NL, LIB, &sta(2.0), &cfg).unwrap();
    assert!(r.inserted.is_empty());
    assert_eq!(r.before_slew, r.after_slew);
}

#[test]
fn dont_touch_blocks_insertion() {
    let cfg =
        parse_cfg("buffer: BUF\nmax_slew: 0.18\nmin_fanout: 2\neffort: high\ndont_touch: u0\n")
            .unwrap();
    let r = run_inputs(NL, LIB, &sta(2.0), &cfg).unwrap();
    assert!(
        r.inserted.is_empty(),
        "the only over-limit driver is dont_touch"
    );
}

#[test]
fn high_fanout_threshold_skips_small_nets() {
    // require fanout >= 5; the net has 4 instance sinks, so it is skipped.
    let cfg = parse_cfg("buffer: BUF\nmax_slew: 0.18\nmin_fanout: 5\neffort: high\n").unwrap();
    let r = run_inputs(NL, LIB, &sta(2.0), &cfg).unwrap();
    assert!(r.inserted.is_empty(), "below the fanout threshold");
}

#[test]
fn globs() {
    assert!(glob_match("clk_*", "clk_a"));
    assert!(glob_match("*_reg", "x_reg"));
    assert!(glob_match("*scan*", "u_scan_0"));
    assert!(!glob_match("u1", "u2"));
}
