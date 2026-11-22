use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;
use wordrect::dict::{self, Dict};
use wordrect::par::ParSearch;
use wordrect::{SOLVERS, Solver, make_solver};
use wordrect::validate::validate;

const USAGE: &str = "\
wordrect — word rectangle solver

USAGE:
  wordrect find  --rows R --cols C [options]   find one R×C rectangle
  wordrect count --rows R --cols C [options]   count all R×C rectangles (exhaustive)
  wordrect squares --rows N [options]          count symmetric N×N word squares (row i = column i)
  wordrect info  [options]                     dictionary stats (words per length)

OPTIONS:
  --dict PATH      dictionary file          [default: dicts/enable.txt]
  --solver NAME    solver to use            [default: par]   (available: par, trie, naive)
  --threads N      threads for the par solver [default: all cores]
  --stats          print nodes visited, nodes/sec, time
  --print          (count) print every solution found
  -h, --help       show this help
";

struct Args {
    cmd: String,
    dict: PathBuf,
    rows: usize,
    cols: usize,
    solver: String,
    stats: bool,
    print: bool,
    threads: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or("missing command")?;
    if cmd == "-h" || cmd == "--help" {
        return Err(String::new());
    }
    let mut a = Args {
        cmd,
        dict: "dicts/enable.txt".into(),
        rows: 0,
        cols: 0,
        solver: SOLVERS[0].into(),
        stats: false,
        print: false,
        threads: 0,
    };
    while let Some(flag) = it.next() {
        let mut val = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--dict" => a.dict = val()?.into(),
            "--rows" | "-r" => a.rows = val()?.parse().map_err(|e| format!("--rows: {e}"))?,
            "--cols" | "-c" => a.cols = val()?.parse().map_err(|e| format!("--cols: {e}"))?,
            "--solver" => a.solver = val()?,
            "--threads" => a.threads = val()?.parse().map_err(|e| format!("--threads: {e}"))?,
            "--stats" => a.stats = true,
            "--print" => a.print = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

fn print_grid(g: &[String]) {
    for row in g {
        let spaced: Vec<String> = row.chars().map(|c| c.to_ascii_uppercase().to_string()).collect();
        println!("  {}", spaced.join(" "));
    }
}

fn run(a: Args) -> Result<(), String> {
    let t_load = Instant::now();
    let d = Dict::load(&a.dict).map_err(|e| format!("cannot read {}: {e}", a.dict.display()))?;
    let fp = dict::fingerprint(&a.dict).map_err(|e| e.to_string())?;
    let load_time = t_load.elapsed();

    println!(
        "# dict={} words={} fnv={} | ruleset=rectangle,repeats-ok,a-z | solver={} | commit={}",
        a.dict.display(),
        d.total,
        fp,
        a.solver,
        env!("GIT_COMMIT")
    );

    if a.cmd == "info" {
        println!("{:>4}  {:>7}", "len", "words");
        for (n, ws) in d.by_len.iter().enumerate().filter(|(_, w)| !w.is_empty()) {
            println!("{n:>4}  {:>7}", ws.len());
        }
        return Ok(());
    }
    if a.cmd == "squares" {
        if a.rows == 0 {
            return Err("--rows is required".into());
        }
        let ws = d.words(a.rows);
        println!("# size={0}x{0} symmetric word squares  words (len {0})={1}", a.rows, ws.len());
        let t = Instant::now();
        let (n, nodes) = wordrect::squares::Squares::new(a.rows, ws).count(a.threads);
        println!("squares={n}");
        if a.stats {
            let secs = t.elapsed().as_secs_f64();
            println!("# nodes={nodes}  time={secs:.6}s  nodes/sec={:.0}  (dict load {:.3}s)", nodes as f64 / secs.max(1e-9), load_time.as_secs_f64());
        }
        return Ok(());
    }
    if a.cmd != "find" && a.cmd != "count" {
        return Err(format!("unknown command {}", a.cmd));
    }
    if a.rows == 0 || a.cols == 0 {
        return Err("--rows and --cols are required".into());
    }

    let (rw, cw) = (d.words(a.cols), d.words(a.rows));
    println!("# size={}x{}  row words (len {})={}  col words (len {})={}", a.rows, a.cols, a.cols, rw.len(), a.rows, cw.len());

    let all: HashSet<String> = d.by_len.iter().flatten().cloned().collect();
    let mut solver: Box<dyn Solver> = if a.solver == "par" && a.threads > 0 {
        Box::new(ParSearch::new(a.rows, a.cols, rw, cw, a.threads))
    } else {
        make_solver(&a.solver, a.rows, a.cols, rw, cw)
            .ok_or_else(|| format!("unknown solver {} (available: {})", a.solver, SOLVERS.join(", ")))?
    };
    let t = Instant::now();

    if a.cmd == "find" {
        match solver.find() {
            Some(g) => {
                validate(&g, &all).map_err(|e| format!("VALIDATION FAILED: {e}"))?;
                println!("found (validated):");
                print_grid(&g);
            }
            None => println!("no solution: {}x{} exhausted", a.rows, a.cols),
        }
    } else if a.print {
        let sols = solver.all();
        for g in &sols {
            validate(g, &all).map_err(|e| format!("VALIDATION FAILED: {e}"))?;
            println!("{}", g.join("/"));
        }
        println!("solutions={} (all validated)", sols.len());
    } else {
        println!("solutions={}", solver.count());
    }

    let el = t.elapsed();
    if a.stats {
        let s = solver.stats();
        let secs = el.as_secs_f64();
        println!(
            "# nodes={}  time={:.6}s  nodes/sec={:.0}  (dict load {:.3}s)",
            s.nodes,
            secs,
            s.nodes as f64 / secs.max(1e-9),
            load_time.as_secs_f64()
        );
        if wordrect::probe::ENABLED {
            for e in wordrect::probe::events() {
                println!("# ev {} {} {:.1} {:.1} {}", e.lane, wordrect::probe::KINDS[e.kind as usize], e.start_us, e.end_us, e.rank);
            }
        }
        if wordrect::probe::ENABLED {
            // per-cell node counts for the heatmap; row "L" = the last-row kernel
            let n = wordrect::probe::N;
            for (i, v) in wordrect::probe::take().iter().enumerate().filter(|(_, v)| **v > 0) {
                let r = if i / n == n - 1 { "L".to_string() } else { (i / n).to_string() };
                println!("# cell {r} {} {v}", i % n);
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match parse_args().and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is_empty() => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
