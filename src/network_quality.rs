use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    GetTcpStatisticsEx, ICMP_ECHO_REPLY, IP_OPTION_INFORMATION, IcmpCloseHandle, IcmpCreateFile,
    IcmpSendEcho, MIB_TCPSTATS_LH,
};

use crate::{report_error_log, report_info_log};

const DEFAULT_PING_TARGET: &str = "8.8.8.8";
const DEFAULT_PING_COUNT: usize = 4;
const DEFAULT_PING_TIMEOUT_MS: u32 = 1000;
const DEFAULT_PROBE_INTERVAL_SECS: u64 = 10;
const IP_FAMILY_IPV4: u32 = 2;

static QUALITY_RUNNING: AtomicBool = AtomicBool::new(false);
static QUALITY_THREAD: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();

// 网络质量采样结果：用于记录一次探测周期内的主要指标
#[derive(Debug, Clone)]
struct NetworkQualitySample {
    latency_avg_ms: Option<u32>,
    latency_min_ms: Option<u32>,
    latency_max_ms: Option<u32>,
    jitter_ms: Option<u32>,
    packet_loss_percent: Option<f32>,
    tcp_retransmission_percent: Option<f32>,
    tcp_segments_sent: Option<u64>,
    tcp_segments_retransmitted: Option<u64>,
}

// 启动网络质量探测线程：周期性采样并输出到日志
pub fn start_quality_probe() {
    let already_running = QUALITY_RUNNING.swap(true, Ordering::SeqCst);
    if already_running {
        report_info_log!("网络质量探测线程已启动，跳过重复创建");
        return;
    }

    let handle = thread::spawn(|| {
        let interval = Duration::from_secs(DEFAULT_PROBE_INTERVAL_SECS);
        while QUALITY_RUNNING.load(Ordering::SeqCst) {
            let start_at = Instant::now();
            if let Some(sample) = probe_quality_once() {
                report_quality_sample(&sample);
            }

            let elapsed = start_at.elapsed();
            if elapsed < interval {
                thread::sleep(interval - elapsed);
            }
        }
    });

    QUALITY_THREAD
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .replace(handle);
}

// 停止网络质量探测线程：等待后台线程退出
pub fn stop_quality_probe() {
    QUALITY_RUNNING.store(false, Ordering::SeqCst);
    if let Some(lock) = QUALITY_THREAD.get()
        && let Some(handle) = lock.lock().unwrap().take()
    {
        let _ = handle.join();
    }
}

// 执行一次完整的质量探测：包含延迟、丢包和 TCP 重传率
fn probe_quality_once() -> Option<NetworkQualitySample> {
    let target = DEFAULT_PING_TARGET.parse::<Ipv4Addr>().ok()?;
    let ping = measure_latency_and_loss(target, DEFAULT_PING_COUNT, DEFAULT_PING_TIMEOUT_MS);
    let tcp_stats = query_tcp_stats();

    Some(NetworkQualitySample {
        latency_avg_ms: ping.as_ref().map(|p| p.avg_ms),
        latency_min_ms: ping.as_ref().map(|p| p.min_ms),
        latency_max_ms: ping.as_ref().map(|p| p.max_ms),
        jitter_ms: ping.as_ref().map(|p| p.jitter_ms),
        packet_loss_percent: ping.as_ref().map(|p| p.loss_percent),
        tcp_retransmission_percent: tcp_stats.as_ref().map(|t| t.retransmission_percent),
        tcp_segments_sent: tcp_stats.as_ref().map(|t| t.segments_sent),
        tcp_segments_retransmitted: tcp_stats.as_ref().map(|t| t.segments_retransmitted),
    })
}

// 记录采样结果：统一输出，便于日志聚合与后续消费
fn report_quality_sample(sample: &NetworkQualitySample) {
    report_info_log!(
        "网络质量采样：延迟avg={:?}ms,min={:?}ms,max={:?}ms,jitter={:?}ms,丢包={:?}%,重传率={:?}%,发送段={:?},重传段={:?}",
        sample.latency_avg_ms,
        sample.latency_min_ms,
        sample.latency_max_ms,
        sample.jitter_ms,
        sample.packet_loss_percent,
        sample.tcp_retransmission_percent,
        sample.tcp_segments_sent,
        sample.tcp_segments_retransmitted
    );
}

