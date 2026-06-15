//! vyges-buffer-insert CLI.
//!
//!   vyges-buffer-insert run   JOB  [-o OUT] [--json] [--fail-on-violation]
//!   vyges-buffer-insert check JOB
//!   vyges-buffer-insert demo
//!
//! Common flags: -h/--help, -V/--version, -q/--quiet, -v/--verbose.
//! Exit codes: 0 ok · 1 runtime error · 2 usage/validation · 3 still-violating (--fail-on-violation).

use std::process::exit;

use vyges_buffer_insert::engine::{self, BufResult};
use vyges_buffer_insert::job::{parse_cfg, BufJob};
use vyges_sta_si::job::StaJob;

const USAGE: &str = "\
vyges-buffer-insert — STA-driven buffer insertion (split over-transition / high-fanout nets)

usage:
  vyges-buffer-insert run   JOB  [-o OUT] [--json] [--fail-on-violation]   buffer -> resized netlist
  vyges-buffer-insert check JOB                                            validate the job
  vyges-buffer-insert demo                                                 buffer a built-in example (no files)

flags:
  -o FILE              write the buffered netlist to FILE (default: stdout)
  --json               emit the before/after report as JSON
  --fail-on-violation  exit 3 if the result still has negative setup slack (CI gate)
  -q, --quiet          suppress non-essential output
  -v, --verbose        extra detail on stderr
  -h, --help           show this help
  -V, --version        show version
  --bug-report         file a bug (central: vyges/community)
  --feature-request    request a feature (central)
  --sponsor            sponsor Vyges (github.com/sponsors/vyges-ip)
  --star               star this tool on GitHub ⭐
";

const BUG_URL: &str = "https://github.com/vyges/community/issues/new?template=bug_report_template.yaml";
const FEATURE_URL: &str = "https://github.com/vyges/community/issues/new?labels=enhancement";
const SPONSOR_URL: &str = "https://github.com/sponsors/vyges-ip";
const STAR_URL: &str = "https://github.com/vyges-tools/buffer-insert";

fn link(label: &str, url: &str) {
    use std::io::IsTerminal;
    println!("{label}:\n  {url}");
    if std::io::stdout().is_terminal() {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(opener).arg(url).status();
    }
}

#[derive(Default)]
struct Cli {
    positionals: Vec<String>,
    out: Option<String>,
    json: bool,
    fail_on_violation: bool,
    quiet: bool,
    verbose: bool,
    help: bool,
    version: bool,
    bug_report: bool,
    feature_request: bool,
    sponsor: bool,
    star: bool,
}

fn parse_cli(args: &[String]) -> Cli {
    let mut c = Cli::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                c.out = args.get(i + 1).cloned();
                i += 1;
            }
            "--json" => c.json = true,
            "--fail-on-violation" => c.fail_on_violation = true,
            "-q" | "--quiet" => c.quiet = true,
            "-v" | "--verbose" => c.verbose = true,
            "-h" | "--help" => c.help = true,
            "-V" | "--version" => c.version = true,
            "--bug-report" => c.bug_report = true,
            "--feature-request" => c.feature_request = true,
            "--sponsor" => c.sponsor = true,
            "--star" => c.star = true,
            other => c.positionals.push(other.to_string()),
        }
        i += 1;
    }
    c
}

fn render_report(r: &BufResult) -> String {
    let met = |w: f64| if w >= 0.0 { "MET" } else { "VIOLATED" };
    let mut s = String::new();
    s.push_str("vyges-buffer-insert — buffer insertion\n");
    s.push_str(&format!(
        "  mode:    {}\n",
        if r.eco { "post-place ECO (SPEF interconnect)" } else { "pre-place (ideal interconnect)" }
    ));
    s.push_str(&format!(
        "  slew:    worst {:.4} -> {:.4} ns  (limit {:.4})\n",
        r.before_slew, r.after_slew, r.max_slew_limit
    ));
    s.push_str(&format!("  setup:   WNS {:.4} -> {:.4} ns [{}]\n", r.before_wns, r.after_wns, met(r.after_wns)));
    s.push_str(&format!("  buffers: {} inserted\n", r.inserted.len()));
    for (buf, net) in &r.inserted {
        s.push_str(&format!("    {buf} relieves net {net}\n"));
    }
    s
}

fn report_json(r: &BufResult) -> String {
    let ins: Vec<String> = r
        .inserted
        .iter()
        .map(|(b, n)| format!("{{\"buffer\":\"{b}\",\"net\":\"{n}\"}}"))
        .collect();
    format!(
        "{{\"eco\":{},\"before_wns\":{},\"after_wns\":{},\"before_slew\":{},\"after_slew\":{},\"max_slew\":{},\"inserted\":[{}]}}",
        r.eco, r.before_wns, r.after_wns, r.before_slew, r.after_slew, r.max_slew_limit, ins.join(",")
    )
}

