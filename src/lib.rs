#![deny(clippy::all)]
mod global;
mod messages;
mod monitor;
mod network;
mod network_quality;
mod wlan;

use napi::threadsafe_function::ThreadsafeFunction;
use napi::{Env, Status};
use napi_derive::napi;

use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::global::{ARGS, GLOBAL_LOG, GLOBAL_REPORT, SOME_EVENT, SomeInfo, report_func};
use crate::monitor::{cleanup_monitor_thread, start_monitor_thread};

// Node 侧初始化入口：注册回调、启动监控线程，并推送一次空消息用于握手
#[napi]
pub fn do_initialize(
    args: u32,
    mut report: ThreadsafeFunction<Vec<SomeInfo>>,
    mut log: ThreadsafeFunction<String>,
    env: Env,
) -> napi::Result<()> {
    // 仅在初始化阶段持有线程安全函数，随后交由全局缓存管理
    #[allow(deprecated)]
    report.unref(&env)?;
    #[allow(deprecated)]
    log.unref(&env)?;

    GLOBAL_REPORT.set(report).map_err(|_| {
        napi::Error::new(
            Status::GenericFailure,
            "Global report listener already registered",
        )
    })?;
    GLOBAL_LOG.set(log).map_err(|_| {
        napi::Error::new(
            Status::GenericFailure,
            "Global log listener already registered",
        )
    })?;

    // 初始化事件节流缓存，避免高频日志冲击主线程
    SOME_EVENT.get_or_init(|| {
        Mutex::new((
            String::from("Ready"),
            Instant::now() - Duration::from_secs(100),
        ))
    });

    // 保存初始化参数，供 WLAN 阈值等配置在后台线程解析
    ARGS.store(args, Ordering::SeqCst);

    if cfg!(debug_assertions) {
        report_info_log!("[Debug] 当前正处于开发模式运行，开启详细日志...");
    } else {
        report_info_log!("[Release] 生产模式运行");
    }

    // 绑定清理钩子，确保 Node 退出时请求监控线程停止
    env.add_env_cleanup_hook(null_mut(), |arg| unsafe { cleanup_monitor_thread(arg) })?;

    // 启动后台监控线程：网络事件与 WLAN 事件在该线程中处理
    start_monitor_thread();

    // 发送一次空结构用于通知 JS 侧初始化完成
    report_func(vec![SomeInfo {
        pname: "".to_string(),
        pid: 0,
        title: "".to_string(),
    }]);

    Ok(())
}
