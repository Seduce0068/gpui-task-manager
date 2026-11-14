//! sysinfo 0.40.3 の実測プローブ（設計判断用 Runtime Proof）
//!
//! 目的:
//! 1. 1 サンプル取得（CPU/メモリ/プロセス + ネットワーク + ディスク）の所要時間を release ビルドで測る
//! 2. タスクマネージャーに必要な項目が実際に取得できることを確認する
//!    （per-core CPU, メモリ/スワップ, ネットワーク, ディスク,
//!     プロセスの cmdline/user/status/start_time, kill 機能）
//! 3. PID 再利用対策に使える start_time の型・値域を確認する
//! 4. スナップショットを serde_json でシリアライズし、ローカル agent の wire format 実現性を確認する
//!
//! 実行: cargo run --release
//! 注意: kill の実証は「自分が spawn した sleep 30」だけを対象にする（他プロセスは触らない）。

use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{Disks, Networks, Pid, ProcessesToUpdate, Signal, System, Users};

const SYSINFO_CRATE_VERSION: &str = "0.40.3";

#[derive(Serialize)]
struct NetRow {
    name: String,
    total_received: u64,
    total_transmitted: u64,
    received_since_last: u64,
    transmitted_since_last: u64,
    packets_received: u64,
    packets_transmitted: u64,
    errors_on_received: u64,
}

#[derive(Serialize)]
struct DiskRow {
    name: String,
    mount_point: String,
    file_system: String,
    kind: String,
    total_space: u64,
    available_space: u64,
    is_removable: bool,
    is_read_only: bool,
}

#[derive(Serialize)]
struct ProcRow {
    pid: u32,
    parent: Option<u32>,
    name: String,
    user: Option<String>,
    status: String,
    start_time_since_boot_secs: u64,
    run_time_secs: u64,
    cpu_percent: f32,
    memory_bytes: u64,
    virtual_memory_bytes: u64,
    cmdline: String,
    exe: Option<String>,
    cwd: Option<String>,
    disk_read_bytes: u64,
    disk_written_bytes: u64,
}

#[derive(Serialize)]
struct Snapshot {
    schema: &'static str,
    captured_at_unix: u64,
    sysinfo_crate_version: &'static str,
    host_name: Option<String>,
    os_name: Option<String>,
    os_version: Option<String>,
    kernel_version: Option<String>,
    uptime_secs: Option<u64>,
    boot_time_unix: Option<u64>,
    load_avg: Option<[f64; 3]>,
    physical_cores: Option<usize>,
    logical_cpus: usize,
    cpu_brand: String,
    cpu_frequency_mhz: u64,
    global_cpu_percent: f32,
    per_core_cpu_percent: Vec<f32>,
    memory_total: u64,
    memory_used: u64,
    memory_available: u64,
    swap_total: u64,
    swap_used: u64,
    networks: Vec<NetRow>,
    disks: Vec<DiskRow>,
    process_count: usize,
    processes: Vec<ProcRow>,
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    println!("== sysinfo probe: crate version {SYSINFO_CRATE_VERSION} ==");

    // ---- 1. コールドスタート（System::new_all の初回構築）----
    let t = Instant::now();
    let mut sys = System::new_all().expect("System::new_all failed");
    let cold = t.elapsed();
    println!("[timing] System::new_all() cold start: {cold:?}");

    // CPU 使用率は 2 回目以降の refresh で意味を持つ（MINIMUM_CPU_UPDATE_INTERVAL=200ms）
    std::thread::sleep(Duration::from_millis(250));
    sys.refresh_all();

    // ---- 2. 定常状態のサンプル取得コスト（median of 7）----
    let mut full_ticks: Vec<Duration> = Vec::new();
    let mut targeted_ticks: Vec<Duration> = Vec::new();

    let t = Instant::now();
    let mut disks = Disks::new_with_refreshed_list().expect("Disks failed");
    let disks_init = t.elapsed();
    let t = Instant::now();
    let mut networks = Networks::new_with_refreshed_list().expect("Networks failed");
    let networks_init = t.elapsed();
    let users = Users::new_with_refreshed_list().expect("Users failed");
    println!("[timing] Disks::new_with_refreshed_list(): {disks_init:?}");
    println!("[timing] Networks::new_with_refreshed_list(): {networks_init:?}");

    // 現実的な 1 tick = refresh_all + disks.refresh + networks.refresh を 7 回測る
    let mut disk_ticks = Vec::new();
    let mut net_ticks = Vec::new();
    for _ in 0..7 {
        std::thread::sleep(Duration::from_millis(250));
        let t = Instant::now();
        sys.refresh_all();
        let d = t.elapsed();
        let t = Instant::now();
        disks.refresh(true);
        let d2 = t.elapsed();
        let t = Instant::now();
        networks.refresh(true);
        let d3 = t.elapsed();
        full_ticks.push(d + d2 + d3);
        disk_ticks.push(d2);
        net_ticks.push(d3);
    }

