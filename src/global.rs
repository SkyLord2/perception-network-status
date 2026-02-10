use std::cell::RefCell;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Instant;

use chrono::Local;

use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;

use windows::Win32::Networking::NetworkListManager::{
    INetworkListManager, INetworkListManagerEvents,
};
use windows::Win32::System::Com::{IConnectionPoint, IConnectionPointContainer};

pub static SOME_EVENT: OnceLock<Mutex<(String, Instant)>> = OnceLock::new();

pub static GLOBAL_REPORT_NET_STATUS: OnceLock<ThreadsafeFunction<NetworkStatus>> = OnceLock::new();

pub static GLOBAL_REPORT_WLAN_STATUS: OnceLock<ThreadsafeFunction<WlanStatus>> = OnceLock::new();

pub static GLOBAL_REPORT_NET_QUALITY: OnceLock<ThreadsafeFunction<NetworkQualitySample>> =
    OnceLock::new();

pub static GLOBAL_LOG: OnceLock<ThreadsafeFunction<String>> = OnceLock::new();

// 用于记录后台监控线程的 ID
pub static MONITOR_THREAD_ID: AtomicU32 = AtomicU32::new(0);

pub static THRESHOLD_DROP: AtomicU32 = AtomicU32::new(0);
pub static THRESHOLD_RECOVER: AtomicU32 = AtomicU32::new(0);

// 监控线程是否已经启动，避免重复创建线程
pub static MONITOR_STARTED: AtomicBool = AtomicBool::new(false);

// 当前网络是否具备互联网连通性。
//
// 说明：
// - 监控线程（COM/NLM 回调）与 WLAN 回调可能运行在不同线程。
// - 原先将 network_connected 放在 thread_local 的 MonitorState 中，会导致其他线程看到的值始终是默认 false。
// - 因此把“是否联网”提升为跨线程可见的原子状态，避免线程局部存储带来的状态割裂。
pub static NETWORK_CONNECTED: AtomicBool = AtomicBool::new(false);

// WLAN 信号强度监控上下文：保存阈值与当前状态，供回调使用
pub struct SignalMonitorContext {
    pub wlan_handle: isize,
    pub threshold_drop: u32,
    pub threshold_recover: u32,
    pub is_signal_weak: bool,
    pub last_quality: u32,
}

pub const DEFAULT_PING_TARGET: &str = "www.baidu.com";
pub const DEFAULT_PING_COUNT: usize = 10;
pub const DEFAULT_PING_TIMEOUT_MS: u32 = 3000;
pub const DEFAULT_PROBE_INTERVAL_SECS: u64 = 10;
pub const IP_FAMILY_IPV4: u32 = 2;

pub static QUALITY_RUNNING: AtomicBool = AtomicBool::new(false);
pub static QUALITY_THREAD: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();

// 网络质量采样结果：用于记录一次探测周期内的主要指标
#[napi(object)]
#[derive(Debug, Clone)]
pub struct NetworkQualitySample {
    pub latency_avg_ms: u32,
    pub latency_min_ms: u32,
    pub latency_max_ms: u32,
    pub jitter_ms: u32,
    pub packet_loss_percent: f64,
    pub tcp_retransmission_percent: f64,
    pub tcp_segments_sent: i64,
    pub tcp_segments_retransmitted: i64,
}

// 监控相关的全局状态，统一保存在 global.rs 里
pub struct MonitorState {
    pub network_list_manager: Option<INetworkListManager>,
    pub connection_point_container: Option<IConnectionPointContainer>,
    pub connection_point: Option<IConnectionPoint>,
    pub event_sink: Option<INetworkListManagerEvents>,
    pub cookie: u32,
    pub signal_context: Option<Arc<Mutex<SignalMonitorContext>>>,
}

thread_local! {
    pub static MONITOR_STATE: RefCell<MonitorState> = const { RefCell::new(MonitorState {
        network_list_manager: None,
        connection_point_container: None,
        connection_point: None,
        event_sink: None,
        cookie: 0,
        signal_context: None,
    }) };
}

