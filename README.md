# vyges-buffer-insert

**STA-driven buffer insertion**: a gate-level netlist in, a **buffered netlist** out — buffers
added where a driver's output transition is too slow, scored by a real static-timing engine.

> **Vyges open EDA tools.** The third of the "close-timing" engines, after
> [`vyges-resize`](https://github.com/vyges-tools/resize) (drive strength) and
> [`vyges-vt-swap`](https://github.com/vyges-tools/vt-swap) (threshold voltage). Those swap a
> cell for one with the same footprint; this one **adds** cells. When a net is so heavily loaded
> that its driver's transition (slew) blows past the limit, `vyges-buffer-insert` splits the net:
> a fresh buffer takes over a share of the sinks, so the original driver sees less load and
> switches faster.

## What it does

`vyges-buffer-insert` finds the driver whose output transition most exceeds a `max_slew` limit,
splits its net (the buffer drives half the sinks; the driver keeps the rest), rebuilds the timer
on the mutated netlist, and keeps the insertion if the **worst transition dropped** without
breaking setup. It repeats until every net is under the limit or the effort budget is spent.

```text
  netlist + .lib + constraints ──[ vyges-buffer-insert ]──►  buffered netlist  (+ before/after slew & timing)
```

Each candidate is scored by the [`vyges-sta-si`](https://github.com/vyges-tools/sta-si) timer — it's
**pure Rust**, so you can experiment with GPUs too via [rust-gpu](https://rust-gpu.github.io/). Because inserting a buffer changes the netlist topology
(unlike resize/vt-swap, which only swap a cell), each candidate is a fresh timing build — correct,
and bounded by the effort budget. It is a **pre-place** structural fixup: it decides *where in the
logical net* to split and hands placement of the new buffer back to the flow.

## The job

A `.bufins` file is a superset of a `.sta` timing job, plus the insertion knobs:

```text
design:     top
netlist:    top.v
lib:        tt.lib
spef:       top.spef          # optional — score against real interconnect (post-place)
clock:      clk 1.2
input_slew: 0.02
output_load: 0.01
buffer:     BUF_X2            # the buffer cell to insert (one input, one output)
max_slew:   0.15             # transition limit (ns); drivers above it are relieved
min_fanout: 2                # only split nets with at least this many sinks
effort:     medium           # low | medium | high  (max buffers to insert)
dont_touch: clk_* *scan*     # driver-instance globs to leave alone
```

## Use it

```sh
cargo build --release            # std-only (depends on the open vyges-sta-si timer)

vyges-buffer-insert run   top.bufins -o buffered.v       # buffer -> netlist
vyges-buffer-insert run   top.bufins --json              # before/after slew + timing as JSON
vyges-buffer-insert run   top.bufins --fail-on-violation # exit 3 if still violating (CI gate)
vyges-buffer-insert check top.bufins                     # validate the job
vyges-buffer-insert demo                                 # buffer a built-in example (no files)
# common flags: -o FILE · --json · -q/--quiet · -v/--verbose · -h/--help · -V/--version
```

See [`examples/fanout.bufins`](examples/fanout.bufins) for a runnable example.

## Domain coverage

`vyges-buffer-insert` operates on the **standard-cell digital abstraction** — it splits
over-transition / high-fanout nets in a **gate-level netlist** by inserting **standard-cell
buffers**, each candidate scored by the digital `vyges-sta-si` timer. That makes it a **digital
optimization** engine: it applies wherever a design is built from characterized standard cells
with buffer cells in the Liberty. It does **not** apply to analog / mixed-signal blocks — they
have no gate-level net topology or standard-cell buffers, and no Liberty-arc analogue for the
timer to score. For analog / mixed-signal physical and integrity coverage, reach for the
analog-capable Vyges engines — [`lvs`](https://github.com/vyges-tools/lvs),
[`layout`](https://github.com/vyges-tools/layout), [`em-ir`](https://github.com/vyges-tools/em-ir),
[`thermal`](https://github.com/vyges-tools/thermal), and [`extract`](https://github.com/vyges-tools/extract).

## Status & bounds

v0 relieves over-transition nets by load-splitting with the buffer cell you name; it keeps setup
met (a buffer costs a little delay, absorbed from slack) and reports worst transition + WNS before
and after. The split is a simple halving of the sinks — balanced / recursive / placement-aware
splitting and explicit hold-buffering are future work, as is an incremental timing update for the
topology change (each candidate currently rebuilds). It is **not** a place-and-route tool. Sign-off
is still the golden timer — `vyges-buffer-insert`'s numbers are a fast, license-free guide.