// ICMP 探测结果：用于计算延迟、抖动与丢包
#[derive(Debug)]
struct PingStats {
    avg_ms: u32,
    min_ms: u32,
    max_ms: u32,
    jitter_ms: u32,
    loss_percent: f32,
}

// 计算指定目标的延迟与丢包率
fn measure_latency_and_loss(target: Ipv4Addr, count: usize, timeout_ms: u32) -> Option<PingStats> {
    let handle = unsafe { IcmpCreateFile() };
    let handle = match handle {
        Ok(handle) => handle,
        Err(error) => {
            report_error_log!("IcmpCreateFile 失败: {}", error);
            return None;
        }
    };

    let mut rtts = Vec::with_capacity(count);
    let mut success_count = 0usize;
    let payload = [0u8; 32];
    let reply_size = (std::mem::size_of::<ICMP_ECHO_REPLY>() + payload.len()) as u32;

    for _ in 0..count {
        let mut reply_buffer = vec![0u8; reply_size as usize];
        let response_count = unsafe {
            IcmpSendEcho(
                handle,
                u32::from(target),
                payload.as_ptr().cast(),
                payload.len() as u16,
                Some(&IP_OPTION_INFORMATION::default()),
                reply_buffer.as_mut_ptr().cast(),
                reply_size,
                timeout_ms,
            )
        };

        if response_count > 0 {
            let reply = unsafe { &*(reply_buffer.as_ptr() as *const ICMP_ECHO_REPLY) };
            if reply.Status == ERROR_SUCCESS.0 {
                rtts.push(reply.RoundTripTime);
                success_count += 1;
            }
        }
    }

    let _ = unsafe { IcmpCloseHandle(handle) };

    if rtts.is_empty() {
        return Some(PingStats {
            avg_ms: 0,
            min_ms: 0,
            max_ms: 0,
            jitter_ms: 0,
            loss_percent: 100.0,
        });
    }

    let min_ms = *rtts.iter().min().unwrap();
    let max_ms = *rtts.iter().max().unwrap();
    let sum: u32 = rtts.iter().copied().sum();
    let avg_ms = sum / rtts.len() as u32;
    let jitter_ms = compute_jitter(&rtts);
    let loss_percent = ((count - success_count) as f32 / count as f32) * 100.0;

    Some(PingStats {
        avg_ms,
        min_ms,
        max_ms,
        jitter_ms,
        loss_percent,
    })
}

// 计算简单抖动指标：相邻 RTT 差值的平均值
fn compute_jitter(rtts: &[u32]) -> u32 {
    if rtts.len() < 2 {
        return 0;
    }
    let mut sum = 0u32;
    for pair in rtts.windows(2) {
        let diff = pair[0].abs_diff(pair[1]);
        sum += diff;
    }
    sum / (rtts.len() as u32 - 1)
}

// TCP 统计结果：用于计算重传率并补充其他质量指标
#[derive(Debug)]
struct TcpStats {
    retransmission_percent: f32,
    segments_sent: u64,
    segments_retransmitted: u64,
}

// 读取系统 TCP 统计并计算重传率
fn query_tcp_stats() -> Option<TcpStats> {
    let mut stats = MIB_TCPSTATS_LH::default();
    let result = unsafe { GetTcpStatisticsEx(&mut stats, IP_FAMILY_IPV4) };
    if result != ERROR_SUCCESS.0 {
        report_error_log!("GetTcpStatisticsEx 失败: {:?}", WIN32_ERROR(result));
        return None;
    }

    let segments_sent = stats.dwOutSegs as u64;
    let segments_retransmitted = stats.dwRetransSegs as u64;
    let total = segments_sent + segments_retransmitted;
    let retransmission_percent = if total == 0 {
        0.0
    } else {
        (segments_retransmitted as f32 / total as f32) * 100.0
    };

    Some(TcpStats {
        retransmission_percent,
        segments_sent,
        segments_retransmitted,
    })
}
