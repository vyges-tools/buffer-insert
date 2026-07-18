//! The buffer-insertion loop. Each round: find the driver whose output transition most
//! exceeds the limit, split its net (a fresh buffer takes over half the instance sinks so the
//! original driver sees less load), rebuild the timer on the mutated netlist, and keep the
//! insertion if the worst transition dropped without worsening setup. Topology changes, so —
//! unlike resize / vt-swap — each candidate is a full rebuild, not an in-place timer mutation.

use std::collections::HashSet;

use vyges_sta_si::job::StaJob;
use vyges_sta_si::liberty::{Dir, Lib};
use vyges_sta_si::netlist::{self, Inst, Netlist};
use vyges_sta_si::spef::Spef;
use vyges_sta_si::sta::Timer;

use crate::emit;
use crate::job::{glob_match, BufCfg, BufJob};

/// Outcome of a buffer-insertion run.
#[derive(Debug, Clone)]
pub struct BufResult {
    pub before_wns: f64,
    pub after_wns: f64,
    /// worst driver output transition (ns) before / after.
    pub before_slew: f64,
    pub after_slew: f64,
    pub max_slew_limit: f64,
    /// `(buffer_instance, net_it_relieved)` for every inserted buffer, in order.
    pub inserted: Vec<(String, String)>,
    /// The buffered netlist as structural Verilog.
    pub netlist_v: String,
    /// Whether timing was scored against real interconnect parasitics (a SPEF was supplied).
    pub eco: bool,
}

/// A net whose driver is over the transition limit — a candidate to relieve.
struct Cand {
    net: String,
    sinks: Vec<(String, String)>, // (instance, input pin) on the net
    slew: f64,
}

/// The (single input, single output) pin names of the buffer cell.
fn buffer_pins(lib: &Lib, buf: &str) -> Result<(String, String), String> {
    let cell = lib
        .cell(buf)
        .ok_or_else(|| format!("buffer cell {buf:?} not in any .lib"))?;
    let inp = cell
        .pins
        .values()
        .find(|p| p.direction == Dir::In && !p.clock)
        .ok_or_else(|| format!("buffer {buf:?} has no input pin"))?;
    let out = cell
        .outputs()
        .next()
        .ok_or_else(|| format!("buffer {buf:?} has no output pin"))?;
    Ok((inp.name.clone(), out.name.clone()))
}

/// Worst driver output transition over the design (the figure of merit we drive down).
fn worst_slew(timer: &Timer, nl: &Netlist, lib: &Lib) -> f64 {
    let mut worst: f64 = 0.0;
    for inst in &nl.insts {
        let Some(cell) = lib.cell(&inst.cell) else {
            continue;
        };
        for (pin, _net) in &inst.conns {
            if cell.pins.get(pin).map(|p| p.direction) == Some(Dir::Out) {
                if let Some(id) = timer.pin(&format!("{}/{}", inst.name, pin)) {
                    worst = worst.max(timer.slew(id));
                }
            }
        }
    }
    worst
}

/// The over-limit net with the worst driver transition (≥ `min_fanout` instance sinks, driver
/// not `dont_touch`, not already tried), and the sinks on it.
fn worst_overslew(
    timer: &Timer,
    nl: &Netlist,
    lib: &Lib,
    cfg: &BufCfg,
    tried: &HashSet<String>,
) -> Option<Cand> {
    let mut best: Option<Cand> = None;
    for inst in &nl.insts {
        if cfg.dont_touch.iter().any(|p| glob_match(p, &inst.name)) {
            continue;
        }
        let Some(cell) = lib.cell(&inst.cell) else {
            continue;
        };
        for (pin, net) in &inst.conns {
            if cell.pins.get(pin).map(|p| p.direction) != Some(Dir::Out) {
                continue;
            }
            if tried.contains(net) {
                continue;
            }
            let Some(id) = timer.pin(&format!("{}/{}", inst.name, pin)) else {
                continue;
            };
            let slew = timer.slew(id);
            if slew <= cfg.max_slew {
                continue;
            }
            let sinks = sinks_of(nl, lib, net);
            if sinks.len() < cfg.min_fanout {
                continue;
            }
            if best.as_ref().map(|b| slew > b.slew).unwrap_or(true) {
                best = Some(Cand {
                    net: net.clone(),
                    sinks,
                    slew,
                });
            }
        }
    }
    best
}

/// Instance input pins (instance, pin) connected to `net`.
fn sinks_of(nl: &Netlist, lib: &Lib, net: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    for inst in &nl.insts {
        let Some(cell) = lib.cell(&inst.cell) else {
            continue;
        };
        for (pin, n) in &inst.conns {
            if n == net && cell.pins.get(pin).map(|p| p.direction) == Some(Dir::In) {
                v.push((inst.name.clone(), pin.clone()));
            }
        }
    }
    // deterministic split order
    v.sort();
    v
}

