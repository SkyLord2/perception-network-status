use std::sync::atomic::Ordering;
use std::thread;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_QUIT,
};

use crate::global::{MONITOR_STARTED, MONITOR_THREAD_ID, with_monitor_state};
use crate::messages::{WM_NETWORK_STATUS_CHANGE, WM_WIFI_SIGNAL_CHANGE};
use crate::{network, network_quality, wlan};
use crate::{report_error_log, report_info_log};

// 启动后台监控线程：负责初始化 COM、网络/WLAN 监听与消息循环
pub fn start_monitor_thread() {
    let already_started = MONITOR_STARTED.swap(true, Ordering::SeqCst);
    if already_started {
        report_info_log!("后台监控线程已启动，跳过重复创建");
        return;
    }

    thread::spawn(|| {
        let thread_id = unsafe { GetCurrentThreadId() };
        MONITOR_THREAD_ID.store(thread_id, Ordering::SeqCst);

        let com_result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if com_result.is_err() {
            report_error_log!("初始化 COM 失败: {:?}", com_result);
        }

        if let Err(error) = network::initialize_network_monitor() {
            report_error_log!("初始化网络监控失败: {}", error);
        }

        if let Err(error) = wlan::initialize_wlan_monitor() {
            report_error_log!("初始化 WLAN 监控失败: {}", error);
        }

        network_quality::start_quality_probe();

        run_message_loop();

        network_quality::stop_quality_probe();

        wlan::cleanup_wlan_monitor();
        network::cleanup_network_monitor();

        unsafe { CoUninitialize() };

        MONITOR_THREAD_ID.store(0, Ordering::SeqCst);
        MONITOR_STARTED.store(false, Ordering::SeqCst);
    });
}

// NAPI 清理钩子：请求监控线程退出消息循环
pub unsafe extern "C" fn cleanup_monitor_thread(_arg: *mut std::ffi::c_void) {
    let thread_id = MONITOR_THREAD_ID.load(Ordering::SeqCst);
    if thread_id == 0 {
        return;
    }

    let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
}

// 监控线程消息循环：消费后台消息并驱动状态更新
fn run_message_loop() {
    loop {
        let mut msg = MSG::default();
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };

        if result.0 == -1 {
            report_error_log!("监控线程 GetMessageW 返回错误");
            break;
        }

        if result.0 == 0 {
            break;
        }

        if msg.message == WM_NETWORK_STATUS_CHANGE {
            handle_network_status_message(msg.wParam.0 as u32);
            continue;
        }

        if msg.message == WM_WIFI_SIGNAL_CHANGE {
            handle_wifi_signal_message(msg.wParam.0 as u32, msg.lParam.0 as i32);
            continue;
        }

        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// 处理网络连通性变化：更新状态并输出日志
fn handle_network_status_message(status: u32) {
    let connected = status != 0;
    with_monitor_state(|state| {
        state.network_connected = connected;
    });

    if connected {
        report_info_log!("网络已连接");
    } else {
        report_info_log!("网络已断开");
    }
}

// 处理 WiFi 信号变化：记录质量与 RSSI
fn handle_wifi_signal_message(quality: u32, rssi: i32) {
    report_info_log!("WiFi 信号变化：质量={}，RSSI={}", quality, rssi);
}