    // 対象を絞った tick（CPU + メモリ + プロセスのみ）も測る
    for _ in 0..7 {
        std::thread::sleep(Duration::from_millis(250));
        let t = Instant::now();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        targeted_ticks.push(t.elapsed());
    }

    println!(
        "[timing] full tick  (refresh_all + disks + networks): median={:?} min={:?} max={:?} n=7",
        median(full_ticks.clone()),
        full_ticks.iter().min().unwrap(),
        full_ticks.iter().max().unwrap()
    );
    println!(
        "[timing] targeted tick (cpu+memory+processes only):   median={:?} min={:?} max={:?} n=7",
        median(targeted_ticks.clone()),
        targeted_ticks.iter().min().unwrap(),
        targeted_ticks.iter().max().unwrap()
    );
    println!(
        "[timing] disks.refresh only: median={:?} / networks.refresh only: median={:?}",
        median(disk_ticks),
        median(net_ticks)
    );

    // ---- 3. 取得項目のダンプ ----
    println!("\n== system ==");
    println!(
        "host={:?} os={:?} os_version={:?} kernel={:?}",
        System::host_name(),
        System::name(),
        System::os_version(),
        System::kernel_version()
    );
    println!(
        "uptime={:?} boot_time_unix={:?} physical_cores={:?} logical_cpus={}",
        System::uptime(),
        System::boot_time(),
        System::physical_core_count(),
        sys.cpus().len()
    );
    match System::load_average() {
        Ok(la) => println!("load_avg 1/5/15 = {:.2}/{:.2}/{:.2}", la.one, la.five, la.fifteen),
        Err(e) => println!("load_avg error: {e:?}"),
    }
    let cpu_brand = sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default();
    let cpu_frequency_mhz = sys.cpus().first().map(|c| c.frequency()).unwrap_or(0);
    println!("cpu_brand={cpu_brand:?} cpu_frequency_mhz={cpu_frequency_mhz:?}");
    println!("global_cpu_percent={:.2}", sys.global_cpu_usage());
    print!("per_core_cpu_percent ({} cores) = [", sys.cpus().len());
    for (i, c) in sys.cpus().iter().enumerate() {
        if i > 0 {
            print!(", ");
        }
        print!("{:.1}", c.usage());
    }
    println!("]");

    println!("\n== memory ==");
    println!(
        "total={} used={} available={} free={} swap_total={} swap_used={}",
        sys.total_memory(),
        sys.used_memory(),
        sys.available_memory(),
        sys.free_memory(),
        sys.total_swap(),
        sys.used_swap()
    );

    println!("\n== networks ==");
    for (name, data) in networks.iter() {
        println!(
            "if={} total_rx={} total_tx={} rx_since_last={} tx_since_last={} pkts_rx={} pkts_tx={} err_rx={} mac={} mtu={}",
            name.to_string_lossy(),
            data.total_received(),
            data.total_transmitted(),
            data.received(),
            data.transmitted(),
            data.total_packets_received(),
            data.total_packets_transmitted(),
            data.total_errors_on_received(),
            data.mac_address(),
            data.mtu()
        );
    }

    println!("\n== disks ==");
    for d in disks.iter() {
        println!(
            "disk={:?} mount={:?} fs={:?} kind={:?} total={} available={} removable={} read_only={}",
            d.name(),
            d.mount_point(),
            d.file_system(),
            d.kind(),
            d.total_space(),
            d.available_space(),
            d.is_removable(),
            d.is_read_only()
        );
    }