pub fn with_monitor_state<F, R>(action: F) -> R
where
    F: FnOnce(&mut MonitorState) -> R,
{
    MONITOR_STATE.with(|state| action(&mut state.borrow_mut()))
}

#[napi(object)]
#[derive(Clone)]
pub struct NetworkStatus {
    pub status: u32,
}

#[napi(object)]
#[derive(Clone)]
pub struct WlanStatus {
    pub strong: i32,
    pub quality: u32,
    pub rssi: i32,
}

pub fn report_network_status(info: NetworkStatus) {
    if let Some(tsfn) = GLOBAL_REPORT_NET_STATUS.get() {
        tsfn.call(Ok(info), ThreadsafeFunctionCallMode::NonBlocking);
    } else {
        println!("Warning: No report wnd listener registered yet!");
    }
}

pub fn report_wlan_status(info: WlanStatus) {
    if let Some(tsfn) = GLOBAL_REPORT_WLAN_STATUS.get() {
        tsfn.call(Ok(info), ThreadsafeFunctionCallMode::NonBlocking);
    } else {
        println!("Warning: No report wlan status listener registered yet!");
    }
}

pub fn report_net_quality(info: NetworkQualitySample) {
    if let Some(tsfn) = GLOBAL_REPORT_NET_QUALITY.get() {
        tsfn.call(Ok(info), ThreadsafeFunctionCallMode::NonBlocking);
    } else {
        println!("Warning: No report net quality listener registered yet!");
    }
}

fn report_log(msg: String) {
    if cfg!(debug_assertions) {
        println!("{}", msg);
    } else if let Some(tsfn) = GLOBAL_LOG.get() {
        tsfn.call(Ok(msg), ThreadsafeFunctionCallMode::NonBlocking);
    } else {
        println!("Warning: No report log listener registered yet!");
    }
}

#[doc(hidden)]
pub(crate) fn report_error(
    msg: fmt::Arguments,
    module_path: &'static str,
    file: &'static str,
    line: u32,
    column: u32,
) {
    let curr_time = get_current_time();
    let log_msg = format!(
        "[selection_error]:{} - {}:{}:{} {} - {}",
        curr_time, file, line, column, module_path, msg
    );
    report_log(log_msg);
}

#[doc(hidden)]
pub(crate) fn report_info(
    msg: fmt::Arguments,
    module_path: &'static str,
    file: &'static str,
    line: u32,
    column: u32,
) {
    let curr_time = get_current_time();
    let log_msg = format!(
        "[info]:{} - {}:{}:{} {} - {}",
        curr_time, file, line, column, module_path, msg
    );
    report_log(log_msg);
}

#[macro_export]
macro_rules! report_error_log {
    // format_args! 是编译器内置宏，它不分配内存，只打包参数
    ($($arg:tt)*) => {
        $crate::global::report_error(
            format_args!($($arg)*),
            module_path!(),
            file!(),
            line!(),
            column!(),
        )
    }
}

#[macro_export]
macro_rules! report_info_log {
    // format_args! 是编译器内置宏，它不分配内存，只打包参数
    ($($arg:tt)*) => {
        $crate::global::report_info(
            format_args!($($arg)*),
            module_path!(),
            file!(),
            line!(),
            column!(),
        )
    }
}

pub fn get_current_time() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S.%3f").to_string()
}

#[cfg(test)]
mod tests {
    use super::NETWORK_CONNECTED;
    use std::sync::atomic::Ordering;

    #[test]
    fn network_connected_is_visible_across_threads() {
        NETWORK_CONNECTED.store(false, Ordering::SeqCst);

        let handle = std::thread::spawn(|| {
            NETWORK_CONNECTED.store(true, Ordering::SeqCst);
        });
        handle.join().unwrap();

        assert!(NETWORK_CONNECTED.load(Ordering::SeqCst));
        NETWORK_CONNECTED.store(false, Ordering::SeqCst);
    }
}