// ---- built-in demo: one inverter fanning out to four sinks (a heavy net whose transition is
// over the limit); a BUF takes over half the load. ----
const DEMO_NL: &str = "module top ( a, o1, o2, o3, o4 ); input a; output o1, o2, o3, o4; wire w;\n\
                       INV u0 ( .A(a), .Y(w) );\n\
                       INV u1 ( .A(w), .Y(o1) ); INV u2 ( .A(w), .Y(o2) );\n\
                       INV u3 ( .A(w), .Y(o3) ); INV u4 ( .A(w), .Y(o4) );\n\
                       endmodule";
const DEMO_LIB: &str = r#"
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
const DEMO_JOB: &str = "design: demo\nnetlist: x\nlib: x\nclock: clk 2.0\ninput_slew: 0.02\noutput_load: 0.003\n";

fn run_demo() -> Result<BufResult, String> {
    let sta = StaJob::parse(DEMO_JOB, "").map_err(|e| e.to_string())?;
    let cfg = parse_cfg("buffer: BUF\nmax_slew: 0.18\nmin_fanout: 2\neffort: medium\n")?;
    engine::run_inputs(DEMO_NL, DEMO_LIB, &sta, &cfg)
}

fn write_netlist(text: &str, out: &Option<String>, quiet: bool) {
    match out {
        Some(path) => match std::fs::write(path, text) {
            Ok(_) => {
                if !quiet {
                    eprintln!("wrote {path}");
                }
            }
            Err(e) => {
                eprintln!("error: {path}: {e}");
                exit(1);
            }
        },
        None => print!("{text}"),
    }
}

fn finish(r: BufResult, cli: &Cli) {
    if cli.json {
        println!("{}", report_json(&r));
        if cli.out.is_some() {
            write_netlist(&r.netlist_v, &cli.out, cli.quiet);
        }
    } else {
        write_netlist(&r.netlist_v, &cli.out, cli.quiet);
        if !cli.quiet {
            eprint!("{}", render_report(&r));
        }
    }
    if cli.fail_on_violation && r.after_wns < 0.0 {
        exit(3);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = parse_cli(&args);

    if cli.bug_report {
        return link("Report a bug (central — vyges/community)", BUG_URL);
    }
    if cli.feature_request {
        return link("Request a feature (central — vyges/community)", FEATURE_URL);
    }
    if cli.sponsor {
        return link("Sponsor Vyges", SPONSOR_URL);
    }
    if cli.star {
        return link("Star vyges-buffer-insert on GitHub ⭐", STAR_URL);
    }
    if cli.version {
        println!("vyges-buffer-insert {} ({})", vyges_buffer_insert::VERSION, env!("VYGES_GIT_SHA"));
        println!("{}", vyges_buffer_insert::COPYRIGHT);
        return;
    }
    let cmd = cli.positionals.first().cloned().unwrap_or_default();
    if cli.help || cmd.is_empty() {
        print!("{USAGE}");
        exit(if cmd.is_empty() && !cli.help { 2 } else { 0 });
    }

    match cmd.as_str() {
        "demo" => match run_demo() {
            Ok(r) => finish(r, &cli),
            Err(e) => {
                eprintln!("error: {e}");
                exit(1);
            }
        },
        "check" => {
            let Some(path) = cli.positionals.get(1) else {
                eprintln!("usage: vyges-buffer-insert check JOB");
                exit(2);
            };
            match BufJob::load(path) {
                Ok(j) => println!(
                    "OK  design={} buffer={} max_slew={} min_fanout={} effort={} dont_touch={}",
                    j.sta.design,
                    j.cfg.buffer,
                    j.cfg.max_slew,
                    j.cfg.min_fanout,
                    j.cfg.effort,
                    j.cfg.dont_touch.len()
                ),
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(2);
                }
            }
        }
        "run" => {
            let Some(path) = cli.positionals.get(1) else {
                eprintln!("usage: vyges-buffer-insert run JOB [-o OUT]");
                exit(2);
            };
            let job = match BufJob::load(path) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(2);
                }
            };
            if cli.verbose {
                eprintln!("buffering {} (buffer {}, max_slew {})", job.sta.design, job.cfg.buffer, job.cfg.max_slew);
            }
            match engine::run(&job) {
                Ok(r) => finish(r, &cli),
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(1);
                }
            }
        }
        other => {
            eprintln!("vyges-buffer-insert: unknown command {other:?}\n");
            print!("{USAGE}");
            exit(2);
        }
    }
}
