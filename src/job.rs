//! The `.bufins` job: the timing setup (reused from `vyges-sta-si`'s job parser) plus the
//! buffer-insertion knobs. A `.bufins` file is a superset of a `.sta` file — same
//! `design`/`netlist`/`lib`/`clock`/`spef`/… keys (read by [`StaJob`]) and adds:
//!
//! ```text
//! buffer:     BUF_X2          # the buffer cell to insert (one input, one output)
//! max_slew:   0.15            # transition limit (ns); nets whose driver exceeds it are split
//! min_fanout: 2               # only split nets with at least this many sinks (default 2)
//! effort:     medium          # low | medium | high  (max buffers to insert)
//! dont_touch: clk_* *scan*    # driver-instance globs whose nets are left alone
//! ```

use vyges_sta_si::job::StaJob;

/// The buffer-insertion configuration (everything beyond the timing setup).
#[derive(Debug, Clone)]
pub struct BufCfg {
    /// The buffer cell to insert (must have exactly one input and one output pin).
    pub buffer: String,
    /// Transition (slew) limit in ns; a driver pin above it is a candidate to relieve.
    pub max_slew: f64,
    /// Only split nets with at least this many instance sinks.
    pub min_fanout: usize,
    /// Max buffers to insert (derived from `effort:`).
    pub effort: usize,
    /// Driver-instance globs (leading/trailing `*`) whose nets are never touched.
    pub dont_touch: Vec<String>,
}

/// A loaded buffer-insertion job: the timing job + the config.
#[derive(Debug, Clone)]
pub struct BufJob {
    pub sta: StaJob,
    pub cfg: BufCfg,
}

impl BufJob {
    pub fn load(path: &str) -> Result<BufJob, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let sta = StaJob::load(path).map_err(|e| e.to_string())?;
        let cfg = parse_cfg(&text)?;
        Ok(BufJob { sta, cfg })
    }
}

/// Parse the buffer-insertion keys out of the job text.
pub fn parse_cfg(text: &str) -> Result<BufCfg, String> {
    let mut buffer = String::new();
    let mut max_slew = 0.15;
    let mut min_fanout = 2usize;
    let mut effort_word = "medium".to_string();
    let mut dont_touch = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_lowercase(), v.trim());
        match k.as_str() {
            "buffer" => buffer = v.to_string(),
            "max_slew" => {
                max_slew = v
                    .parse()
                    .map_err(|_| format!("max_slew must be a number, got {v:?}"))?
            }
            "min_fanout" => {
                min_fanout = v
                    .parse()
                    .map_err(|_| format!("min_fanout must be an integer, got {v:?}"))?
            }
            "effort" => effort_word = v.to_lowercase(),
            "dont_touch" => {
                dont_touch.extend(
                    v.split([',', ' '])
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                );
            }
            _ => {}
        }
    }
    if buffer.is_empty() {
        return Err("a `buffer:` cell is required".into());
    }
    let effort = match effort_word.as_str() {
        "low" => 20,
        "medium" => 100,
        "high" => 500,
        other => return Err(format!("effort must be low|medium|high, got {other:?}")),
    };
    Ok(BufCfg {
        buffer,
        max_slew,
        min_fanout,
        effort,
        dont_touch,
    })
}

/// A tiny glob matcher: supports a single leading and/or trailing `*` (e.g. `clk_*`,
/// `*scan*`, `*_reg`). Exact match otherwise.
pub fn glob_match(pat: &str, s: &str) -> bool {
    match (pat.strip_prefix('*'), pat.strip_suffix('*')) {
        (Some(_), Some(_)) => s.contains(pat.trim_matches('*')),
        (Some(suf), None) => s.ends_with(suf),
        (None, Some(pre)) => s.starts_with(pre),
        (None, None) => s == pat,
    }
}