/// Insert a buffer on `cand.net`: a new buffer instance drives a fresh net carrying the first
/// half of the sinks; the buffer's input stays on the original net. The original driver thus
/// sees fewer sinks (less load) and switches faster.
fn split_net(
    nl: &mut Netlist,
    cand: &Cand,
    cfg: &BufCfg,
    bin: &str,
    bout: &str,
    bufname: &str,
    bufnet: &str,
) {
    let take = (cand.sinks.len() / 2).max(1); // move at least one, leave at least one
    let move_set: HashSet<&(String, String)> = cand.sinks.iter().take(take).collect();
    for inst in &mut nl.insts {
        for (pin, n) in &mut inst.conns {
            if n == &cand.net && move_set.contains(&(inst.name.clone(), pin.clone())) {
                *n = bufnet.to_string();
            }
        }
    }
    nl.insts.push(Inst {
        cell: cfg.buffer.clone(),
        name: bufname.to_string(),
        conns: vec![
            (bin.to_string(), cand.net.clone()),
            (bout.to_string(), bufnet.to_string()),
        ],
    });
}

/// Run a buffer-insertion job loaded from disk.
pub fn run(job: &BufJob) -> Result<BufResult, String> {
    let sta = &job.sta;
    let nl = netlist::load(&sta.resolve(&sta.netlist)).map_err(|e| e.to_string())?;
    let mut lib = Lib::default();
    for l in &sta.libs {
        let one = Lib::load(&sta.resolve(l)).map_err(|e| e.to_string())?;
        lib.cells.extend(one.cells);
    }
    if lib.cells.is_empty() {
        return Err("no cells in any .lib".into());
    }
    let spef = match &sta.spef {
        Some(p) => Some(Spef::load(&sta.resolve(p)).map_err(|e| e.to_string())?),
        None => None,
    };
    optimize(nl, &lib, sta, spef, &job.cfg)
}

/// Run on already-parsed inputs (the `demo` path; ideal interconnect, no SPEF).
pub fn run_inputs(
    nl_text: &str,
    lib_text: &str,
    sta: &StaJob,
    cfg: &BufCfg,
) -> Result<BufResult, String> {
    let nl = netlist::parse(nl_text).map_err(|e| e.to_string())?;
    let lib = Lib::parse(lib_text).map_err(|e| e.to_string())?;
    optimize(nl, &lib, sta, None, cfg)
}

/// The optimizer: greedily relieve over-transition nets, rebuilding the timer per candidate.
pub fn optimize(
    mut nl: Netlist,
    lib: &Lib,
    sta: &StaJob,
    spef: Option<Spef>,
    cfg: &BufCfg,
) -> Result<BufResult, String> {
    let (bin, bout) = buffer_pins(lib, &cfg.buffer)?;
    let build = |nl: &Netlist| Timer::build(nl, lib, sta, spef.as_ref()).map_err(|e| e.to_string());

    let mut timer = build(&nl)?;
    let before_wns = timer.wns();
    let before_slew = worst_slew(&timer, &nl, lib);

    let mut inserted: Vec<(String, String)> = Vec::new();
    let mut tried: HashSet<String> = HashSet::new();
    let mut counter = 0usize;

    for _ in 0..cfg.effort {
        let cur_slew = worst_slew(&timer, &nl, lib);
        let Some(cand) = worst_overslew(&timer, &nl, lib, cfg, &tried) else {
            break;
        };

        let mut trial = nl.clone();
        let bufname = format!("__buf_{counter}");
        let bufnet = format!("__bufn_{counter}");
        split_net(&mut trial, &cand, cfg, &bin, &bout, &bufname, &bufnet);
        let ttimer = build(&trial)?;

        // accept iff the worst transition dropped and setup is still acceptable: a buffer adds
        // a little delay, so on a met design we let it consume slack as long as timing stays
        // met; on an already-violating design we require it not get worse.
        let timing_ok = if timer.wns() >= 0.0 {
            ttimer.wns() >= -1e-9
        } else {
            ttimer.wns() >= timer.wns() - 1e-9
        };
        if worst_slew(&ttimer, &trial, lib) < cur_slew - 1e-9 && timing_ok {
            inserted.push((bufname, cand.net.clone()));
            nl = trial;
            timer = ttimer;
            tried.clear(); // slews changed everywhere; re-evaluate
            counter += 1;
        } else {
            tried.insert(cand.net.clone()); // this net can't be helped — move on
        }
    }

    Ok(BufResult {
        before_wns,
        after_wns: timer.wns(),
        before_slew,
        after_slew: worst_slew(&timer, &nl, lib),
        max_slew_limit: cfg.max_slew,
        inserted,
        netlist_v: emit::to_verilog(&nl),
        eco: spef.is_some(),
    })
}
