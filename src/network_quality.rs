use std::net::{Ipv4Addr, SocketAddrV4, TcpStream, ToSocketAddrs};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::thread::{self};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_SUCCESS, GetLastError, WIN32_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    GetTcpStatisticsEx, ICMP_ECHO_REPLY, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho,
    MIB_TCPSTATS_LH,
};

use crate::{report_error_log, report_info_log};

use crate::global::{
    DEFAULT_PING_COUNT, DEFAULT_PING_TARGET, DEFAULT_PING_TIMEOUT_MS, DEFAULT_PROBE_INTERVAL_SECS,
    IP_FAMILY_IPV4, NetworkQualitySample, QUALITY_RUNNING, QUALITY_THREAD, report_net_quality,
};

// TCP 统计结果：用于计算重传率并补充其他质量指标
#[derive(Debug)]
struct TcpStats {
    retransmission_percent: f64,
    segments_sent: i64,
    segments_retransmitted: i64,
}

static TCP_STATS_BASELINE: OnceLock<Mutex<Option<(i64, i64)>>> = OnceLock::new();

// ICMP 探测结果：用于计算延迟、抖动与丢包
#[derive(Debug)]
struct PingStats {
    avg_ms: u32,
    min_ms: u32,
    max_ms: u32,
    jitter_ms: u32,
    loss_percent: f64,
    success_count: usize,
    last_error: u32,
    last_reply_status: Option<u32>,
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
        init_tcp_stats_baseline();
        while QUALITY_RUNNING.load(Ordering::SeqCst) {
            let start_at = Instant::now();
            if let Some(sample) = probe_quality_once() {
                report_quality_sample(&sample);
                report_net_quality(sample);
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
    reset_tcp_stats_baseline();
}

// 执行一次完整的质量探测：包含延迟、丢包和 TCP 重传率
fn probe_quality_once() -> Option<NetworkQualitySample> {
    let target = resolve_ipv4_target(DEFAULT_PING_TARGET)?;
    let mut ping = measure_latency_and_loss(target, DEFAULT_PING_COUNT, DEFAULT_PING_TIMEOUT_MS);
    if let Some(stats) = ping.as_ref()
        && stats.success_count == 0
    {
        report_info_log!(
            "ICMP 探测全失败，切换为 TCP 握手 RTT 探测：target={} ipv4={} success_count={}/{} last_error={} last_reply_status={:?}",
            DEFAULT_PING_TARGET,
            target,
            stats.success_count,
            DEFAULT_PING_COUNT,
            stats.last_error,
            stats.last_reply_status
        );
        ping = measure_tcp_handshake_rtt(
            DEFAULT_PING_TARGET,
            443,
            DEFAULT_PING_COUNT,
            Duration::from_millis(DEFAULT_PING_TIMEOUT_MS as u64),
        );
    }
    let tcp_stats = query_tcp_stats();

    Some(NetworkQualitySample {
        latency_avg_ms: ping.as_ref().map(|p| p.avg_ms).unwrap_or(0),
        latency_min_ms: ping.as_ref().map(|p| p.min_ms).unwrap_or(0),
        latency_max_ms: ping.as_ref().map(|p| p.max_ms).unwrap_or(0),
        jitter_ms: ping.as_ref().map(|p| p.jitter_ms).unwrap_or(0),
        packet_loss_percent: ping.as_ref().map(|p| p.loss_percent).unwrap_or(0.0),
        tcp_retransmission_percent: tcp_stats
            .as_ref()
            .map(|t| t.retransmission_percent)
            .unwrap_or(0.0),
        tcp_segments_sent: tcp_stats.as_ref().map(|t| t.segments_sent).unwrap_or(0),
        tcp_segments_retransmitted: tcp_stats
            .as_ref()
            .map(|t| t.segments_retransmitted)
            .unwrap_or(0),
    })
}

// 记录采样结果：统一输出，便于日志聚合与后续消费
fn report_quality_sample(sample: &NetworkQualitySample) {
    let retransmission_percent_total = compute_retransmission_percent_total(
        sample.tcp_segments_sent,
        sample.tcp_segments_retransmitted,
    );
    report_info_log!(
        "网络质量采样：延迟avg={:?}ms,min={:?}ms,max={:?}ms,jitter={:?}ms,丢包={:?}%,重传率(out)={:?}%,重传率(total)={:?}%,发送段={:?},重传段={:?}",
        sample.latency_avg_ms,
        sample.latency_min_ms,
        sample.latency_max_ms,
        sample.jitter_ms,
        sample.packet_loss_percent,
        sample.tcp_retransmission_percent,
        retransmission_percent_total,
        sample.tcp_segments_sent,
        sample.tcp_segments_retransmitted
    );
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
    let mut last_error = 0u32;
    let mut last_reply_status: Option<u32> = None;
    let payload = [0u8; 32];
    let reply_size = (std::mem::size_of::<ICMP_ECHO_REPLY>() + payload.len()) as u32;

    for _ in 0..count {
        let mut reply_buffer = vec![0u8; reply_size as usize];
        // IcmpSendEcho 的目标 IP 字节序必须使用小端序
        // 虽然网络字节序为大端序，但是 x86/x64/ARM 架构使用是小端序
        // 192.168.0.1 被存储为 01 00 A8 C0
        // 如果以大端序传入，实际ping的是 1.0.168.192
        let response_count = unsafe {
            IcmpSendEcho(
                handle,
                u32::from_le_bytes(target.octets()),
                payload.as_ptr().cast(),
                payload.len() as u16,
                None,
                reply_buffer.as_mut_ptr().cast(),
                reply_size,
                timeout_ms,
            )
        };

        if response_count > 0 {
            let reply = unsafe { &*(reply_buffer.as_ptr() as *const ICMP_ECHO_REPLY) };
            last_reply_status = Some(reply.Status);
            if reply.Status == ERROR_SUCCESS.0 {
                rtts.push(reply.RoundTripTime);
                success_count += 1;
            }
        } else {
            last_error = unsafe { GetLastError().0 };
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
            success_count,
            last_error,
            last_reply_status,
        });
    }

    let min_ms = *rtts.iter().min().unwrap();
    let max_ms = *rtts.iter().max().unwrap();
    let sum: u32 = rtts.iter().copied().sum();
    let avg_ms = sum / rtts.len() as u32;
    let jitter_ms = compute_jitter(&rtts);
    let failure_count = count.saturating_sub(success_count);
    let loss_percent = (failure_count as f64 / count as f64) * 100.0;

    Some(PingStats {
        avg_ms,
        min_ms,
        max_ms,
        jitter_ms,
        loss_percent,
        success_count,
        last_error,
        last_reply_status,
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

fn measure_tcp_handshake_rtt(
    target: &str,
    port: u16,
    count: usize,
    timeout: Duration,
) -> Option<PingStats> {
    let addrs = resolve_ipv4_socket_addrs(target, port)?;
    let addr = addrs.first().copied()?;

    let mut rtts = Vec::with_capacity(count);
    let mut success_count = 0usize;
    let mut last_error = 0u32;

    for _ in 0..count {
        let start_at = Instant::now();
        match TcpStream::connect_timeout(&addr.into(), timeout) {
            Ok(stream) => {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                let elapsed_ms = start_at.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
                rtts.push(elapsed_ms);
                success_count += 1;
            }
            Err(error) => {
                last_error = error.raw_os_error().unwrap_or(0) as u32;
            }
        }
    }

    if rtts.is_empty() {
        return Some(PingStats {
            avg_ms: 0,
            min_ms: 0,
            max_ms: 0,
            jitter_ms: 0,
            loss_percent: 100.0,
            success_count,
            last_error,
            last_reply_status: None,
        });
    }

    let min_ms = *rtts.iter().min().unwrap();
    let max_ms = *rtts.iter().max().unwrap();
    let sum: u32 = rtts.iter().copied().sum();
    let avg_ms = sum / rtts.len() as u32;
    let jitter_ms = compute_jitter(&rtts);
    let failure_count = count.saturating_sub(success_count);
    let loss_percent = (failure_count as f64 / count as f64) * 100.0;

    Some(PingStats {
        avg_ms,
        min_ms,
        max_ms,
        jitter_ms,
        loss_percent,
        success_count,
        last_error,
        last_reply_status: None,
    })
}

// 读取系统 TCP 统计并计算重传率
fn query_tcp_stats() -> Option<TcpStats> {
    let (current_sent, current_retrans) = read_tcp_counters()?;
    let baseline_lock = TCP_STATS_BASELINE.get_or_init(|| Mutex::new(None));
    let mut baseline = baseline_lock.lock().unwrap();
    let previous = *baseline;
    let stats = compute_interval_tcp_stats(&mut baseline, (current_sent, current_retrans));

    if cfg!(debug_assertions) {
        report_info_log!(
            "TCP 重传率（周期内）：prev={:?} curr=({},{}) delta=({},{}) percent={:.6}%",
            previous,
            current_sent,
            current_retrans,
            stats.segments_sent,
            stats.segments_retransmitted,
            stats.retransmission_percent
        );
    }

    Some(stats)
}

fn init_tcp_stats_baseline() {
    let baseline_lock = TCP_STATS_BASELINE.get_or_init(|| Mutex::new(None));
    let mut baseline = baseline_lock.lock().unwrap();
    if baseline.is_some() {
        return;
    }
    if let Some((sent, retrans)) = read_tcp_counters() {
        *baseline = Some((sent, retrans));
        if cfg!(debug_assertions) {
            report_info_log!("TCP 重传率（周期开始）：baseline=({},{})", sent, retrans);
        }
    }
}

fn reset_tcp_stats_baseline() {
    if let Some(lock) = TCP_STATS_BASELINE.get() {
        *lock.lock().unwrap() = None;
    }
}

fn read_tcp_counters() -> Option<(i64, i64)> {
    let mut stats = MIB_TCPSTATS_LH::default();
    let result = unsafe { GetTcpStatisticsEx(&mut stats, IP_FAMILY_IPV4) };
    if result != ERROR_SUCCESS.0 {
        report_error_log!("GetTcpStatisticsEx 失败: {:?}", WIN32_ERROR(result));
        return None;
    }
    Some((stats.dwOutSegs as i64, stats.dwRetransSegs as i64))
}

fn compute_interval_tcp_stats(baseline: &mut Option<(i64, i64)>, current: (i64, i64)) -> TcpStats {
    let (current_sent, current_retrans) = current;
    let Some((prev_sent, prev_retrans)) = *baseline else {
        *baseline = Some(current);
        return TcpStats {
            retransmission_percent: 0.0,
            segments_sent: 0,
            segments_retransmitted: 0,
        };
    };

    if current_sent < prev_sent || current_retrans < prev_retrans {
        *baseline = Some(current);
        return TcpStats {
            retransmission_percent: 0.0,
            segments_sent: 0,
            segments_retransmitted: 0,
        };
    }

    let delta_sent = current_sent - prev_sent;
    let delta_retrans = current_retrans - prev_retrans;
    *baseline = Some(current);

    let retransmission_percent = compute_retransmission_percent_out(delta_sent, delta_retrans);

    TcpStats {
        retransmission_percent,
        segments_sent: delta_sent,
        segments_retransmitted: delta_retrans,
    }
}

fn resolve_ipv4_target(target: &str) -> Option<Ipv4Addr> {
    if let Ok(ipv4) = target.parse::<Ipv4Addr>() {
        return Some(ipv4);
    }

    let mut addrs = (target, 0).to_socket_addrs().ok()?;
    addrs.find_map(|addr| match addr.ip() {
        std::net::IpAddr::V4(ipv4) => Some(ipv4),
        std::net::IpAddr::V6(_) => None,
    })
}

fn resolve_ipv4_socket_addrs(host: &str, port: u16) -> Option<Vec<SocketAddrV4>> {
    let addrs = (host, port).to_socket_addrs().ok()?;
    let mut result = Vec::new();
    for addr in addrs {
        if let std::net::SocketAddr::V4(v4) = addr {
            result.push(v4);
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

fn compute_retransmission_percent_out(segments_sent: i64, segments_retransmitted: i64) -> f64 {
    if segments_sent <= 0 {
        return 0.0;
    }
    (segments_retransmitted.max(0) as f64 / segments_sent as f64) * 100.0
}

fn compute_retransmission_percent_total(segments_sent: i64, segments_retransmitted: i64) -> f64 {
    let sent = segments_sent.max(0) as f64;
    let retrans = segments_retransmitted.max(0) as f64;
    let total = sent + retrans;
    if total == 0.0 {
        0.0
    } else {
        (retrans / total) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compute_interval_tcp_stats, compute_retransmission_percent_out,
        compute_retransmission_percent_total,
    };

    #[test]
    fn retransmission_percent_formulas_match_expectations() {
        let sent = 4_238_258i64;
        let retrans = 32_906i64;
        let out = compute_retransmission_percent_out(sent, retrans);
        let total = compute_retransmission_percent_total(sent, retrans);
        assert!(out > total);
        assert!((out - 0.776).abs() < 0.01);
        assert!((total - 0.770).abs() < 0.01);
    }

    #[test]
    fn interval_stats_resets_on_first_sample() {
        let mut baseline = None;
        let stats = compute_interval_tcp_stats(&mut baseline, (100, 10));
        assert_eq!(stats.segments_sent, 0);
        assert_eq!(stats.segments_retransmitted, 0);
        assert_eq!(stats.retransmission_percent, 0.0);
        assert_eq!(baseline, Some((100, 10)));
    }

    #[test]
    fn interval_stats_isolated_across_cycles() {
        let mut baseline = Some((100, 10));
        let stats1 = compute_interval_tcp_stats(&mut baseline, (150, 12));
        assert_eq!(stats1.segments_sent, 50);
        assert_eq!(stats1.segments_retransmitted, 2);
        assert!((stats1.retransmission_percent - 4.0).abs() < 1e-9);
        assert_eq!(baseline, Some((150, 12)));

        let stats2 = compute_interval_tcp_stats(&mut baseline, (180, 12));
        assert_eq!(stats2.segments_sent, 30);
        assert_eq!(stats2.segments_retransmitted, 0);
        assert_eq!(stats2.retransmission_percent, 0.0);
        assert_eq!(baseline, Some((180, 12)));
    }

    #[test]
    fn interval_stats_handles_counter_reset() {
        let mut baseline = Some((200, 20));
        let stats = compute_interval_tcp_stats(&mut baseline, (50, 2));
        assert_eq!(stats.segments_sent, 0);
        assert_eq!(stats.segments_retransmitted, 0);
        assert_eq!(stats.retransmission_percent, 0.0);
        assert_eq!(baseline, Some((50, 2)));
    }
}