    // ---- 4. プロセス情報 ----
    println!("\n== processes ==");
    println!("process_count={}", sys.processes().len());
    let mut procs: Vec<ProcRow> = sys
        .processes()
        .iter()
        .map(|(pid, p)| {
            let user = p
                .user_id()
                .and_then(|uid| users.get_user_by_id(uid))
                .map(|u| u.name().to_string());
            let cmdline = p
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            let du = p.disk_usage();
            ProcRow {
                pid: pid.as_u32(),
                parent: p.parent().map(|pp| pp.as_u32()),
                name: p.name().to_string_lossy().to_string(),
                user,
                status: format!("{:?}", p.status()),
                start_time_since_boot_secs: p.start_time(),
                run_time_secs: p.run_time(),
                cpu_percent: p.cpu_usage(),
                memory_bytes: p.memory(),
                virtual_memory_bytes: p.virtual_memory(),
                cmdline,
                exe: p.exe().map(|e| e.display().to_string()),
                cwd: p.cwd().map(|c| c.display().to_string()),
                disk_read_bytes: du.read_bytes,
                disk_written_bytes: du.written_bytes,
            }
        })
        .collect();
    procs.sort_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap_or(std::cmp::Ordering::Equal));
    println!("top 5 by cpu:");
    for p in procs.iter().take(5) {
        println!(
            "pid={} name={:?} user={:?} status={} start_time={} run_time={} cpu={:.2} mem={} cmdline={:?}",
            p.pid, p.name, p.user, p.status, p.start_time_since_boot_secs, p.run_time_secs, p.cpu_percent, p.memory_bytes, p.cmdline
        );
    }
    let no_user = procs.iter().filter(|p| p.user.is_none()).count();
    let no_cmd = procs.iter().filter(|p| p.cmdline.is_empty()).count();
    let no_exe = procs.iter().filter(|p| p.exe.is_none()).count();
    println!("field coverage: user=None {no_user}/{} / cmdline empty {no_cmd}/{} / exe=None {no_exe}/{}",
        procs.len(), procs.len(), procs.len());

    // hidepid の影響確認（/proc のマウントオプションを読むだけ）
    let mut hidepid_line = String::from("(not found)");
    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            if line.split_whitespace().nth(1) == Some("/proc") {
                hidepid_line = line.to_string();
                break;
            }
        }
    }
    println!("\n/proc mount line: {hidepid_line}");

    // ---- 5. kill 機能の実証（自分が spawn した子プロセスのみ）----
    println!("\n== kill test (own child only) ==");
    let mut child = Command::new("sleep").arg("30").spawn().expect("spawn sleep failed");
    let child_pid = Pid::from_u32(child.id());
    std::thread::sleep(Duration::from_millis(300));
    sys.refresh_processes(ProcessesToUpdate::Some(&[child_pid]), true);
    let before = sys.process(child_pid).map(|p| {
        (p.name().to_string_lossy().to_string(), p.start_time(), format!("{:?}", p.status()))
    });
    println!("spawned pid={} before={:?}", child.id(), before);
    let kill_result = sys.process(child_pid).and_then(|p| p.kill_with(Signal::Term));
    println!("kill_with(Signal::Term) -> {kill_result:?}");
    std::thread::sleep(Duration::from_millis(300));
    sys.refresh_processes(ProcessesToUpdate::Some(&[child_pid]), true);
    let still_there = sys.process(child_pid).is_some();
    let wait = child.try_wait();
    println!("after kill: still_in_sysinfo={still_there} child.try_wait()={wait:?}");
    let _ = child.kill(); // 念のため（既に終了していれば Err は無視）

    // ---- 6. JSON シリアライズ（ローカル agent の wire format 実現性）----
    let snapshot = Snapshot {
        schema: "gpui-task-manager/snapshot/v1",
        captured_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        sysinfo_crate_version: SYSINFO_CRATE_VERSION,
        host_name: System::host_name().ok(),
        os_name: System::name().ok(),
        os_version: System::os_version().ok(),
        kernel_version: System::kernel_version().ok(),
        uptime_secs: System::uptime().ok(),
        boot_time_unix: System::boot_time().ok(),
        load_avg: System::load_average().ok().map(|l| [l.one, l.five, l.fifteen]),
        physical_cores: System::physical_core_count().ok(),
        logical_cpus: sys.cpus().len(),
        cpu_brand: cpu_brand.clone(),
        cpu_frequency_mhz,
        global_cpu_percent: sys.global_cpu_usage(),
        per_core_cpu_percent: sys.cpus().iter().map(|c| c.usage()).collect(),
        memory_total: sys.total_memory(),
        memory_used: sys.used_memory(),
        memory_available: sys.available_memory(),
        swap_total: sys.total_swap(),
        swap_used: sys.used_swap(),
        networks: networks
            .iter()
            .map(|(name, d)| NetRow {
                name: name.to_string_lossy().to_string(),
                total_received: d.total_received(),
                total_transmitted: d.total_transmitted(),
                received_since_last: d.received(),
                transmitted_since_last: d.transmitted(),
                packets_received: d.total_packets_received(),
                packets_transmitted: d.total_packets_transmitted(),
                errors_on_received: d.total_errors_on_received(),
            })
            .collect(),
        disks: disks
            .iter()
            .map(|d| DiskRow {
                name: d.name().to_string_lossy().to_string(),
                mount_point: d.mount_point().display().to_string(),
                file_system: d.file_system().to_string_lossy().to_string(),
                kind: format!("{:?}", d.kind()),
                total_space: d.total_space(),
                available_space: d.available_space(),
                is_removable: d.is_removable(),
                is_read_only: d.is_read_only(),
            })
            .collect(),
        process_count: procs.len(),
        processes: procs,
    };
    let json = serde_json::to_string(&snapshot).expect("serialize failed");
    println!("\n== json ==");
    println!("snapshot json bytes={} (processes={})", json.len(), snapshot.process_count);
    println!("head: {}", &json[..json.len().min(600)]);
    println!("\nDONE");
}
