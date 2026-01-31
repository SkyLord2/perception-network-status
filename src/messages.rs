use windows::Win32::Foundation::{GetLastError, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_USER};

use crate::global::MONITOR_THREAD_ID;
use crate::report_error_log;

// 监控线程私有消息：网络连通性变化，wParam=0/1
pub const WM_NETWORK_STATUS_CHANGE: u32 = WM_USER + 107;
// 监控线程私有消息：WiFi 信号变化，wParam=质量，lParam=RSSI
pub const WM_WIFI_SIGNAL_CHANGE: u32 = WM_USER + 108;

// 将网络连通性变化投递到监控线程消息循环
pub fn send_network_status_message(status: u32) {
    let thread_id = MONITOR_THREAD_ID.load(std::sync::atomic::Ordering::SeqCst);
    if thread_id == 0 {
        report_error_log!("后台监控线程未初始化，无法发送网络状态消息");
        return;
    }

    let posted = unsafe {
        PostThreadMessageW(
            thread_id,
            WM_NETWORK_STATUS_CHANGE,
            WPARAM(status as usize),
            LPARAM(0),
        )
    };
    if let Err(error) = posted {
        report_error_log!("发送网络状态消息失败: {}", error);
        let last_error = unsafe { GetLastError() };
        report_error_log!("发送网络状态消息失败，错误码: {:?}", last_error);
    }
}

// 将 WiFi 信号变化投递到监控线程消息循环
pub fn send_wlan_status_message(quality: u32, rssi: i32) {
    let thread_id = MONITOR_THREAD_ID.load(std::sync::atomic::Ordering::SeqCst);
    if thread_id == 0 {
        report_error_log!("后台监控线程未初始化，无法发送 WiFi 信号消息");
        return;
    }

    let posted = unsafe {
        PostThreadMessageW(
            thread_id,
            WM_WIFI_SIGNAL_CHANGE,
            WPARAM(quality as usize),
            LPARAM(rssi as isize),
        )
    };
    if let Err(error) = posted {
        report_error_log!("发送 WiFi 信号消息失败: {}", error);
        let last_error = unsafe { GetLastError() };
        report_error_log!("发送 WiFi 信号消息失败，错误码: {:?}", last_error);
    }
}
